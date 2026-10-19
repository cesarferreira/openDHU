//! GPU conversion of normalized, limited-range BT.601 YUV420 frames.
use eframe::glow::{self, HasContext};
use std::{ops::Range, sync::Arc};

pub fn plane_layout([w, h]: [usize; 2]) -> [(usize, usize, Range<usize>); 3] {
    let y = w * h;
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let c = cw * ch;
    [(w, h, 0..y), (cw, ch, y..y + c), (cw, ch, y + c..y + 2 * c)]
}

pub struct VideoGpu {
    gl: Arc<glow::Context>,
    program: Option<glow::Program>,
    vao: Option<glow::VertexArray>,
    planes: Vec<glow::Texture>,
    locations: Vec<Option<glow::UniformLocation>>,
    pending: Option<([usize; 2], Vec<u8>)>,
    size: [usize; 2],
    max_texture_size: usize,
    pub problem: Option<String>,
}
impl VideoGpu {
    pub fn new(gl: Arc<glow::Context>) -> Result<Self, String> {
        let version = eframe::egui_glow::ShaderVersion::get(&gl);
        if !version.is_new_shader_interface() {
            return Err("GPU video needs OpenGL 3 or OpenGL ES 3".into());
        }
        let prefix = if version.is_embedded() {
            "#version 300 es\nprecision highp float;\n"
        } else {
            "#version 140\n"
        };
        // SAFETY: eframe has made this GL context current during creation.
        let max_texture_size =
            unsafe { gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE).max(0) as usize };
        let mut gpu = Self {
            gl,
            program: None,
            vao: None,
            planes: vec![],
            locations: vec![],
            pending: None,
            size: [0, 0],
            max_texture_size,
            problem: None,
        };
        // SAFETY: eframe supplies a current GL context on the window thread.
        let result = unsafe {
            (|| -> Result<(), String> {
                let gl = &gpu.gl;
                let p = gl.create_program()?;
                gpu.program = Some(p);
                for (kind, source) in [
                    (glow::VERTEX_SHADER, VERTEX),
                    (glow::FRAGMENT_SHADER, FRAGMENT),
                ] {
                    let s = gl.create_shader(kind)?;
                    gl.shader_source(s, &format!("{prefix}{source}"));
                    gl.compile_shader(s);
                    if !gl.get_shader_compile_status(s) {
                        let e = gl.get_shader_info_log(s);
                        gl.delete_shader(s);
                        return Err(e);
                    }
                    gl.attach_shader(p, s);
                    gl.delete_shader(s);
                }
                gl.link_program(p);
                if !gl.get_program_link_status(p) {
                    return Err(gl.get_program_info_log(p));
                }
                gpu.vao = Some(gl.create_vertex_array()?);
                for _ in 0..3 {
                    gpu.planes.push(gl.create_texture()?);
                }
                for name in ["plane_y", "plane_u", "plane_v", "brightness"] {
                    gpu.locations.push(gl.get_uniform_location(p, name));
                }
                Ok(())
            })()
        };
        if let Err(e) = result {
            gpu.destroy();
            return Err(e);
        }
        Ok(gpu)
    }
    pub fn has_frame(&self) -> bool {
        self.size != [0, 0] || self.pending.is_some()
    }
    pub fn reset(&mut self) {
        self.pending = None;
        self.size = [0, 0];
    }
    pub fn submit(&mut self, size: [usize; 2], bytes: Vec<u8>) {
        self.pending = Some((size, bytes));
    }
    pub fn paint(&mut self, brightness: f32) {
        if self.problem.is_some() {
            return;
        }
        let gl = &self.gl;
        // SAFETY: egui invokes this callback on its current GL context with the
        // fitted viewport and clip already set, then restores its painter state.
        // Plane uploads are synchronous and bounded by the checked frame length.
        unsafe {
            gl.use_program(self.program);
            gl.bind_vertex_array(self.vao);
            let pending = self.pending.take();
            if let Some((size, bytes)) = pending.as_ref() {
                let layout = plane_layout(*size);
                if size.contains(&0) || size.iter().any(|v| *v > self.max_texture_size) {
                    self.problem = Some("video dimensions exceed this GPU's texture limit".into());
                    return;
                }
                if bytes.len() != layout[2].2.end {
                    self.problem = Some("incomplete YUV frame".into());
                    return;
                }
                gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
                for (i, (w, h, range)) in layout.iter().enumerate() {
                    gl.active_texture(glow::TEXTURE0 + i as u32);
                    gl.bind_texture(glow::TEXTURE_2D, Some(self.planes[i]));
                    if self.size != *size {
                        texture_options(gl);
                        gl.tex_image_2d(
                            glow::TEXTURE_2D,
                            0,
                            glow::R8 as i32,
                            *w as i32,
                            *h as i32,
                            0,
                            glow::RED,
                            glow::UNSIGNED_BYTE,
                            glow::PixelUnpackData::Slice(Some(&bytes[range.clone()])),
                        );
                    } else {
                        gl.tex_sub_image_2d(
                            glow::TEXTURE_2D,
                            0,
                            0,
                            0,
                            *w as i32,
                            *h as i32,
                            glow::RED,
                            glow::UNSIGNED_BYTE,
                            glow::PixelUnpackData::Slice(Some(&bytes[range.clone()])),
                        );
                    }
                }
                let error = gl.get_error();
                if error != glow::NO_ERROR {
                    self.problem = Some(format!("GPU video upload failed (OpenGL {error:#x})"));
                    return;
                }
                self.size = *size;
            } else {
                for (i, t) in self.planes.iter().enumerate() {
                    gl.active_texture(glow::TEXTURE0 + i as u32);
                    gl.bind_texture(glow::TEXTURE_2D, Some(*t));
                }
            }
            for i in 0..3 {
                gl.uniform_1_i32(self.locations[i].as_ref(), i as i32);
            }
            gl.uniform_1_f32(self.locations[3].as_ref(), brightness);
            if self.size != [0, 0] {
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                let error = gl.get_error();
                if error != glow::NO_ERROR {
                    self.problem = Some(format!("GPU video draw failed (OpenGL {error:#x})"));
                }
            }
        }
    }
    pub fn destroy(&mut self) {
        // SAFETY: creation failure and App::on_exit run with GL current. The
        // textures/program/VAO are exclusively owned here, never registered elsewhere.
        unsafe {
            let gl = &self.gl;
            if let Some(p) = self.program.take() {
                gl.delete_program(p);
            }
            if let Some(v) = self.vao.take() {
                gl.delete_vertex_array(v);
            }
            for t in self.planes.drain(..) {
                gl.delete_texture(t);
            }
        }
    }
}
unsafe fn texture_options(gl: &glow::Context) {
    unsafe {
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_S,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_T,
            glow::CLAMP_TO_EDGE as i32,
        );
    }
}
const VERTEX: &str = r#"
out vec2 uv;
void main() {
    vec2 p=vec2(float(gl_VertexID & 1),float((gl_VertexID >> 1) & 1));
    uv=vec2(p.x,1.0-p.y);
    gl_Position=vec4(p*2.0-1.0,0.0,1.0);
}
"#;
const FRAGMENT: &str = r#"
in vec2 uv;
uniform sampler2D plane_y;
uniform sampler2D plane_u;
uniform sampler2D plane_v;
uniform float brightness;
out vec4 color;
void main() {
    float y=(texture(plane_y,uv).r*255.0-16.0)/219.0;
    float u=(texture(plane_u,uv).r*255.0-128.0)/224.0;
    float v=(texture(plane_v,uv).r*255.0-128.0)/224.0;
    color=vec4(clamp(vec3(y+1.402*v,y-0.344136*u-0.714136*v,y+1.772*u),0.0,1.0)*brightness,1.0);
}
"#;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn planar_frames_cover_every_byte_including_odd_chroma_dimensions() {
        assert_eq!(
            plane_layout([800, 480]),
            [
                (800, 480, 0..384000),
                (400, 240, 384000..480000),
                (400, 240, 480000..576000)
            ]
        );
        assert_eq!(
            plane_layout([3, 3]),
            [(3, 3, 0..9), (2, 2, 9..13), (2, 2, 13..17)]
        );
    }
}

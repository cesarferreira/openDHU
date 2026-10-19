//! `opendhu-gui`: a desktop window onto the openDHU daemon. It uses the same HTTP API as the CLI.

mod gpu;
mod live;
mod model;
mod sound;
mod ui;
mod worker;

use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use eframe::egui;
use model::State;

/// Set to a `.png` path to save an image of the window after a short wait, then quit. Used to check
/// the rendered window without a screen recorder, so nothing else on the desktop is captured.
const SNAPSHOT_ENV: &str = "OPENDHU_GUI_SNAPSHOT";
const SNAPSHOT_AFTER: Duration = Duration::from_millis(2500);
/// Overrides the wait, for a window that needs longer to fill, such as the live picture.
const SNAPSHOT_AFTER_ENV: &str = "OPENDHU_GUI_SNAPSHOT_AFTER_MS";

fn gpu_video_enabled(value: Option<&std::ffi::OsStr>) -> bool {
    value != Some(std::ffi::OsStr::new("0"))
}

struct App {
    state: State,
    form: ui::Form,
    tx: mpsc::Sender<model::Update>,
    rx: mpsc::Receiver<model::Update>,
    live: live::Live,
    gpu: Option<Arc<Mutex<gpu::VideoGpu>>>,
    sound: sound::Sound,
    started: Instant,
    snapshot_to: Option<std::path::PathBuf>,
    snapshot_requested: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        ui::configure(&cc.egui_ctx);
        let (tx, rx) = mpsc::channel();
        worker::spawn_device_poller(cc.egui_ctx.clone(), tx.clone());
        worker::spawn_poller(cc.egui_ctx.clone(), tx.clone());
        let gpu = if gpu_video_enabled(std::env::var_os("OPENDHU_GPU_VIDEO").as_deref()) {
            cc.gl.clone().and_then(|gl| {
                gpu::VideoGpu::new(gl)
                    .map_err(|e| eprintln!("GPU video disabled: {e}"))
                    .ok()
            })
        } else {
            None
        };
        let gpu = gpu.map(|g| Arc::new(Mutex::new(g)));
        let mut live = live::Live::new();
        live.yuv = gpu.is_some();
        Self {
            gpu,
            state: State::new(),
            form: ui::Form::new(),
            tx,
            rx,
            live,
            sound: sound::Sound::new(),
            started: Instant::now(),
            snapshot_to: std::env::var_os(SNAPSHOT_ENV).map(Into::into),
            snapshot_requested: false,
        }
    }

    fn snapshot_step(&mut self, ctx: &egui::Context) {
        let Some(path) = self.snapshot_to.clone() else {
            return;
        };
        let wait = std::env::var(SNAPSHOT_AFTER_ENV)
            .ok()
            .and_then(|v| v.parse().ok())
            .map_or(SNAPSHOT_AFTER, Duration::from_millis);
        if !self.snapshot_requested && self.started.elapsed() >= wait {
            self.snapshot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = image {
            let [w, h] = img.size;
            let saved = image::save_buffer(
                &path,
                img.as_raw(),
                w as u32,
                h as u32,
                image::ColorType::Rgba8,
            );
            if let Err(e) = saved {
                eprintln!("cannot save the snapshot: {e}");
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint_after(Duration::from_millis(200));
    }
}

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.lock().unwrap_or_else(|e| e.into_inner()).destroy();
        }
    }
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(update) = self.rx.try_recv() {
            self.state.apply(update);
        }
        // The live picture runs only while a native session does and the box is ticked.
        let want = self.state.native_running();
        let gpu_failed = self.gpu.as_ref().is_some_and(|gpu| {
            gpu.lock()
                .unwrap_or_else(|e| e.into_inner())
                .problem
                .is_some()
        });
        self.live.tick(
            ctx,
            want && self.form.live && !gpu_failed,
            self.state.screen(),
        );
        if let Some(gpu) = self.gpu.as_ref() {
            let failed = gpu
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .problem
                .clone();
            if let Some(e) = failed {
                self.live.tick(ctx, false, self.state.screen());
                self.state.gpu_video = None;
                if !self.live.is_running() {
                    eprintln!("GPU video failed, reverting to RGBA: {e}");
                    gpu.lock().unwrap_or_else(|e| e.into_inner()).destroy();
                    self.gpu = None;
                    self.live = live::Live::new();
                }
            } else {
                if let Some((size, bytes)) = self.live.take_yuv() {
                    gpu.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .submit(size, bytes);
                }
                if gpu.lock().unwrap_or_else(|e| e.into_inner()).has_frame() {
                    let gpu = Arc::clone(gpu);
                    let brightness = if self.state.phone_stopped_projecting()
                        || self.state.live_problem.is_some()
                    {
                        65.0 / 255.0
                    } else {
                        1.0
                    };
                    self.state.gpu_video = Some(egui::PaintCallback {
                        rect: egui::Rect::NOTHING,
                        callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |_, _| {
                            gpu.lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .paint(brightness);
                        })),
                    });
                }
            }
        } else if let Some(img) = self.live.take_frame() {
            update_live_texture(ctx, &mut self.state.live_texture, img);
        }
        if !want {
            if let Some(gpu) = self.gpu.as_ref() {
                gpu.lock().unwrap_or_else(|e| e.into_inner()).reset();
            }
            self.state.gpu_video = None;
            self.state.live_texture = None;
        }
        // Audio is opt-in: it plays sound, and the phone may be playing something private.
        self.sound.tick(want && self.form.audio);
        self.state.audio_summary = self.form.audio.then(|| self.sound.summary());
        self.state.live_summary = self.live.summary();
        self.state.live_problem = self
            .live
            .problem
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        ui::draw(ctx, &mut self.state, &mut self.form, &self.tx);
        self.snapshot_step(ctx);
    }
}

fn update_live_texture(
    ctx: &egui::Context,
    texture: &mut Option<egui::TextureHandle>,
    image: egui::ColorImage,
) {
    match texture {
        // Same-sized video frames only replace texels. A full set asks the OpenGL
        // renderer to reallocate texture storage for every frame.
        Some(t) if t.size() == image.size => {
            t.set_partial([0, 0], image, egui::TextureOptions::LINEAR);
        }
        Some(t) => t.set(image, egui::TextureOptions::LINEAR),
        None => {
            *texture = Some(ctx.load_texture("live", image, egui::TextureOptions::LINEAR));
        }
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("openDHU")
            .with_inner_size([1152.0, 768.0])
            .with_min_inner_size([720.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "openDHU",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

#[cfg(test)]
mod texture_tests {
    use super::*;

    #[test]
    fn gpu_video_is_default_with_an_explicit_rgba_override() {
        use std::ffi::OsStr;
        assert!(gpu_video_enabled(None));
        assert!(gpu_video_enabled(Some(OsStr::new("1"))));
        assert!(!gpu_video_enabled(Some(OsStr::new("0"))));
    }

    #[test]
    fn live_texture_updates_reuse_storage_until_the_resolution_changes() {
        let ctx = egui::Context::default();
        let mut texture = None;
        let first = ctx.run(egui::RawInput::default(), |ctx| {
            update_live_texture(
                ctx,
                &mut texture,
                egui::ColorImage::filled([2, 1], egui::Color32::RED),
            );
        });
        let id = texture.as_ref().unwrap().id();
        let delta = first
            .textures_delta
            .set
            .iter()
            .find(|(t, _)| *t == id)
            .unwrap();
        assert_eq!(delta.1.pos, None);

        let next = ctx.run(egui::RawInput::default(), |ctx| {
            update_live_texture(
                ctx,
                &mut texture,
                egui::ColorImage::filled([2, 1], egui::Color32::BLUE),
            );
        });
        assert_eq!(texture.as_ref().unwrap().id(), id);
        let delta = next
            .textures_delta
            .set
            .iter()
            .find(|(t, _)| *t == id)
            .unwrap();
        assert_eq!(delta.1.pos, Some([0, 0]));
        assert_eq!(delta.1.image.size(), [2, 1]);
        let egui::ImageData::Color(pixels) = &delta.1.image;
        assert_eq!(pixels.pixels, [egui::Color32::BLUE; 2]);

        let resized = ctx.run(egui::RawInput::default(), |ctx| {
            update_live_texture(
                ctx,
                &mut texture,
                egui::ColorImage::filled([1, 2], egui::Color32::GREEN),
            );
        });
        let delta = resized
            .textures_delta
            .set
            .iter()
            .find(|(t, _)| *t == id)
            .unwrap();
        assert_eq!(delta.1.pos, None);
        assert_eq!(texture.as_ref().unwrap().size(), [1, 2]);
    }
}

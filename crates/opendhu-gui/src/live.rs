//! The live picture: the daemon's H.264 stream, decoded by `ffmpeg` into RGBA frames.
//!
//! One background thread feeds the daemon's bytes to `ffmpeg` and a second reads the frames back.
//! Only the newest frame is kept, so a slow window never builds up a backlog.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use opendhu_daemon::Client;

/// After a failed or ended stream, wait this long before trying again.
const RETRY: Duration = Duration::from_secs(1);

pub struct Live {
    frame: Arc<Mutex<Option<Vec<u8>>>>,
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    last_try: Option<Instant>,
    /// Why the last stream ended, for the window to show.
    pub problem: Arc<Mutex<Option<String>>>,
    stats: Arc<Stats>,
    started: Instant,
    /// The screen size of the stream currently running (or last started), for `take_frame` to shape
    /// the raw bytes `stream` decoded at that same size.
    screen: (usize, usize),
}

/// Counters the window shows under the picture, so a frozen or restarting stream can be told apart
/// from a quiet screen.
#[derive(Default)]
struct Stats {
    frames: AtomicU64,
    restarts: AtomicU64,
    /// Milliseconds since `Live::started` when the last frame arrived. 0 means none yet.
    last_frame_ms: AtomicU64,
}

impl Live {
    pub fn new() -> Self {
        Self {
            frame: Arc::default(),
            stop: Arc::default(),
            running: Arc::default(),
            last_try: None,
            problem: Arc::default(),
            stats: Arc::default(),
            started: Instant::now(),
            screen: (
                opendhu_native::protocol::SCREEN_WIDTH as usize,
                opendhu_native::protocol::SCREEN_HEIGHT as usize,
            ),
        }
    }

    /// Start the stream when it is wanted and not running, and stop it when it is not wanted.
    /// `screen` is the session's reported pixel size: what a new stream is decoded at.
    pub fn tick(&mut self, ctx: &egui::Context, want: bool, screen: (u16, u16)) {
        let running = self.running.load(Ordering::SeqCst);
        if !want {
            if running {
                self.stop.store(true, Ordering::SeqCst);
            }
            return;
        }
        if running || self.last_try.is_some_and(|t| t.elapsed() < RETRY) {
            return;
        }
        self.last_try = Some(Instant::now());
        self.stats.restarts.fetch_add(1, Ordering::SeqCst);
        // A new attempt starts clean: the last attempt's error no longer applies.
        *self.problem.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.stop = Arc::new(AtomicBool::new(false));
        self.running.store(true, Ordering::SeqCst);
        self.screen = (screen.0 as usize, screen.1 as usize);
        let (frame, stop, running, problem, stats, started, ctx, screen) = (
            Arc::clone(&self.frame),
            Arc::clone(&self.stop),
            Arc::clone(&self.running),
            Arc::clone(&self.problem),
            Arc::clone(&self.stats),
            self.started,
            ctx.clone(),
            self.screen,
        );
        thread::spawn(move || {
            let result = stream(&frame, &stop, &ctx, &stats, started, screen);
            *problem.lock().unwrap_or_else(|e| e.into_inner()) = result.err();
            running.store(false, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }

    /// One line for the window: how the stream is doing.
    pub fn summary(&self) -> String {
        let frames = self.stats.frames.load(Ordering::SeqCst);
        let restarts = self.stats.restarts.load(Ordering::SeqCst).saturating_sub(1);
        let last = self.stats.last_frame_ms.load(Ordering::SeqCst);
        let ago = if last == 0 {
            "no frame yet".to_string()
        } else {
            let now = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
            format!(
                "last frame {:.1} s ago",
                now.saturating_sub(last) as f64 / 1000.0
            )
        };
        format!(
            "{frames} frames received, {ago}, {restarts} restart{}",
            if restarts == 1 { "" } else { "s" }
        )
    }

    /// The newest decoded frame, if one arrived since the last call.
    pub fn take_frame(&self) -> Option<egui::ColorImage> {
        let bytes = self
            .frame
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()?;
        // H.264 is opaque: FFmpeg's RGBA pixels already have alpha 255, so their bytes
        // are also valid premultiplied Color32 pixels. Move the allocation instead of
        // allocating and converting every pixel again on the UI thread.
        Some(egui::ColorImage::new(
            [self.screen.0, self.screen.1],
            bytemuck::allocation::cast_vec(bytes),
        ))
    }
}

fn stream(
    frame: &Arc<Mutex<Option<Vec<u8>>>>,
    stop: &AtomicBool,
    ctx: &egui::Context,
    stats: &Arc<Stats>,
    started: Instant,
    (width, height): (usize, usize),
) -> Result<(), String> {
    let frame_bytes = width * height * 4;
    let client = Client::connect().map_err(|e| e.to_string())?;
    let mut video = client.video().map_err(|e| e.to_string())?;
    let ffmpeg = opendhu_native::find_ffmpeg()
        .ok_or("ffmpeg not found; it decodes the live picture (macOS: brew install ffmpeg)")?;
    let mut child = Command::new(ffmpeg)
        // Not `-fflags nobuffer`: with it ffmpeg stalls on a still screen and on a long burst (checked).
        // This is one interactive stream. Automatic frame/filter thread pools queue
        // full-sized frames and add worker overhead; keep the pipeline low latency.
        .args(["-v", "error", "-nostdin", "-flags", "low_delay"])
        .args(["-threads", "1", "-filter_threads", "1"])
        .args([
            "-probesize",
            "32",
            "-analyzeduration",
            "0",
            "-f",
            "h264",
            "-i",
            "pipe:0",
        ])
        // Output options follow -i: rawvideo also enables frame threading by default.
        .args(["-threads", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run ffmpeg: {e}"))?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let mut stdout = child.stdout.take().expect("stdout is piped");

    let sink = {
        let (slot, ctx, stats) = (Arc::clone(frame), ctx.clone(), Arc::clone(stats));
        thread::spawn(move || {
            let mut buf = vec![0u8; frame_bytes];
            while stdout.read_exact(&mut buf).is_ok() {
                // Swap, don't copy: the slot's old buffer (if the window has not taken it) becomes the
                // next one to fill, so catching up on a long burst allocates nothing.
                let old = slot.lock().unwrap_or_else(|e| e.into_inner()).replace(buf);
                buf = old.unwrap_or_else(|| vec![0u8; frame_bytes]);
                stats.frames.fetch_add(1, Ordering::SeqCst);
                let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                stats.last_frame_ms.store(ms.max(1), Ordering::SeqCst);
                ctx.request_repaint();
            }
        })
    };

    let mut chunk = vec![0u8; 64 * 1024];
    let outcome = loop {
        if stop.load(Ordering::SeqCst) {
            break Ok(());
        }
        match video.read(&mut chunk) {
            Ok(0) => break Err("the video stream ended".to_string()),
            Ok(n) => {
                if stdin.write_all(&chunk[..n]).is_err() {
                    break Err("ffmpeg stopped decoding".to_string());
                }
            }
            Err(e) => break Err(format!("the video stream broke: {e}")),
        }
    };
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    let _ = sink.join();
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoded_rgba_frame_reuses_its_storage_without_changing_colors() {
        let mut live = Live::new();
        live.screen = (2, 1);
        let bytes = vec![255, 0, 0, 255, 0, 128, 255, 255];
        let allocation = bytes.as_ptr() as usize;
        *live.frame.lock().unwrap() = Some(bytes);

        let image = live.take_frame().unwrap();
        assert_eq!(image.size, [2, 1]);
        assert_eq!(
            image.pixels,
            [egui::Color32::RED, egui::Color32::from_rgb(0, 128, 255)]
        );
        assert_eq!(image.pixels.as_ptr() as usize, allocation);
        assert!(live.take_frame().is_none());
    }
}

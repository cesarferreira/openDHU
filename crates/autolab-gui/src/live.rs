//! The live picture: the daemon's H.264 stream, decoded by `ffmpeg` into RGBA frames.
//!
//! One background thread feeds the daemon's bytes to `ffmpeg` and a second reads the frames back.
//! Only the newest frame is kept, so a slow window never builds up a backlog.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use autolab_daemon::Client;
use autolab_native::protocol::{SCREEN_HEIGHT, SCREEN_WIDTH};
use eframe::egui;

const FRAME_BYTES: usize = SCREEN_WIDTH as usize * SCREEN_HEIGHT as usize * 4;
/// After a failed or ended stream, wait this long before trying again.
const RETRY: Duration = Duration::from_secs(1);

pub struct Live {
    frame: Arc<Mutex<Option<Vec<u8>>>>,
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    last_try: Option<Instant>,
    /// Why the last stream ended, for the window to show.
    pub problem: Arc<Mutex<Option<String>>>,
}

impl Live {
    pub fn new() -> Self {
        Self {
            frame: Arc::default(),
            stop: Arc::default(),
            running: Arc::default(),
            last_try: None,
            problem: Arc::default(),
        }
    }

    /// Start the stream when it is wanted and not running, and stop it when it is not wanted.
    pub fn tick(&mut self, ctx: &egui::Context, want: bool) {
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
        // A new attempt starts clean: the last attempt's error no longer applies.
        *self.problem.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.stop = Arc::new(AtomicBool::new(false));
        self.running.store(true, Ordering::SeqCst);
        let (frame, stop, running, problem, ctx) = (
            Arc::clone(&self.frame),
            Arc::clone(&self.stop),
            Arc::clone(&self.running),
            Arc::clone(&self.problem),
            ctx.clone(),
        );
        thread::spawn(move || {
            let result = stream(&frame, &stop, &ctx);
            *problem.lock().unwrap_or_else(|e| e.into_inner()) = result.err();
            running.store(false, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }

    /// The newest decoded frame, if one arrived since the last call.
    pub fn take_frame(&self) -> Option<egui::ColorImage> {
        let bytes = self
            .frame
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()?;
        Some(egui::ColorImage::from_rgba_unmultiplied(
            [SCREEN_WIDTH as usize, SCREEN_HEIGHT as usize],
            &bytes,
        ))
    }
}

fn stream(
    frame: &Arc<Mutex<Option<Vec<u8>>>>,
    stop: &AtomicBool,
    ctx: &egui::Context,
) -> Result<(), String> {
    let client = Client::connect().map_err(|e| e.to_string())?;
    let mut video = client.video().map_err(|e| e.to_string())?;
    let ffmpeg = autolab_native::find_ffmpeg()
        .ok_or("ffmpeg not found; it decodes the live picture (macOS: brew install ffmpeg)")?;
    let mut child = Command::new(ffmpeg)
        .args([
            "-v",
            "error",
            "-nostdin",
            "-fflags",
            "nobuffer",
            "-flags",
            "low_delay",
        ])
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
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run ffmpeg: {e}"))?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let mut stdout = child.stdout.take().expect("stdout is piped");

    let sink = {
        let (slot, ctx) = (Arc::clone(frame), ctx.clone());
        thread::spawn(move || {
            let mut buf = vec![0u8; FRAME_BYTES];
            while stdout.read_exact(&mut buf).is_ok() {
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(buf.clone());
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

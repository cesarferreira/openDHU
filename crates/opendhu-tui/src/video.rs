//! The live picture: the daemon's H.264 decoded by `ffmpeg`, scaled to the size it will be drawn at.
//!
//! Only the newest frame is kept, so a slow terminal never builds up a backlog.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use opendhu_daemon::Client;

use crate::Msg;

/// After a failed or ended stream, wait this long before trying again.
const RETRY: Duration = Duration::from_secs(1);

pub struct Frame {
    pub size: (u32, u32),
    pub rgb: Vec<u8>,
}

struct Stream {
    size: (u32, u32),
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

pub struct Video {
    slot: Arc<Mutex<Option<Frame>>>,
    problem: Arc<Mutex<Option<String>>>,
    stream: Option<Stream>,
    last_try: Option<Instant>,
    tx: Sender<Msg>,
}

impl Video {
    pub fn new(tx: Sender<Msg>) -> Self {
        Self {
            slot: Arc::default(),
            problem: Arc::default(),
            stream: None,
            last_try: None,
            tx,
        }
    }

    /// Keep a stream running at `size`: restart it when the size changes, retry after a failure.
    pub fn tick(&mut self, size: (u32, u32)) {
        if let Some(s) = &self.stream {
            let running = s.running.load(Ordering::SeqCst);
            if s.size == size && (running || self.last_try.is_some_and(|t| t.elapsed() < RETRY)) {
                return;
            }
            s.stop.store(true, Ordering::SeqCst);
        }
        self.last_try = Some(Instant::now());
        let stream = Stream {
            size,
            stop: Arc::default(),
            running: Arc::new(AtomicBool::new(true)),
        };
        let (slot, problem, tx) = (
            Arc::clone(&self.slot),
            Arc::clone(&self.problem),
            self.tx.clone(),
        );
        let (stop, running) = (Arc::clone(&stream.stop), Arc::clone(&stream.running));
        thread::spawn(move || {
            let result = run(&slot, &stop, &tx, size);
            if !stop.load(Ordering::SeqCst) {
                *problem.lock().unwrap_or_else(|e| e.into_inner()) = result.err();
            }
            running.store(false, Ordering::SeqCst);
            let _ = tx.send(Msg::Frame);
        });
        self.stream = Some(stream);
    }

    pub fn take(&self) -> Option<Frame> {
        let frame = self.slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        if frame.is_some() {
            *self.problem.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        frame
    }

    pub fn problem(&self) -> Option<String> {
        self.problem
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

fn run(
    slot: &Arc<Mutex<Option<Frame>>>,
    stop: &AtomicBool,
    tx: &Sender<Msg>,
    (width, height): (u32, u32),
) -> Result<(), String> {
    let client = Client::connect().map_err(|e| e.to_string())?;
    let mut video = client.video().map_err(|e| e.to_string())?;
    let ffmpeg = opendhu_native::find_ffmpeg()
        .ok_or("ffmpeg not found; it decodes the live picture (macOS: brew install ffmpeg)")?;
    let mut child = Command::new(ffmpeg)
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
        .args([
            "-vf",
            &format!("scale={width}:{height}:flags=fast_bilinear"),
        ])
        .args(["-threads", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run ffmpeg: {e}"))?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let mut stdout = child.stdout.take().expect("stdout is piped");

    let frame_bytes = width as usize * height as usize * 3;
    let sink = {
        let (tx, slot) = (tx.clone(), Arc::clone(slot));
        thread::spawn(move || {
            let mut buf = vec![0u8; frame_bytes];
            while stdout.read_exact(&mut buf).is_ok() {
                // Swap, don't copy: catching up on a burst reuses the unclaimed frame's storage.
                let old = slot
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .replace(Frame {
                        size: (width, height),
                        rgb: buf,
                    });
                buf = old.map_or_else(|| vec![0u8; frame_bytes], |f| f.rgb);
                let _ = tx.send(Msg::Frame);
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

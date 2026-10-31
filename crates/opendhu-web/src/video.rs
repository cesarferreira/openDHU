//! The live picture for browsers: the daemon's H.264 turned into JPEGs by `ffmpeg`, which an `<img>`
//! shows as an MJPEG stream. One decoder serves every open page, and it runs only while a page is
//! watching. Only the newest frame is kept, so a slow page skips frames instead of falling behind.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tokio::sync::watch;

/// After a failed or ended stream, wait this long before trying again.
const RETRY: Duration = Duration::from_secs(1);
/// How often an idle decoder checks whether a page has started watching.
const IDLE: Duration = Duration::from_millis(200);

pub type Jpeg = Arc<Vec<u8>>;

pub struct Video {
    frames: watch::Sender<Option<Jpeg>>,
    problem: std::sync::Mutex<Option<String>>,
}

impl Video {
    pub fn start() -> Arc<Self> {
        let video = Arc::new(Self {
            frames: watch::channel(None).0,
            problem: std::sync::Mutex::default(),
        });
        let worker = Arc::clone(&video);
        thread::spawn(move || worker.run());
        video
    }

    pub fn subscribe(&self) -> watch::Receiver<Option<Jpeg>> {
        self.frames.subscribe()
    }

    pub fn problem(&self) -> Option<String> {
        self.problem
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn set_problem(&self, problem: Option<String>) {
        *self.problem.lock().unwrap_or_else(|e| e.into_inner()) = problem;
    }

    fn watched(&self) -> bool {
        self.frames.receiver_count() > 0
    }

    fn run(&self) {
        loop {
            if !self.watched() {
                // A problem nobody is looking at would be stale by the time someone is.
                self.set_problem(None);
                thread::sleep(IDLE);
                continue;
            }
            let generation = crate::session::generation();
            let result = self.stream(generation);
            // The old session's picture is someone else's now.
            if crate::session::generation() != generation {
                self.frames.send_replace(None);
                continue;
            }
            if let Err(e) = result {
                self.set_problem(Some(e));
            }
            thread::sleep(RETRY);
        }
    }

    fn stream(&self, generation: u64) -> Result<(), String> {
        let client = crate::session::client().map_err(|e| e.to_string())?;
        let mut h264 = client.video().map_err(|e| e.to_string())?;
        self.set_problem(None);
        let ffmpeg = opendhu_native::find_ffmpeg()
            .ok_or("ffmpeg not found; it decodes the live picture (macOS: brew install ffmpeg)")?;
        let mut child = Command::new(ffmpeg)
            .args(["-v", "error", "-nostdin", "-flags", "low_delay"])
            .args(["-probesize", "32", "-analyzeduration", "0"])
            .args(["-f", "h264", "-i", "pipe:0"])
            .args(["-f", "mjpeg", "-q:v", "5", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("cannot run ffmpeg: {e}"))?;
        let mut stdin = child.stdin.take().expect("stdin is piped");
        let mut stdout = child.stdout.take().expect("stdout is piped");

        thread::scope(|scope| {
            let sink = scope.spawn(|| {
                let mut split = Splitter::default();
                let mut buf = vec![0u8; 64 * 1024];
                while let Ok(n) = stdout.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    for jpeg in split.feed(&buf[..n]) {
                        self.set_problem(None);
                        self.frames.send_replace(Some(Arc::new(jpeg)));
                    }
                }
            });
            let mut chunk = vec![0u8; 64 * 1024];
            let outcome = loop {
                // Nobody is watching, or the run moved on to another session. A still screen still
                // sends a filler unit every second, so this is checked at least that often.
                if !self.watched() || crate::session::generation() != generation {
                    break Ok(());
                }
                match h264.read(&mut chunk) {
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
        })
    }
}

/// Cuts ffmpeg's MJPEG output into whole JPEGs: each starts with `FF D8` and ends with `FF D9`.
/// Inside the compressed data a literal `FF` is always followed by `00`, so the first `FF D9` after
/// a start is the end of that picture.
#[derive(Default)]
pub struct Splitter {
    buf: Vec<u8>,
}

impl Splitter {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            let Some(start) = find(&self.buf, &[0xFF, 0xD8], 0) else {
                // Keep a trailing FF: it may be the first half of the next start marker.
                let keep = usize::from(self.buf.last() == Some(&0xFF));
                self.buf.drain(..self.buf.len() - keep);
                break;
            };
            let Some(end) = find(&self.buf, &[0xFF, 0xD9], start + 2) else {
                self.buf.drain(..start);
                break;
            };
            out.push(self.buf[start..end + 2].to_vec());
            self.buf.drain(..end + 2);
        }
        out
    }
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| i + from)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &[u8] = &[0xFF, 0xD8, 1, 0xFF, 0x00, 2, 0xFF, 0xD9];
    const B: &[u8] = &[0xFF, 0xD8, 3, 0xFF, 0xD9];

    #[test]
    fn whole_pictures_come_out_one_by_one() {
        let mut s = Splitter::default();
        let mut both = A.to_vec();
        both.extend_from_slice(B);
        assert_eq!(s.feed(&both), vec![A.to_vec(), B.to_vec()]);
        assert!(s.feed(&[]).is_empty());
    }

    #[test]
    fn a_picture_split_across_reads_is_put_back_together() {
        let mut s = Splitter::default();
        // Split inside both markers, the worst places.
        assert!(s.feed(&[0x00, 0xFF]).is_empty());
        assert!(s.feed(&[0xD8, 1, 0xFF, 0x00, 2, 0xFF]).is_empty());
        assert_eq!(s.feed(&[0xD9, 0xFF]), vec![A.to_vec()]);
        assert_eq!(s.feed(&B[1..]), vec![B.to_vec()]);
    }

    #[test]
    fn noise_before_a_start_marker_is_dropped() {
        let mut s = Splitter::default();
        let mut noisy = vec![9, 9, 9];
        noisy.extend_from_slice(B);
        assert_eq!(s.feed(&noisy), vec![B.to_vec()]);
        assert!(s.buf.is_empty());
    }
}

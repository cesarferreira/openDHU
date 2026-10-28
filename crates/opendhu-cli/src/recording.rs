//! Video of the projected screen: the daemon's raw H.264 feed, encoded to an MP4 by ffmpeg.
//!
//! The phone only sends a frame when the picture changes, so the feed has no steady frame rate. ffmpeg
//! stamps each frame with the time it arrives, which keeps the clip's pace true to what happened.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use clap::ValueEnum;
use opendhu_core::{Error, Result};
use opendhu_daemon::Client;

/// Which tests of an `aa test --record` run keep their clip.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Only tests that did not pass
    Failures,
    /// Every test
    All,
}

impl Mode {
    pub fn keeps(self, passed: bool) -> bool {
        self == Self::All || !passed
    }
}

/// An ffmpeg encoding the feed into `path` until `finish` is called.
pub struct Recorder {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    writer: Option<JoinHandle<()>>,
    ffmpeg: Child,
}

fn ffmpeg_args(out: &str) -> Vec<&str> {
    vec![
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "h264",
        "-use_wallclock_as_timestamps",
        "1",
        "-i",
        "pipe:0",
        // H.264 in MP4 needs even dimensions.
        "-vf",
        "pad=ceil(iw/2)*2:ceil(ih/2)*2",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-crf",
        "28",
        "-pix_fmt",
        "yuv420p",
        "-movflags",
        "+faststart",
        "-y",
        out,
    ]
}

impl Recorder {
    /// Start recording. Fails at once when the session has no video (the DHU backend shows it in its
    /// own window) or ffmpeg cannot start.
    pub fn start(client: &Client, ffmpeg: &Path, path: &Path) -> Result<Self> {
        Self::from_feed(client.video()?, ffmpeg, path)
    }

    fn from_feed(mut feed: Box<dyn Read + Send>, ffmpeg: &Path, path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::Other(e.to_string()))?;
        }
        let out = path
            .to_str()
            .ok_or_else(|| Error::InvalidArgs("video path is not UTF-8".into()))?;
        let mut ffmpeg = Command::new(ffmpeg)
            .args(ffmpeg_args(out))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::BackendUnavailable(format!("could not start ffmpeg: {e}")))?;
        let mut stdin = ffmpeg.stdin.take().expect("stdin pipe");
        // The read blocks until the next chunk, and a still screen sends none. So the read lives in a
        // thread nothing waits for: it ends at the next chunk after the recorder is done (its send
        // fails), or when the session ends.
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            while let Ok(n) = feed.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let writer = thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(chunk) if stdin.write_all(&chunk).is_err() => break,
                    Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            // Dropping stdin ends the input, which is what makes ffmpeg finish the file.
        });
        Ok(Self {
            path: path.to_path_buf(),
            stop,
            writer: Some(writer),
            ffmpeg,
        })
    }

    /// Stop and finish the file, returning the finished file.
    pub fn finish(mut self) -> Result<PathBuf> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
        let output = self
            .ffmpeg
            .wait_with_output()
            .map_err(|e| Error::Other(format!("ffmpeg did not finish: {e}")))?;
        if !output.status.success() {
            let _ = std::fs::remove_file(&self.path);
            return Err(Error::Other(format!(
                "ffmpeg failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(self.path)
    }

    /// Stop and throw the clip away.
    pub fn discard(self) {
        let path = self.path.clone();
        let _ = self.finish();
        let _ = std::fs::remove_file(path);
    }
}

/// `aa record`: film the car screen for `seconds`.
pub fn record(client: &Client, ffmpeg: &Path, path: &Path, seconds: u64) -> Result<PathBuf> {
    let recorder = Recorder::start(client, ffmpeg, path)?;
    thread::sleep(Duration::from_secs(seconds));
    recorder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_mode_keeps_only_clips_of_tests_that_did_not_pass() {
        assert!(Mode::Failures.keeps(false));
        assert!(!Mode::Failures.keeps(true));
        assert!(Mode::All.keeps(true));
        assert!(Mode::All.keeps(false));
    }

    /// A feed that hands out `chunks`, one every `gap`, then blocks until dropped like a still screen.
    struct Paced {
        chunks: std::vec::IntoIter<Vec<u8>>,
        gap: Duration,
    }

    impl Read for Paced {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.chunks.next() {
                Some(chunk) => {
                    thread::sleep(self.gap);
                    buf[..chunk.len()].copy_from_slice(&chunk);
                    Ok(chunk.len())
                }
                None => {
                    thread::sleep(Duration::from_secs(30));
                    Ok(0)
                }
            }
        }
    }

    #[test]
    fn encodes_a_paced_feed_into_a_playable_clip_and_stops_on_a_still_screen() {
        let Some(ffmpeg) = opendhu_native::find_ffmpeg() else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("opendhu-rec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let raw = dir.join("in.h264");
        let made = Command::new(&ffmpeg)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=201x101:rate=10",
                "-t",
                "2",
            ])
            .args([
                "-c:v",
                "libx264",
                "-bsf:v",
                "h264_mp4toannexb",
                "-f",
                "h264",
                "-y",
            ])
            .arg(&raw)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            return; // no libx264 here
        }
        let bytes = std::fs::read(&raw).unwrap();
        let chunks: Vec<Vec<u8>> = bytes
            .chunks(bytes.len().div_ceil(4))
            .map(<[u8]>::to_vec)
            .collect();
        let out = dir.join("out.mp4");
        let started = std::time::Instant::now();
        let recorder = Recorder::from_feed(
            Box::new(Paced {
                chunks: chunks.into_iter(),
                gap: Duration::from_millis(300),
            }),
            &ffmpeg,
            &out,
        )
        .unwrap();
        thread::sleep(Duration::from_millis(1500));
        let path = recorder.finish().unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "finish waited for the blocked feed"
        );
        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size > 1000, "clip is {size} bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ffmpeg_stamps_frames_with_their_arrival_time_and_pads_to_even_sizes() {
        let args = ffmpeg_args("out.mp4");
        let at = |flag: &str| args.iter().position(|a| *a == flag).unwrap();
        assert_eq!(args[at("-use_wallclock_as_timestamps") + 1], "1");
        assert!(args[at("-vf") + 1].starts_with("pad=ceil(iw/2)*2"));
        assert_eq!(args.last(), Some(&"out.mp4"));
    }
}

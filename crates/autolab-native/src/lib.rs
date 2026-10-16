//! The native backend: AutoLab's own Android Auto head unit, run as a child process.
//!
//! The head unit lives in `autolab-native-hu` and talks JSON lines (see `protocol`). Running it in a
//! separate process keeps the third-party protocol crate's panics out of the daemon, and the child
//! exits when this side closes its stdin, so a killed daemon never leaves a head unit behind.

pub mod protocol;

/// `ffmpeg` on `PATH` or in the usual install directories. The native backend decodes the phone's
/// H.264 video with it to make screenshots.
pub fn find_ffmpeg() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    // A daemon started from a GUI often has a bare PATH.
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from));
    dirs.into_iter()
        .map(|d| d.join("ffmpeg"))
        .find(|p| p.is_file())
}

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use autolab_core::{
    AudioChannel, Capabilities, Error, FocusState, HeadUnitBackend, HeadUnitStatus, InputResult,
    Key, OperationResult, Result, ScreenshotResult, SessionState, Support, VideoFocusStatus,
};
use protocol::{Command as Cmd, ErrorInfo, ErrorKind, Event, Line, Outcome, Reply, Request};

/// The phone may take a while to wake and start projecting.
const READY_TIMEOUT: Duration = Duration::from_secs(60);
/// A status read is local to the child, so a slow answer means it is stuck.
const STATUS_TIMEOUT: Duration = Duration::from_secs(5);
/// Focus waits up to 4 s for the phone to confirm, so allow a margin on top.
const FOCUS_TIMEOUT: Duration = Duration::from_secs(8);
/// Decoding a screenshot runs `ffmpeg` in the child.
const SHOT_TIMEOUT: Duration = Duration::from_secs(20);
/// A tap is quick; the child also watches the screen for half a second afterwards.
const TOUCH_TIMEOUT: Duration = Duration::from_secs(5);
const STOP_GRACE: Duration = Duration::from_secs(5);

/// One chunk of the phone's video: Annex-B H.264, one or more NAL units.
pub type VideoChunk = Arc<Vec<u8>>;

/// Viewers of the live video. The reader thread publishes to all of them. A viewer that falls
/// behind or goes away is dropped: skipping frames would corrupt its picture, and a viewer can
/// always subscribe again, which restarts the stream at a keyframe.
#[derive(Default)]
struct VideoHub {
    viewers: Mutex<Vec<Viewer>>,
}

struct Viewer {
    tx: SyncSender<VideoChunk>,
    /// Has seen a start-up burst. Until then it gets nothing: live frames without the parameter sets
    /// and a keyframe cannot be decoded.
    started: bool,
}

/// How many chunks a viewer may lag by before it is dropped.
const VIEWER_BACKLOG: usize = 256;

impl VideoHub {
    /// `started`: audio has no start-up burst to wait for, video does.
    fn add(&self, started: bool) -> Receiver<VideoChunk> {
        let (tx, rx) = mpsc::sync_channel(VIEWER_BACKLOG);
        self.viewers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Viewer { tx, started });
        rx
    }

    /// Send to the viewers it is meant for. A burst starts the viewers still waiting for one; a live
    /// chunk goes to the viewers that have started. Returns how many viewers remain.
    fn publish(&self, chunk: &VideoChunk, burst: bool) -> usize {
        let mut viewers = self.viewers.lock().unwrap_or_else(|e| e.into_inner());
        viewers.retain_mut(|v| {
            if burst == v.started {
                // A burst for a viewer that is already running, or a live chunk for one that is not
                // started yet: not for this viewer.
                return true;
            }
            if burst {
                v.started = true;
            }
            v.tx.try_send(Arc::clone(chunk)).is_ok()
        });
        viewers.len()
    }

    fn count(&self) -> usize {
        self.viewers.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// How to launch the head unit. The phone is found by the child itself over USB.
#[derive(Debug, Clone)]
pub struct NativeLaunch {
    pub program: PathBuf,
    /// The adb serial, for status and tunnels only. The child cannot choose among several phones.
    pub serial: Option<String>,
}

enum Incoming {
    Line(Line),
    /// A line that is not protocol JSON. Kept so a child failure is never silently dropped.
    Junk(String),
    Eof,
}

pub struct NativeBackend {
    child: Child,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    video: Arc<VideoHub>,
    audio: Arc<[VideoHub; 3]>,
    rx: Receiver<Incoming>,
    next_id: u64,
    serial: Option<String>,
    version: String,
    build: String,
    /// The phone's own last word on video, from its start and stop indications.
    phone_video: FocusState,
    ended: Option<String>,
}

fn io_err(what: &str, e: impl std::fmt::Display) -> Error {
    Error::BackendUnavailable(format!("{what}: {e}"))
}

impl NativeBackend {
    pub fn launch(launch: &NativeLaunch) -> Result<Self> {
        Self::launch_with_ready_timeout(launch, &[] as &[&OsStr], READY_TIMEOUT)
    }

    /// `args` is for tests that drive a fake child.
    pub fn launch_with_ready_timeout<S: AsRef<OsStr>>(
        launch: &NativeLaunch,
        args: &[S],
        ready_timeout: Duration,
    ) -> Result<Self> {
        let mut child = Command::new(&launch.program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| {
                Error::BackendUnavailable(format!("cannot start {}: {e}", launch.program.display()))
            })?;
        let stdin = Arc::new(Mutex::new(child.stdin.take()));
        let stdout = child.stdout.take().expect("stdout is piped");
        let video = Arc::new(VideoHub::default());
        let audio: Arc<[VideoHub; 3]> = Arc::default();
        let (tx, rx) = mpsc::channel();
        {
            let (video, audio, stdin) =
                (Arc::clone(&video), Arc::clone(&audio), Arc::clone(&stdin));
            thread::spawn(move || read_child(stdout, &tx, &video, &audio, &stdin));
        }
        let mut backend = Self {
            child,
            stdin,
            video,
            audio,
            rx,
            next_id: 1,
            serial: launch.serial.clone(),
            version: String::new(),
            build: String::new(),
            phone_video: FocusState::Unknown,
            ended: None,
        };
        backend.wait_ready(ready_timeout)?;
        Ok(backend)
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Watch the phone's video live. The first chunk is the parameter sets and the last keyframe,
    /// so a viewer can start decoding at once. Dropping the receiver ends the subscription, and
    /// the head unit stops streaming when the last viewer has gone.
    pub fn subscribe_video(&mut self) -> Result<Receiver<VideoChunk>> {
        let rx = self.video.add(false);
        // Asking again also makes the head unit resend the keyframe for a viewer that joins late.
        match self.send(Cmd::VideoStream { enabled: true }, STATUS_TIMEOUT) {
            Ok(Reply::Done) => Ok(rx),
            Ok(_) => Err(unexpected("video stream")),
            Err(e) => Err(e),
        }
    }

    /// Listen to one channel of the phone's audio: raw 16-bit PCM (`AudioChannel::format`). Chunks
    /// arrive only while the phone plays something on that channel. Dropping the receiver ends it.
    pub fn subscribe_audio(&mut self, channel: AudioChannel) -> Result<Receiver<VideoChunk>> {
        let rx = self.audio[audio_slot(channel)].add(true);
        match self.send(
            Cmd::AudioStream {
                channel,
                enabled: true,
            },
            STATUS_TIMEOUT,
        )? {
            Reply::Done => Ok(rx),
            _ => Err(unexpected("audio stream")),
        }
    }

    /// How many viewers are watching the video.
    pub fn video_viewers(&self) -> usize {
        self.video.count()
    }

    /// `Some(description)` once the child has exited or reported the link as gone.
    pub fn exit_status(&mut self) -> Option<String> {
        self.drain();
        if let Some(reason) = &self.ended {
            return Some(reason.clone());
        }
        match self.child.try_wait() {
            Ok(Some(st)) => {
                let reason = format!("the native head unit exited ({st})");
                self.ended = Some(reason.clone());
                Some(reason)
            }
            _ => None,
        }
    }

    fn wait_ready(&mut self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        let mut junk = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(Incoming::Line(Line::Event(Event::Ready {
                    backend_version,
                    backend_build,
                }))) => {
                    self.version = backend_version;
                    self.build = backend_build;
                    self.phone_video = FocusState::On;
                    return Ok(());
                }
                Ok(Incoming::Line(Line::Event(Event::Disconnected { reason }))) => {
                    self.kill();
                    return Err(Error::DeviceDisconnected(reason));
                }
                Ok(Incoming::Line(_)) => {}
                Ok(Incoming::Junk(j)) => junk.push(j),
                Ok(Incoming::Eof) | Err(RecvTimeoutError::Disconnected) => {
                    let st = self.child.wait().ok();
                    return Err(Error::BackendUnavailable(format!(
                        "the native head unit exited before the phone started projecting{}{}",
                        st.map(|s| format!(" ({s})")).unwrap_or_default(),
                        if junk.is_empty() {
                            String::new()
                        } else {
                            format!(": {}", junk.join("; "))
                        }
                    )));
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.kill();
                    return Err(Error::Timeout(format!(
                        "the phone did not start projecting within {}s (unlock it, and check that Android Auto is installed and that no DHU holds the phone)",
                        timeout.as_secs()
                    )));
                }
            }
        }
    }

    /// Apply whatever the child announced since the last call.
    fn drain(&mut self) {
        while let Ok(item) = self.rx.try_recv() {
            self.absorb(&item);
        }
    }

    fn absorb(&mut self, item: &Incoming) {
        match item {
            Incoming::Line(Line::Event(Event::Video { state })) => self.phone_video = *state,
            Incoming::Line(Line::Event(Event::Disconnected { reason })) => {
                self.ended = Some(reason.clone());
            }
            Incoming::Eof if self.ended.is_none() => {
                self.ended = Some("the native head unit closed its output".into());
            }
            _ => {}
        }
    }

    fn send(&mut self, command: Cmd, timeout: Duration) -> Result<Reply> {
        if let Some(reason) = self.exit_status() {
            return Err(Error::BackendUnavailable(reason));
        }
        let id = self.next_id;
        self.next_id += 1;
        let line = protocol::encode(&Request { id, command });
        write_line(&self.stdin, &line)?;
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let item = self.rx.recv_timeout(left).map_err(|e| match e {
                RecvTimeoutError::Timeout => Error::Timeout(format!(
                    "the native head unit did not answer within {}s",
                    timeout.as_secs()
                )),
                RecvTimeoutError::Disconnected => {
                    Error::BackendUnavailable("the native head unit closed its output".into())
                }
            })?;
            self.absorb(&item);
            match item {
                Incoming::Line(Line::Response(r)) if r.id == id => {
                    return match r.outcome {
                        Outcome::Ok(reply) => Ok(reply),
                        Outcome::Err(ErrorInfo { kind, message }) => Err(match kind {
                            ErrorKind::InvalidArgs => Error::InvalidArgs(message),
                            ErrorKind::Unavailable => Error::BackendUnavailable(message),
                            ErrorKind::Timeout => Error::Timeout(message),
                            ErrorKind::Other => Error::Other(message),
                        }),
                    };
                }
                Incoming::Eof => {
                    return Err(Error::BackendUnavailable(
                        self.ended
                            .clone()
                            .unwrap_or_else(|| "the native head unit exited".into()),
                    ));
                }
                _ => {}
            }
        }
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn write_line(stdin: &Mutex<Option<ChildStdin>>, line: &str) -> Result<()> {
    let mut guard = stdin.lock().unwrap_or_else(|e| e.into_inner());
    let stdin = guard
        .as_mut()
        .ok_or_else(|| Error::BackendUnavailable("the head unit is stopping".into()))?;
    stdin
        .write_all(line.as_bytes())
        .and_then(|()| stdin.flush())
        .map_err(|e| io_err("cannot write to the native head unit", e))
}

/// Read the child's output: protocol lines for `rx`, and the raw video that follows a `VideoData`
/// line for the viewers. When the last viewer is gone the child is told to stop streaming.
fn read_child(
    stdout: impl Read,
    tx: &mpsc::Sender<Incoming>,
    video: &VideoHub,
    audio: &[VideoHub; 3],
    stdin: &Mutex<Option<ChildStdin>>,
) {
    let mut out = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        match out.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let text = line.trim_end();
        match protocol::decode_line(text) {
            Ok(Line::Event(Event::VideoData { len, burst })) => {
                let mut data = vec![0u8; len];
                if out.read_exact(&mut data).is_err() {
                    break;
                }
                if video.publish(&Arc::new(data), burst) == 0 {
                    // Nobody is watching any more.
                    let _ = write_line(
                        stdin,
                        &protocol::encode(&Request {
                            id: 0,
                            command: Cmd::VideoStream { enabled: false },
                        }),
                    );
                }
            }
            Ok(Line::Event(Event::AudioData { channel, len })) => {
                let mut data = vec![0u8; len];
                if out.read_exact(&mut data).is_err() {
                    break;
                }
                if audio[audio_slot(channel)].publish(&Arc::new(data), false) == 0 {
                    let _ = write_line(
                        stdin,
                        &protocol::encode(&Request {
                            id: 0,
                            command: Cmd::AudioStream {
                                channel,
                                enabled: false,
                            },
                        }),
                    );
                }
            }
            Ok(l) => {
                if tx.send(Incoming::Line(l)).is_err() {
                    return;
                }
            }
            Err(_) => {
                if tx.send(Incoming::Junk(text.to_string())).is_err() {
                    return;
                }
            }
        }
    }
    let _ = tx.send(Incoming::Eof);
}

fn audio_slot(channel: AudioChannel) -> usize {
    match channel {
        AudioChannel::Media => 0,
        AudioChannel::System => 1,
        AudioChannel::Speech => 2,
    }
}

fn unexpected(what: &str) -> Error {
    Error::Other(format!(
        "the native head unit sent an unexpected reply to {what}"
    ))
}

impl HeadUnitBackend for NativeBackend {
    fn capabilities(&mut self) -> Result<Capabilities> {
        // Video focus, screenshots and touch were exercised on a real phone; the rest is not implemented.
        Ok(Capabilities {
            backend: "native".into(),
            video_focus: Support::Supported,
            screenshots: Support::Supported,
            audio_focus: Support::Unverified,
            nav_focus: Support::Unverified,
            night_mode: Support::Unverified,
            touch_input: Support::Supported,
            key_input: Support::Unverified, // becomes Supported once checked on a phone
            screen_recording: Support::Unsupported,
        })
    }

    fn status(&mut self) -> Result<HeadUnitStatus> {
        match self.send(Cmd::Status, STATUS_TIMEOUT)? {
            Reply::Status(report) => Ok(HeadUnitStatus {
                backend: "native".into(),
                state: SessionState::Running,
                serial: self.serial.clone(),
                backend_version: Some(self.version.clone()).filter(|v| !v.is_empty()),
                backend_build: Some(self.build.clone()).filter(|v| !v.is_empty()),
                video_focus: VideoFocusStatus {
                    requested: report.video_focus.requested,
                    backend_reported: report.video_focus.backend_reported,
                    observed: report.video_focus.observed,
                },
            }),
            _ => Err(unexpected("status")),
        }
    }

    fn set_video_focus(&mut self, enabled: bool) -> Result<OperationResult> {
        match self.send(Cmd::Focus { enabled }, FOCUS_TIMEOUT)? {
            Reply::Focus(r) => Ok(r),
            _ => Err(unexpected("focus")),
        }
    }

    fn screenshot(&mut self, path: &Path) -> Result<ScreenshotResult> {
        match self.send(
            Cmd::Screenshot {
                path: path.to_path_buf(),
            },
            SHOT_TIMEOUT,
        )? {
            Reply::Screenshot(r) => Ok(r),
            _ => Err(unexpected("screenshot")),
        }
    }

    fn tap(&mut self, x: u32, y: u32) -> Result<InputResult> {
        match self.send(Cmd::Tap { x, y }, TOUCH_TIMEOUT)? {
            Reply::Input(r) => Ok(r),
            _ => Err(unexpected("tap")),
        }
    }

    fn swipe(
        &mut self,
        from: (u32, u32),
        to: (u32, u32),
        duration: Duration,
    ) -> Result<InputResult> {
        let command = Cmd::Swipe {
            x1: from.0,
            y1: from.1,
            x2: to.0,
            y2: to.1,
            duration_ms: u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
        };
        match self.send(command, duration + TOUCH_TIMEOUT)? {
            Reply::Input(r) => Ok(r),
            _ => Err(unexpected("swipe")),
        }
    }

    fn press_key(&mut self, key: Key, long_press: bool) -> Result<InputResult> {
        match self.send(Cmd::Key { key, long_press }, TOUCH_TIMEOUT)? {
            Reply::Input(r) => Ok(r),
            _ => Err(unexpected("key")),
        }
    }

    fn stop(&mut self) -> Result<()> {
        if self.child.try_wait().ok().flatten().is_some() {
            return Ok(());
        }
        // Ask politely, then close stdin (the child's real shutdown signal), then insist.
        let _ = write_line(
            &self.stdin,
            &protocol::encode(&Request {
                id: 0,
                command: Cmd::Quit,
            }),
        );
        self.stdin.lock().unwrap_or_else(|e| e.into_inner()).take();
        let deadline = Instant::now() + STOP_GRACE;
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
        self.kill();
        Err(Error::Other(
            "the native head unit did not quit gracefully and was killed".into(),
        ))
    }
}

impl Drop for NativeBackend {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            self.stdin.lock().unwrap_or_else(|e| e.into_inner()).take();
            thread::sleep(Duration::from_millis(100));
            if self.child.try_wait().ok().flatten().is_none() {
                self.kill();
            }
        }
    }
}

#[cfg(test)]
mod hub_tests {
    use super::*;

    fn chunk(s: &str) -> VideoChunk {
        Arc::new(s.as_bytes().to_vec())
    }

    #[test]
    fn a_new_viewer_gets_nothing_until_a_burst_then_live_chunks_and_no_second_burst() {
        let hub = VideoHub::default();
        let rx = hub.add(false);
        assert_eq!(hub.publish(&chunk("live-1"), false), 1);
        assert!(
            rx.try_recv().is_err(),
            "live frames before the burst are undecodable"
        );
        hub.publish(&chunk("burst"), true);
        assert_eq!(rx.try_recv().unwrap().as_slice(), b"burst");
        hub.publish(&chunk("live-2"), false);
        assert_eq!(rx.try_recv().unwrap().as_slice(), b"live-2");
        // Another viewer's burst must not repeat itself to a viewer that is already running.
        hub.publish(&chunk("burst-for-someone-else"), true);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn an_audio_listener_hears_live_chunks_at_once_because_audio_has_no_burst() {
        let hub = VideoHub::default();
        let rx = hub.add(true);
        hub.publish(&chunk("pcm-1"), false);
        assert_eq!(rx.try_recv().unwrap().as_slice(), b"pcm-1");
    }

    #[test]
    fn a_late_viewer_is_started_by_a_burst_without_disturbing_the_running_one() {
        let hub = VideoHub::default();
        let first = hub.add(false);
        hub.publish(&chunk("burst-1"), true);
        let second = hub.add(false);
        hub.publish(&chunk("live"), false);
        hub.publish(&chunk("burst-2"), true);
        hub.publish(&chunk("live-after"), false);
        let got = |rx: &Receiver<VideoChunk>| -> Vec<String> {
            rx.try_iter()
                .map(|c| String::from_utf8_lossy(&c).into_owned())
                .collect()
        };
        assert_eq!(got(&first), ["burst-1", "live", "live-after"]);
        assert_eq!(got(&second), ["burst-2", "live-after"]);
    }

    #[test]
    fn a_viewer_that_is_gone_or_too_slow_is_dropped_and_the_count_reaches_zero() {
        let hub = VideoHub::default();
        let rx = hub.add(false);
        hub.publish(&chunk("burst"), true);
        drop(rx);
        assert_eq!(hub.publish(&chunk("live"), false), 0);
        let slow = hub.add(false);
        hub.publish(&chunk("burst"), true);
        let mut remaining = 1;
        for _ in 0..(VIEWER_BACKLOG + 5) {
            remaining = hub.publish(&chunk("x"), false);
        }
        assert_eq!(
            remaining, 0,
            "a viewer that never reads is dropped, not waited for"
        );
        drop(slow);
    }
}

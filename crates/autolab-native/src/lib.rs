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
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use autolab_core::{
    Capabilities, Error, FocusState, HeadUnitBackend, HeadUnitStatus, OperationResult, Result,
    ScreenshotResult, SessionState, Support, VideoFocusStatus,
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
const STOP_GRACE: Duration = Duration::from_secs(5);

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
    stdin: Option<ChildStdin>,
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
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("stdout is piped");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let item = match protocol::decode_line(&line) {
                    Ok(l) => Incoming::Line(l),
                    Err(_) => Incoming::Junk(line),
                };
                if tx.send(item).is_err() {
                    return;
                }
            }
            let _ = tx.send(Incoming::Eof);
        });
        let mut backend = Self {
            child,
            stdin,
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
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| Error::BackendUnavailable("the head unit is stopping".into()))?;
        stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.flush())
            .map_err(|e| io_err("cannot write to the native head unit", e))?;
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

fn unexpected(what: &str) -> Error {
    Error::Other(format!(
        "the native head unit sent an unexpected reply to {what}"
    ))
}

impl HeadUnitBackend for NativeBackend {
    fn capabilities(&mut self) -> Result<Capabilities> {
        // Video focus and screenshots were exercised on a real phone; the rest is not implemented.
        Ok(Capabilities {
            backend: "native".into(),
            video_focus: Support::Supported,
            screenshots: Support::Supported,
            audio_focus: Support::Unverified,
            nav_focus: Support::Unverified,
            night_mode: Support::Unverified,
            touch_input: Support::Unverified,
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

    fn stop(&mut self) -> Result<()> {
        if self.child.try_wait().ok().flatten().is_some() {
            return Ok(());
        }
        // Ask politely, then close stdin (the child's real shutdown signal), then insist.
        if let Some(mut stdin) = self.stdin.take() {
            let _ = stdin.write_all(
                protocol::encode(&Request {
                    id: 0,
                    command: Cmd::Quit,
                })
                .as_bytes(),
            );
        }
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
            self.stdin.take();
            thread::sleep(Duration::from_millis(100));
            if self.child.try_wait().ok().flatten().is_none() {
                self.kill();
            }
        }
    }
}

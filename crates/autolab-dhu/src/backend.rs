use std::ffi::OsString;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use autolab_core::{
    Capabilities, Error, FocusState, HeadUnitBackend, HeadUnitStatus, OperationResult, Result,
    ScreenshotResult, SessionState, Support, VideoFocusStatus,
};

use crate::console::{self, DhuInfo};
use crate::png;
use crate::process::DhuProcess;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
/// The DHU replies before the file is necessarily complete, so wait for it to settle.
const SCREENSHOT_SETTLE: Duration = Duration::from_secs(3);
const READY_TIMEOUT: Duration = Duration::from_secs(45);
const STOP_GRACE: Duration = Duration::from_secs(5);

/// How to launch the DHU. Only the USB transport has been verified on real hardware.
#[derive(Debug, Clone)]
pub struct DhuLaunch {
    pub program: PathBuf,
    pub serial: String,
    pub config: Option<PathBuf>,
    pub headless: bool,
}

impl DhuLaunch {
    pub fn args(&self) -> Vec<OsString> {
        let mut a = vec![OsString::from(format!("--usb={}", self.serial))];
        if let Some(c) = &self.config {
            let mut s = OsString::from("--config=");
            s.push(c);
            a.push(s);
        }
        if self.headless {
            a.push("--headless".into());
        }
        a
    }
}

pub struct DhuBackend {
    proc: DhuProcess,
    info: DhuInfo,
    requested_video: FocusState,
    serial: Option<String>,
}

impl DhuBackend {
    pub fn launch(launch: &DhuLaunch) -> Result<Self> {
        let proc = DhuProcess::spawn(&launch.program, &launch.args(), &[])?;
        let mut backend = Self::from_process(proc, READY_TIMEOUT)?;
        backend.serial = Some(launch.serial.clone());
        Ok(backend)
    }

    /// Wrap an already spawned process (used by tests with a fake DHU).
    pub fn from_process(mut proc: DhuProcess, ready_timeout: Duration) -> Result<Self> {
        proc.wait_ready(ready_timeout)?;
        let info = console::parse_info(proc.startup_lines());
        Ok(Self {
            proc,
            info,
            requested_video: FocusState::Unknown,
            serial: None,
        })
    }

    pub fn pid(&self) -> u32 {
        self.proc.id()
    }

    pub fn info(&self) -> &DhuInfo {
        &self.info
    }

    pub fn requested_video(&self) -> FocusState {
        self.requested_video
    }

    /// `Some(description)` once the DHU process has exited.
    pub fn exit_status(&mut self) -> Option<String> {
        self.proc.exit_status()
    }

    /// Ask the DHU for its own idea of video focus. This is DHU-side state, not a phone ack.
    pub fn read_video_focus(&mut self) -> Result<FocusState> {
        let lines = self
            .proc
            .round_trip(console::VIDEO_FOCUS_QUERY, COMMAND_TIMEOUT)?;
        Ok(console::parse_video_focus(&lines))
    }
}

impl HeadUnitBackend for DhuBackend {
    fn capabilities(&mut self) -> Result<Capabilities> {
        Ok(Capabilities {
            backend: "dhu".into(),
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
        if let Some(st) = self.exit_status() {
            return Err(Error::BackendUnavailable(format!("DHU has exited ({st})")));
        }
        let backend_reported = self.read_video_focus()?;
        Ok(HeadUnitStatus {
            backend: "dhu".into(),
            state: SessionState::Running,
            serial: self.serial.clone(),
            backend_version: self.info.version.clone(),
            backend_build: self.info.build.clone(),
            video_focus: VideoFocusStatus {
                requested: self.requested_video,
                backend_reported,
                observed: FocusState::Unknown,
            },
        })
    }

    fn set_video_focus(&mut self, enabled: bool) -> Result<OperationResult> {
        let requested = FocusState::from_enabled(enabled);
        let lines = self
            .proc
            .round_trip(console::video_focus_command(enabled), COMMAND_TIMEOUT)?;
        let errors = console::error_lines(&lines);
        if !errors.is_empty() {
            return Ok(OperationResult::failed(requested, lines));
        }
        self.requested_video = requested;
        let reported = self.read_video_focus()?;
        Ok(OperationResult::accepted(requested, reported, lines))
    }

    fn screenshot(&mut self, path: &Path) -> Result<ScreenshotResult> {
        let text = path
            .to_str()
            .ok_or_else(|| Error::InvalidArgs("screenshot path must be valid UTF-8".into()))?;
        // How the DHU parses quoting is unverified, so only unambiguous paths are passed through.
        if !path.is_absolute() || text.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(Error::InvalidArgs(
                "screenshot path must be absolute and contain no whitespace".into(),
            ));
        }
        // A leftover file would otherwise satisfy the check below without a new capture.
        if path.is_file() {
            std::fs::remove_file(path)
                .map_err(|e| Error::Other(format!("cannot replace {text}: {e}")))?;
        }
        let lines = self
            .proc
            .round_trip(&format!("screenshot {text}"), COMMAND_TIMEOUT)?;
        let errors = console::error_lines(&lines);
        if !errors.is_empty() {
            return Err(Error::Other(format!(
                "DHU rejected the screenshot: {}",
                errors.join("; ")
            )));
        }
        let (width, height) = wait_for_png(path)?;
        Ok(ScreenshotResult {
            path: path.to_path_buf(),
            width,
            height,
        })
    }

    fn stop(&mut self) -> Result<()> {
        if !self.proc.shutdown(STOP_GRACE) {
            return Err(Error::Other(
                "DHU did not quit gracefully and was killed".into(),
            ));
        }
        Ok(())
    }
}

/// Wait until `path` holds a complete-looking PNG (size stable across two polls).
fn wait_for_png(path: &Path) -> Result<(u32, u32)> {
    let deadline = Instant::now() + SCREENSHOT_SETTLE;
    let mut last_len = None;
    loop {
        if let Ok(meta) = std::fs::metadata(path) {
            let len = meta.len();
            if len >= 24 && last_len == Some(len) {
                let mut head = [0u8; 24];
                if let Ok(mut f) = File::open(path)
                    && f.read_exact(&mut head).is_ok()
                {
                    return png::dimensions(&head).ok_or_else(|| {
                        Error::Other(format!("{} was written but is not a PNG", path.display()))
                    });
                }
            }
            last_len = Some(len);
        }
        if Instant::now() >= deadline {
            return Err(Error::Timeout(format!(
                "the DHU accepted the screenshot command but {} did not appear within {}s",
                path.display(),
                SCREENSHOT_SETTLE.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_args_are_a_structured_argv() {
        let l = DhuLaunch {
            program: "/x/dhu".into(),
            serial: "S 1; rm -rf /".into(),
            config: Some("/c/my config.ini".into()),
            headless: true,
        };
        assert_eq!(
            l.args(),
            vec![
                OsString::from("--usb=S 1; rm -rf /"),
                OsString::from("--config=/c/my config.ini"),
                OsString::from("--headless")
            ]
        );
    }
}

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use autolab_core::{
    Capabilities, Error, FocusState, HeadUnitBackend, OperationResult, Result, Support,
};

use crate::console::{self, DhuInfo};
use crate::process::DhuProcess;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
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
}

impl DhuBackend {
    pub fn launch(launch: &DhuLaunch) -> Result<Self> {
        let proc = DhuProcess::spawn(&launch.program, &launch.args(), &[])?;
        Self::from_process(proc, READY_TIMEOUT)
    }

    /// Wrap an already spawned process (used by tests with a fake DHU).
    pub fn from_process(mut proc: DhuProcess, ready_timeout: Duration) -> Result<Self> {
        proc.wait_ready(ready_timeout)?;
        let info = console::parse_info(proc.startup_lines());
        Ok(Self {
            proc,
            info,
            requested_video: FocusState::Unknown,
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
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            backend: "dhu".into(),
            video_focus: Support::Supported,
        }
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

    fn stop(&mut self) -> Result<()> {
        if !self.proc.shutdown(STOP_GRACE) {
            return Err(Error::Other(
                "DHU did not quit gracefully and was killed".into(),
            ));
        }
        Ok(())
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

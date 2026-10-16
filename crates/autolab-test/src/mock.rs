//! A scriptable backend and clock for tests. Public so other crates' tests can use them too.

use std::cell::Cell;
use std::path::Path;
use std::time::Duration;

use autolab_core::{
    Capabilities, Error, FocusState, HeadUnitBackend, HeadUnitStatus, InputResult, OpStatus,
    OperationResult, Result, ScreenshotResult, SessionState, Support, VideoFocusStatus,
};

use crate::runner::Clock;

/// Time that only moves when something sleeps. No real delay ever happens.
#[derive(Default)]
pub struct FakeClock {
    now: Cell<Duration>,
    sleeps: Cell<usize>,
    /// Report `cancelled` once this many sleeps have happened, as if Ctrl+C arrived then.
    pub cancel_after_sleeps: Cell<Option<usize>>,
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        self.now.get()
    }
    fn sleep(&self, d: Duration) {
        self.sleeps.set(self.sleeps.get() + 1);
        // An interrupted sleep returns early, like the real clock's.
        if !self.cancelled() {
            self.now.set(self.now.get() + d);
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel_after_sleeps
            .get()
            .is_some_and(|n| self.sleeps.get() >= n)
    }
}

type Failure = Option<(u8, String)>;

pub struct MockBackend {
    pub caps: Capabilities,
    pub state: HeadUnitStatus,
    /// Every call, in order, e.g. `focus.video off`, `status`, `screenshot`.
    pub calls: Vec<String>,
    /// What the backend reports after a focus command, instead of echoing the request.
    pub readback_override: Option<FocusState>,
    pub reject_focus: bool,
    /// Fail a focus command whose requested value equals `.0`, with exit code and message `.1`.
    pub focus_error_when: Option<(bool, u8, String)>,
    pub screenshot_error: Failure,
    pub status_error: Failure,
    /// After this many `status` calls the phone "confirms" video focus on.
    pub observed_on_after_status_calls: Option<usize>,
    status_calls: usize,
}

impl Default for MockBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MockBackend {
    pub fn new() -> Self {
        Self {
            caps: Capabilities {
                backend: "dhu".into(),
                video_focus: Support::Supported,
                screenshots: Support::Supported,
                audio_focus: Support::Unverified,
                nav_focus: Support::Unverified,
                night_mode: Support::Unverified,
                touch_input: Support::Unverified,
                key_input: Support::Unverified,
                screen_recording: Support::Unsupported,
            },
            state: HeadUnitStatus {
                backend: "dhu".into(),
                state: SessionState::Running,
                serial: Some("MOCK0001".into()),
                backend_version: Some("0.0-mock".into()),
                backend_build: Some("mock-build".into()),
                certificate_source: None,
                screen_width: None,
                screen_height: None,
                video_focus: VideoFocusStatus {
                    requested: FocusState::Unknown,
                    backend_reported: FocusState::On,
                    observed: FocusState::Unknown,
                },
            },
            calls: Vec::new(),
            readback_override: None,
            reject_focus: false,
            focus_error_when: None,
            screenshot_error: None,
            status_error: None,
            observed_on_after_status_calls: None,
            status_calls: 0,
        }
    }
}

fn fail(f: &Failure) -> Result<()> {
    match f {
        Some((code, msg)) => Err(Error::from_code(*code, msg.clone())),
        None => Ok(()),
    }
}

impl HeadUnitBackend for MockBackend {
    fn tap(&mut self, x: u32, y: u32) -> Result<InputResult> {
        self.calls.push(format!("tap {x} {y}"));
        Ok(InputResult {
            status: OpStatus::Accepted,
            evidence: vec!["mock".into()],
        })
    }

    fn press_key(&mut self, key: autolab_core::Key, long_press: bool) -> Result<InputResult> {
        self.calls.push(format!(
            "key {}{}",
            key.name(),
            if long_press { " long" } else { "" }
        ));
        Ok(InputResult {
            status: OpStatus::Accepted,
            evidence: vec!["mock".into()],
        })
    }

    fn swipe(&mut self, from: (u32, u32), to: (u32, u32), d: Duration) -> Result<InputResult> {
        self.calls.push(format!(
            "swipe {} {} {} {} {}ms",
            from.0,
            from.1,
            to.0,
            to.1,
            d.as_millis()
        ));
        Ok(InputResult {
            status: OpStatus::Accepted,
            evidence: vec!["mock".into()],
        })
    }

    fn capabilities(&mut self) -> Result<Capabilities> {
        self.calls.push("capabilities".into());
        Ok(self.caps.clone())
    }

    fn status(&mut self) -> Result<HeadUnitStatus> {
        self.calls.push("status".into());
        self.status_calls += 1;
        fail(&self.status_error)?;
        if let Some(n) = self.observed_on_after_status_calls
            && self.status_calls >= n
        {
            self.state.video_focus.observed = FocusState::On;
        }
        Ok(self.state.clone())
    }

    fn set_video_focus(&mut self, enabled: bool) -> Result<OperationResult> {
        self.calls.push(format!(
            "focus.video {}",
            if enabled { "on" } else { "off" }
        ));
        if let Some((when, code, msg)) = &self.focus_error_when
            && *when == enabled
        {
            return Err(Error::from_code(*code, msg.clone()));
        }
        let requested = FocusState::from_enabled(enabled);
        if self.reject_focus {
            return Ok(OperationResult::failed(
                requested,
                vec!["[E]: rejected".into()],
            ));
        }
        let reported = self.readback_override.unwrap_or(requested);
        self.state.video_focus.requested = requested;
        self.state.video_focus.backend_reported = reported;
        Ok(OperationResult::accepted(requested, reported, vec![]))
    }

    fn screenshot(&mut self, path: &Path) -> Result<ScreenshotResult> {
        self.calls.push("screenshot".into());
        fail(&self.screenshot_error)?;
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
        png.extend(b"IHDR");
        png.extend(1280u32.to_be_bytes());
        png.extend(720u32.to_be_bytes());
        std::fs::write(path, png).map_err(|e| Error::Other(e.to_string()))?;
        Ok(ScreenshotResult {
            path: path.to_path_buf(),
            width: 1280,
            height: 720,
        })
    }

    fn stop(&mut self) -> Result<()> {
        self.calls.push("stop".into());
        Ok(())
    }
}

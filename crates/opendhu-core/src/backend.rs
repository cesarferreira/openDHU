use serde::{Deserialize, Serialize};

use std::path::Path;
use std::time::Duration;

use crate::{Error, HeadUnitStatus, InputResult, Key, OperationResult, Result, ScreenshotResult};

/// Capability levels. `Unverified` means "not yet proven against real hardware".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Support {
    Supported,
    Unsupported,
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub backend: String,
    pub video_focus: Support,
    pub screenshots: Support,
    // The rest are not implemented in openDHU yet. `Unverified` means the backend may offer it
    // (the DHU console has the command) but openDHU has not exercised it; `Unsupported` means the
    // backend has no way to do it.
    pub audio_focus: Support,
    pub nav_focus: Support,
    pub night_mode: Support,
    pub touch_input: Support,
    pub key_input: Support,
    pub screen_recording: Support,
}

/// The only way callers (CLI, future test runner, daemon) reach a head unit.
/// Deliberately small: grow it only when a verified operation needs it.
pub trait HeadUnitBackend {
    /// Queried, not cached: a backend may need to ask a live session.
    fn capabilities(&mut self) -> Result<Capabilities>;
    fn status(&mut self) -> Result<HeadUnitStatus>;
    fn set_video_focus(&mut self, enabled: bool) -> Result<OperationResult>;
    /// Write a PNG of the projected display to `path` (absolute) and verify it exists.
    fn screenshot(&mut self, path: &Path) -> Result<ScreenshotResult>;
    /// Touch the projected screen at a pixel of the screenshot (origin top left).
    fn tap(&mut self, _x: u32, _y: u32) -> Result<InputResult> {
        Err(Error::Unsupported(
            "touch input is not supported by this backend".into(),
        ))
    }
    /// Drag from one pixel to another over `duration`.
    fn swipe(
        &mut self,
        _from: (u32, u32),
        _to: (u32, u32),
        _duration: Duration,
    ) -> Result<InputResult> {
        Err(Error::Unsupported(
            "touch input is not supported by this backend".into(),
        ))
    }
    /// Press one of the car's buttons. A long press is held for about a second.
    fn press_key(&mut self, _key: Key, _long_press: bool) -> Result<InputResult> {
        Err(Error::Unsupported(
            "key input is not supported by this backend".into(),
        ))
    }
    fn stop(&mut self) -> Result<()>;
}

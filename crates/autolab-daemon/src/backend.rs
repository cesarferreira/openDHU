//! The head unit a session runs. One enum instead of a trait object, because the daemon also needs
//! each backend's process id and liveness, which are not part of `HeadUnitBackend`.

use std::path::Path;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use autolab_core::{
    Capabilities, Error, HeadUnitBackend, HeadUnitStatus, InputResult, OperationResult, Result,
    ScreenshotResult,
};
use autolab_dhu::DhuBackend;
use autolab_native::{NativeBackend, VideoChunk};

pub enum AnyBackend {
    Dhu(DhuBackend),
    Native(NativeBackend),
}

impl AnyBackend {
    pub fn pid(&self) -> u32 {
        match self {
            Self::Dhu(b) => b.pid(),
            Self::Native(b) => b.pid(),
        }
    }

    /// Watch the phone's video. Only the native backend has any: the DHU draws it in its own window.
    pub fn subscribe_video(&mut self) -> Result<Receiver<VideoChunk>> {
        match self {
            Self::Native(b) => b.subscribe_video(),
            Self::Dhu(_) => Err(Error::Unsupported(
                "live video needs the native backend; the DHU shows it in its own window".into(),
            )),
        }
    }

    /// `Some(description)` once the head unit has exited or its link is gone.
    pub fn exit_status(&mut self) -> Option<String> {
        match self {
            Self::Dhu(b) => b.exit_status(),
            Self::Native(b) => b.exit_status(),
        }
    }
}

impl HeadUnitBackend for AnyBackend {
    fn capabilities(&mut self) -> Result<Capabilities> {
        match self {
            Self::Dhu(b) => b.capabilities(),
            Self::Native(b) => b.capabilities(),
        }
    }
    fn status(&mut self) -> Result<HeadUnitStatus> {
        match self {
            Self::Dhu(b) => b.status(),
            Self::Native(b) => b.status(),
        }
    }
    fn set_video_focus(&mut self, enabled: bool) -> Result<OperationResult> {
        match self {
            Self::Dhu(b) => b.set_video_focus(enabled),
            Self::Native(b) => b.set_video_focus(enabled),
        }
    }
    fn screenshot(&mut self, path: &Path) -> Result<ScreenshotResult> {
        match self {
            Self::Dhu(b) => b.screenshot(path),
            Self::Native(b) => b.screenshot(path),
        }
    }
    fn tap(&mut self, x: u32, y: u32) -> Result<InputResult> {
        match self {
            Self::Dhu(b) => b.tap(x, y),
            Self::Native(b) => b.tap(x, y),
        }
    }
    fn swipe(&mut self, from: (u32, u32), to: (u32, u32), d: Duration) -> Result<InputResult> {
        match self {
            Self::Dhu(b) => b.swipe(from, to, d),
            Self::Native(b) => b.swipe(from, to, d),
        }
    }
    fn stop(&mut self) -> Result<()> {
        match self {
            Self::Dhu(b) => b.stop(),
            Self::Native(b) => b.stop(),
        }
    }
}

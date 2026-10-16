//! The head unit a session runs. One enum instead of a trait object, because the daemon also needs
//! each backend's process id and liveness, which are not part of `HeadUnitBackend`.

use std::path::Path;

use autolab_core::{
    Capabilities, HeadUnitBackend, HeadUnitStatus, OperationResult, Result, ScreenshotResult,
};
use autolab_dhu::DhuBackend;
use autolab_native::NativeBackend;

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
    fn stop(&mut self) -> Result<()> {
        match self {
            Self::Dhu(b) => b.stop(),
            Self::Native(b) => b.stop(),
        }
    }
}

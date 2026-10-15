use serde::{Deserialize, Serialize};

use crate::{OperationResult, Result};

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
}

/// The only way callers (CLI, future test runner, daemon) reach a head unit.
/// Deliberately small: grow it only when a verified operation needs it.
pub trait HeadUnitBackend {
    fn capabilities(&self) -> Capabilities;
    fn set_video_focus(&mut self, enabled: bool) -> Result<OperationResult>;
    fn stop(&mut self) -> Result<()>;
}

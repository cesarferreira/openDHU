//! Backend-agnostic types for AutoLab. No GUI, process, or DHU knowledge lives here.

mod backend;
mod error;
mod result;
mod status;
mod time;

pub use backend::{Capabilities, HeadUnitBackend, Support};
pub use error::{Error, Result};
pub use result::{FocusState, OpStatus, OperationResult, ScreenshotResult};
pub use status::{HeadUnitStatus, SessionState, VideoFocusStatus};
pub use time::{iso8601_utc, unix_now};

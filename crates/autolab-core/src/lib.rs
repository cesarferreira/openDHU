//! Backend-agnostic types for AutoLab. No GUI, process, or DHU knowledge lives here.

mod audio;
mod backend;
mod error;
mod key;
mod result;
mod status;
mod time;

pub use audio::AudioChannel;
pub use backend::{Capabilities, HeadUnitBackend, Support};
pub use error::{Error, Result};
pub use key::Key;
pub use result::{FocusState, InputResult, OpStatus, OperationResult, ScreenshotResult};
pub use status::{HeadUnitStatus, SessionState, VideoFocusStatus};
pub use time::{iso8601_utc, unix_now};

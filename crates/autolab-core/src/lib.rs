//! Backend-agnostic types for AutoLab. No GUI, process, or DHU knowledge lives here.

mod backend;
mod error;
mod result;

pub use backend::{Capabilities, HeadUnitBackend, Support};
pub use error::{Error, Result};
pub use result::{FocusState, OpStatus, OperationResult};

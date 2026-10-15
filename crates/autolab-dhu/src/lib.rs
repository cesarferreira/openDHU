//! Backend adapter for Google's Desktop Head Unit (DHU).

pub mod adb;
mod backend;
pub mod console;
pub mod discovery;
mod process;

pub use backend::{DhuBackend, DhuLaunch};
pub use console::DhuInfo;
pub use process::DhuProcess;

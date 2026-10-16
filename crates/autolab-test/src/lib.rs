//! YAML test runner. Depends only on `autolab-core`, so it cannot call a head unit directly.

pub mod mock;
pub mod model;
pub mod report;
pub mod runner;
pub mod schema;

pub use model::{Outcome, StepResult, TestResult};
pub use report::{Format, Metadata, RunReport};
pub use runner::{Clock, RunConfig, SystemClock, run_test};
pub use schema::{TestFile, parse};

pub mod screen;

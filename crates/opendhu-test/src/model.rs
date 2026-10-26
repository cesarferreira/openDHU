use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Passed,
    /// An assertion or wait did not hold. Exit code 7, or 6 for a timeout.
    Failed,
    /// Something went wrong that is not a test verdict (backend down, bad setup, cleanup failed).
    Error,
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepResult {
    /// 1-based position in the test. `0` is the preflight check.
    pub index: usize,
    pub action: String,
    /// What the step is for: the test's `as:` label, else a description of its arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub outcome: Outcome,
    pub detail: String,
    pub duration_ms: u64,
    /// Process exit code this step maps to. `None` for passed and skipped steps.
    pub exit_code: Option<u8>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub name: String,
    pub file: String,
    pub outcome: Outcome,
    /// Why the test did not pass, in one line.
    pub message: Option<String>,
    pub steps: Vec<StepResult>,
    pub duration_ms: u64,
    pub exit_code: u8,
}

impl TestResult {
    pub fn passed(&self) -> bool {
        self.outcome == Outcome::Passed
    }
}

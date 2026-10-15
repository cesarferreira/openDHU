use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A state we either know or explicitly do not. Never guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FocusState {
    On,
    Off,
    Unknown,
}

impl FocusState {
    pub fn from_enabled(enabled: bool) -> Self {
        if enabled { Self::On } else { Self::Off }
    }
}

/// How far an operation got. Ordered by strength of evidence.
///
/// `Accepted` means the backend took the command. It says nothing about the phone.
/// `Acknowledged` and `Observed` require a signal that came from the device side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpStatus {
    Accepted,
    Acknowledged,
    Observed,
    Unsupported,
    Failed,
    TimedOut,
}

impl OpStatus {
    pub fn is_success(self) -> bool {
        matches!(self, Self::Accepted | Self::Acknowledged | Self::Observed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationResult {
    pub status: OpStatus,
    pub requested: FocusState,
    /// State the backend itself reports after the command (for the DHU: its own console
    /// readback). This is not proof that the phone changed state.
    pub backend_reported: FocusState,
    /// State confirmed by a signal from the phone. `Unknown` unless such a signal exists.
    pub observed: FocusState,
    /// Raw backend output lines that justify `status`, kept for diagnostics.
    pub evidence: Vec<String>,
}

impl OperationResult {
    pub fn accepted(
        requested: FocusState,
        backend_reported: FocusState,
        evidence: Vec<String>,
    ) -> Self {
        Self {
            status: OpStatus::Accepted,
            requested,
            backend_reported,
            observed: FocusState::Unknown,
            evidence,
        }
    }

    pub fn failed(requested: FocusState, evidence: Vec<String>) -> Self {
        Self {
            status: OpStatus::Failed,
            requested,
            backend_reported: FocusState::Unknown,
            observed: FocusState::Unknown,
            evidence,
        }
    }
}

/// A screenshot that exists on disk. Dimensions are read from the written PNG, not assumed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotResult {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_never_claims_observed_state() {
        let r = OperationResult::accepted(FocusState::Off, FocusState::Off, vec![]);
        assert_eq!(r.observed, FocusState::Unknown);
        assert_eq!(r.backend_reported, FocusState::Off);
        assert!(r.status.is_success());
    }

    #[test]
    fn json_uses_camel_case_keys() {
        let r = OperationResult::accepted(FocusState::On, FocusState::On, vec![]);
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["backendReported"], "on");
        assert!(v.get("backend_reported").is_none());
    }

    #[test]
    fn unsupported_and_failure_statuses_are_not_success() {
        for s in [OpStatus::Unsupported, OpStatus::Failed, OpStatus::TimedOut] {
            assert!(!s.is_success());
        }
    }
}

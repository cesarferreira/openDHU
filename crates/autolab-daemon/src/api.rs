//! Wire types for the v1 HTTP API, shared by the server and the blocking client.

use std::path::PathBuf;

use autolab_core::{Error, HeadUnitStatus, InputResult, OperationResult, ScreenshotResult};
use serde::{Deserialize, Serialize};

// ---- errors ----------------------------------------------------------------------------------

/// Every error response, including the framework's own (404, 405, malformed JSON), has this shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBody {
    pub code: String,
    /// The bare message, without a category prefix.
    pub message: String,
    /// The CLI exit code this error maps to, so clients can reproduce it exactly.
    pub exit_code: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
}

pub fn code_name(e: &Error) -> &'static str {
    match e {
        Error::InvalidArgs(_) => "invalid_args",
        Error::BackendUnavailable(_) => "backend_unavailable",
        Error::DeviceDisconnected(_) => "device_disconnected",
        Error::Unsupported(_) => "unsupported",
        Error::Timeout(_) => "timeout",
        Error::SessionConflict(_) => "session_conflict",
        Error::Other(_) => "error",
    }
}

impl BackendKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Dhu => "dhu",
            Self::Native => "native",
        }
    }
}

impl ErrorBody {
    pub fn from_error(e: &Error) -> Self {
        Self {
            code: code_name(e).into(),
            message: e.message().to_string(),
            exit_code: e.exit_code(),
        }
    }
}

// ---- requests --------------------------------------------------------------------------------

/// Which head unit to run. `dhu` is Google's Desktop Head Unit; `native` is AutoLab's own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    #[default]
    Dhu,
    Native,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartRequest {
    /// Default: `dhu`.
    #[serde(default)]
    pub backend: BackendKind,
    /// Android device serial. Default: the only authorized adb device.
    #[serde(default)]
    pub serial: Option<String>,
    /// DHU executable. Default: discovered from the environment and the Android SDK.
    #[serde(default)]
    pub dhu_path: Option<PathBuf>,
    #[serde(default)]
    pub config: Option<PathBuf>,
    #[serde(default)]
    pub headless: bool,
    /// Ports to keep reachable on the phone with `adb reverse tcp:PORT tcp:PORT`.
    #[serde(default)]
    pub reverse: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FocusRequest {
    pub enabled: bool,
}

/// A touch at a pixel of the screenshot (origin top left).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TapRequest {
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SwipeRequest {
    pub x1: u32,
    pub y1: u32,
    pub x2: u32,
    pub y2: u32,
    /// Default 300 ms; 50 to 10000.
    #[serde(default = "default_swipe_ms")]
    pub duration_ms: u64,
}

/// Press a car button. `key` is one of the names in `autolab_core::Key` (`home`, `back`, `up`, ...).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyRequest {
    pub key: autolab_core::Key,
    #[serde(default)]
    pub long_press: bool,
}

pub fn default_swipe_ms() -> u64 {
    300
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputResponse {
    pub operation_id: String,
    pub backend: BackendKind,
    #[serde(flatten)]
    pub result: InputResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScreenshotRequest {
    /// Absolute path ending in `.png`. The directory must exist.
    pub path: PathBuf,
}

// ---- responses -------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonInfo {
    pub version: String,
    pub pid: u32,
    pub port: u16,
    /// True for a daemon started by `aa start`: it exits when its session ends.
    pub exit_when_idle: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Starting,
    Running,
    Stopping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TunnelState {
    Active,
    Missing,
    /// adb could not be queried, so nothing is known.
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelStatus {
    pub port: u16,
    pub state: TunnelState,
    /// Times AutoLab re-applied this tunnel after the phone dropped it.
    #[serde(default)]
    pub reapplied: u32,
}

/// Details that exist only while the backend is up.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningDetails {
    #[serde(flatten)]
    pub head_unit: HeadUnitStatus,
    pub transport: String,
    pub daemon_pid: u32,
    pub dhu_pid: u32,
    #[serde(default)]
    pub tunnels: Vec<TunnelStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    pub session_id: String,
    /// Daemon-side lifecycle. `state` (inside the details) is the backend link state.
    pub phase: Phase,
    #[serde(flatten)]
    pub details: Option<RunningDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusResponse {
    pub daemon: DaemonInfo,
    pub session: Option<SessionStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationResponse {
    pub operation_id: String,
    /// The backend that handled it: `dhu` or `native`. It decides how to read `backendReported`.
    #[serde(default)]
    pub backend: BackendKind,
    #[serde(flatten)]
    pub result: OperationResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotResponse {
    pub operation_id: String,
    #[serde(flatten)]
    pub result: ScreenshotResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoppedResponse {
    pub stopped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub serial: String,
    pub state: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicesResponse {
    pub devices: Vec<DeviceInfo>,
}

/// One event on `GET /v1/events`. `*.requested` events are commands AutoLab issued; `*.accepted`
/// and `*.failed` report what the backend answered. Nothing is ever called "confirmed" or
/// "observed": no phone-side signal exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: u64,
    #[serde(rename = "type")]
    pub kind: String,
    pub timestamp: String,
    pub session_id: Option<String>,
    pub data: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use autolab_core::{FocusState, SessionState, VideoFocusStatus};

    fn head() -> HeadUnitStatus {
        HeadUnitStatus {
            backend: "dhu".into(),
            state: SessionState::Running,
            serial: Some("S".into()),
            backend_version: None,
            backend_build: None,
            certificate_source: None,
            screen_width: None,
            screen_height: None,
            video_focus: VideoFocusStatus {
                requested: FocusState::Off,
                backend_reported: FocusState::Off,
                observed: FocusState::Unknown,
            },
        }
    }

    #[test]
    fn running_session_status_is_flat_and_round_trips() {
        let s = SessionStatus {
            session_id: "8f31c2".into(),
            phase: Phase::Running,
            details: Some(RunningDetails {
                head_unit: head(),
                transport: "usb".into(),
                daemon_pid: 1,
                dhu_pid: 2,
                tunnels: vec![],
            }),
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["sessionId"], "8f31c2");
        assert_eq!(v["phase"], "running");
        assert_eq!(v["backend"], "dhu", "details are flattened, not nested");
        assert_eq!(v["videoFocus"]["requested"], "off");
        let back: SessionStatus = serde_json::from_value(v).unwrap();
        assert_eq!(back.details.unwrap().dhu_pid, 2);
    }

    #[test]
    fn a_starting_session_has_no_details_and_round_trips() {
        let s = SessionStatus {
            session_id: "a".into(),
            phase: Phase::Starting,
            details: None,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"sessionId": "a", "phase": "starting"})
        );
        let back: SessionStatus = serde_json::from_value(v).unwrap();
        assert!(back.details.is_none());
    }

    #[test]
    fn request_bodies_reject_unknown_fields() {
        assert!(serde_json::from_str::<FocusRequest>(r#"{"enabled":true,"extra":1}"#).is_err());
        assert!(serde_json::from_str::<StartRequest>(r#"{"serail":"x"}"#).is_err());
        assert!(serde_json::from_str::<ScreenshotRequest>(r#"{"path":"/a.png"}"#).is_ok());
        assert!(serde_json::from_str::<FocusRequest>(r#"{}"#).is_err());
    }

    #[test]
    fn error_bodies_carry_the_bare_message_and_the_exit_code() {
        let b = ErrorBody::from_error(&Error::Unsupported("nope".into()));
        assert_eq!(
            (b.code.as_str(), b.message.as_str(), b.exit_code),
            ("unsupported", "nope", 5)
        );
    }
}

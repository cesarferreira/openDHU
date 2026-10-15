use serde::{Deserialize, Serialize};

use crate::FocusState;

/// What AutoLab can actually know about the link. Today that is only "the backend process is up
/// and finished its handshake with the phone". It is not a claim that projection is visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoFocusStatus {
    pub requested: FocusState,
    pub backend_reported: FocusState,
    pub observed: FocusState,
}

/// Backend-independent status. Unknown values are explicit (`None` / `FocusState::Unknown`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeadUnitStatus {
    pub backend: String,
    pub state: SessionState,
    pub serial: Option<String>,
    pub backend_version: Option<String>,
    pub backend_build: Option<String>,
    pub video_focus: VideoFocusStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_keys_are_camel_case() {
        let s = HeadUnitStatus {
            backend: "dhu".into(),
            state: SessionState::Running,
            serial: None,
            backend_version: Some("2.0".into()),
            backend_build: None,
            video_focus: VideoFocusStatus {
                requested: FocusState::Off,
                backend_reported: FocusState::Off,
                observed: FocusState::Unknown,
            },
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["state"], "running");
        assert_eq!(v["backendVersion"], "2.0");
        assert_eq!(v["videoFocus"]["backendReported"], "off");
        assert_eq!(v["serial"], serde_json::Value::Null);
    }
}

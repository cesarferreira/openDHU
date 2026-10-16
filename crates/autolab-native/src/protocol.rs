//! The JSON-lines protocol between the daemon (`NativeBackend`) and the `autolab-native-hu` child.
//!
//! The daemon writes one `Request` per line to the child's stdin. The child writes one line per
//! `Response` or `Event` to its stdout; nothing else may go to stdout. The child exits when its stdin
//! closes, which is why a killed daemon never leaves a head unit behind.

use std::path::PathBuf;

use autolab_core::{FocusState, InputResult, OperationResult, ScreenshotResult, VideoFocusStatus};
use serde::{Deserialize, Serialize};

/// The projected screen, in pixels. The head unit declares this as its touch screen and asks the
/// phone for the matching video size, so touches and screenshots share one coordinate system.
pub const SCREEN_WIDTH: u16 = 800;
pub const SCREEN_HEIGHT: u16 = 480;
/// A swipe is dragged in steps of about this many milliseconds.
pub const SWIPE_STEP_MS: u64 = 16;
pub const SWIPE_MIN_MS: u64 = 50;
pub const SWIPE_MAX_MS: u64 = 10_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    Status,
    Focus {
        enabled: bool,
    },
    Screenshot {
        path: PathBuf,
    },
    Tap {
        x: u32,
        y: u32,
    },
    Swipe {
        x1: u32,
        y1: u32,
        x2: u32,
        y2: u32,
        duration_ms: u64,
    },
    Quit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    InvalidArgs,
    Unavailable,
    Timeout,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub kind: ErrorKind,
    pub message: String,
}

/// What `Status` returns. Only what the phone itself did is ever put in `observed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusReport {
    pub video_focus: VideoFocusStatus,
    /// Video frames received from the phone so far.
    pub frames: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Reply {
    Status(StatusReport),
    Focus(OperationResult),
    Screenshot(ScreenshotResult),
    Input(InputResult),
    Done,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok(Reply),
    Err(ErrorInfo),
}

/// Things the child announces on its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// The phone finished the handshake and started projecting.
    Ready {
        backend_version: String,
        backend_build: String,
    },
    /// The phone's own video state changed (it sent a start or stop indication).
    Video { state: FocusState },
    /// The USB link ended. The child exits right after.
    Disconnected { reason: String },
}

/// One line from the child.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Line {
    Response(Response),
    Event(Event),
}

pub fn encode<T: Serialize>(v: &T) -> String {
    let mut s = serde_json::to_string(v).expect("protocol types serialize");
    s.push('\n');
    s
}

pub fn decode_line(line: &str) -> Result<Line, serde_json::Error> {
    serde_json::from_str(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip_and_stay_one_line() {
        for cmd in [
            Command::Status,
            Command::Focus { enabled: false },
            Command::Screenshot {
                path: "/tmp/a b.png".into(),
            },
            Command::Tap { x: 1, y: 2 },
            Command::Swipe {
                x1: 1,
                y1: 2,
                x2: 3,
                y2: 4,
                duration_ms: 300,
            },
            Command::Quit,
        ] {
            let req = Request {
                id: 7,
                command: cmd,
            };
            let text = encode(&req);
            assert_eq!(text.matches('\n').count(), 1, "{text:?}");
            let back: Request = serde_json::from_str(text.trim_end()).unwrap();
            assert_eq!(back, req);
        }
    }

    #[test]
    fn responses_and_events_are_told_apart() {
        let ok = Response {
            id: 3,
            outcome: Outcome::Ok(Reply::Done),
        };
        let err = Response {
            id: 4,
            outcome: Outcome::Err(ErrorInfo {
                kind: ErrorKind::Timeout,
                message: "x".into(),
            }),
        };
        let ev = Event::Video {
            state: FocusState::Off,
        };
        for line in [
            Line::Response(ok),
            Line::Response(err),
            Line::Event(ev),
            Line::Event(Event::Disconnected {
                reason: "unplugged".into(),
            }),
        ] {
            let text = encode(&line);
            assert_eq!(decode_line(text.trim_end()).unwrap(), line, "{text}");
        }
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(decode_line("not json").is_err());
        assert!(decode_line("{\"nothing\":1}").is_err());
    }
}

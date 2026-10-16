//! What the window knows and shows, with no drawing code so it can be tested without a display.

use std::collections::VecDeque;
use std::path::PathBuf;

use autolab_daemon::api::{Phase, StatusResponse};

/// The event log keeps this many lines. Older ones are dropped.
pub const LOG_CAP: usize = 300;

#[derive(Debug, Clone)]
pub enum Conn {
    /// No daemon answers. Nothing is running, or the daemon has not started yet.
    Disconnected,
    Connected(Box<StatusResponse>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub time: String,
    pub kind: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub is_error: bool,
}

/// Something that happened in the background, delivered to the window.
pub enum Update {
    Conn(Conn),
    Event(LogLine),
    /// A message that is not tied to a button press (a touch from the live picture).
    Notice(Notice),
    /// A command finished. `Ok` carries a short success message.
    Done(Result<String, String>),
    Screenshot {
        path: PathBuf,
        width: usize,
        height: usize,
        texture: eframe::egui::TextureHandle,
    },
}

pub struct Shot {
    pub path: PathBuf,
    pub width: usize,
    pub height: usize,
    pub texture: eframe::egui::TextureHandle,
}

pub struct State {
    pub conn: Conn,
    pub log: VecDeque<LogLine>,
    pub notice: Option<Notice>,
    /// Commands still running. Buttons stay disabled while this is above zero.
    pub busy: usize,
    pub shot: Option<Shot>,
    /// The newest frame of the live picture.
    pub live_texture: Option<eframe::egui::TextureHandle>,
    /// Why the live picture is not showing, if it is not.
    pub live_problem: Option<String>,
    /// Frame counters, shown under the picture.
    pub live_summary: String,
    /// Audio counters, shown while audio is on.
    pub audio_summary: Option<String>,
}

/// Which buttons are usable right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Controls {
    pub can_start: bool,
    pub can_stop: bool,
    pub can_focus: bool,
}

impl State {
    pub fn new() -> Self {
        Self {
            conn: Conn::Disconnected,
            log: VecDeque::new(),
            notice: None,
            busy: 0,
            shot: None,
            live_texture: None,
            live_problem: None,
            live_summary: String::new(),
            audio_summary: None,
        }
    }

    pub fn phase(&self) -> Option<Phase> {
        match &self.conn {
            Conn::Connected(s) => s.session.as_ref().map(|s| s.phase),
            Conn::Disconnected => None,
        }
    }

    /// The phone itself has said it stopped projecting (video focus off). Its last picture is old.
    pub fn phone_stopped_projecting(&self) -> bool {
        match &self.conn {
            Conn::Connected(s) => s
                .session
                .as_ref()
                .and_then(|s| s.details.as_ref())
                .is_some_and(|d| d.head_unit.video_focus.observed == autolab_core::FocusState::Off),
            Conn::Disconnected => false,
        }
    }

    /// A native session is up and running: the only kind that has live video and touch.
    pub fn native_running(&self) -> bool {
        match &self.conn {
            Conn::Connected(s) => s.session.as_ref().is_some_and(|s| {
                s.phase == Phase::Running
                    && s.details
                        .as_ref()
                        .is_some_and(|d| d.head_unit.backend == "native")
            }),
            Conn::Disconnected => false,
        }
    }

    /// The projected screen, in pixels: what the running native session reported, or the default
    /// 800x480 before one has (or for the DHU, which never reports one).
    pub fn screen(&self) -> (u16, u16) {
        let reported = match &self.conn {
            Conn::Connected(s) => s
                .session
                .as_ref()
                .and_then(|s| s.details.as_ref())
                .and_then(|d| Some((d.head_unit.screen_width?, d.head_unit.screen_height?))),
            Conn::Disconnected => None,
        };
        reported.unwrap_or((
            autolab_native::protocol::SCREEN_WIDTH,
            autolab_native::protocol::SCREEN_HEIGHT,
        ))
    }

    pub fn controls(&self) -> Controls {
        let idle = self.busy == 0;
        let phase = self.phase();
        Controls {
            can_start: idle && phase.is_none(),
            can_stop: idle && matches!(phase, Some(Phase::Running)),
            can_focus: idle && matches!(phase, Some(Phase::Running)),
        }
    }

    pub fn begin_command(&mut self, what: &str) {
        self.busy += 1;
        self.notice = Some(Notice {
            text: format!("{what}…"),
            is_error: false,
        });
    }

    pub fn apply(&mut self, update: Update) {
        match update {
            Update::Conn(c) => self.conn = c,
            Update::Notice(n) => self.notice = Some(n),
            Update::Event(line) => {
                if self.log.len() == LOG_CAP {
                    self.log.pop_front();
                }
                self.log.push_back(line);
            }
            Update::Done(result) => {
                self.busy = self.busy.saturating_sub(1);
                self.notice = Some(match result {
                    Ok(text) => Notice {
                        text,
                        is_error: false,
                    },
                    Err(text) => Notice {
                        text,
                        is_error: true,
                    },
                });
            }
            Update::Screenshot {
                path,
                width,
                height,
                texture,
            } => {
                self.shot = Some(Shot {
                    path,
                    width,
                    height,
                    texture,
                });
            }
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

/// A server-sent event as one log line: `10:26:30  focus.video.accepted  op-1 requested off`.
pub fn log_line(timestamp: &str, kind: &str, data: &serde_json::Value) -> LogLine {
    // Timestamps are `2026-10-16T09:26:26Z`; show only the clock part.
    let time = timestamp.get(11..19).unwrap_or(timestamp).to_string();
    let mut parts = Vec::new();
    if let Some(op) = data["operationId"].as_str() {
        parts.push(op.to_string());
    }
    for key in ["requested", "backendReported", "reason", "message", "port"] {
        match &data[key] {
            serde_json::Value::String(s) => parts.push(format!("{key} {s}")),
            serde_json::Value::Number(n) => parts.push(format!("{key} {n}")),
            _ => {}
        }
    }
    if let Some(on) = data["enabled"].as_bool() {
        parts.push(format!("enabled {on}"));
    }
    LogLine {
        time,
        kind: kind.to_string(),
        detail: parts.join(" "),
    }
}

/// Parse "10101, 8080" into ports. Empty is fine. A bad entry is an error that names it.
pub fn parse_ports(text: &str) -> Result<Vec<u16>, String> {
    text.split([',', ' '])
        .filter(|t| !t.is_empty())
        .map(|t| match t.parse::<u16>() {
            Ok(p) if p > 0 => Ok(p),
            _ => Err(format!("'{t}' is not a port (1-65535)")),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use autolab_daemon::api::{DaemonInfo, SessionStatus};
    use serde_json::json;

    fn connected(phase: Option<Phase>) -> Conn {
        Conn::Connected(Box::new(StatusResponse {
            daemon: DaemonInfo {
                version: "0".into(),
                pid: 1,
                port: 1,
                exit_when_idle: false,
            },
            session: phase.map(|phase| SessionStatus {
                session_id: "s".into(),
                phase,
                details: None,
            }),
        }))
    }

    #[test]
    fn buttons_follow_the_session_phase() {
        let mut s = State::new();
        let c = s.controls();
        assert!((c.can_start, c.can_stop, c.can_focus) == (true, false, false));
        s.conn = connected(None);
        assert!(s.controls().can_start);
        s.conn = connected(Some(Phase::Starting));
        assert_eq!(
            s.controls(),
            Controls {
                can_start: false,
                can_stop: false,
                can_focus: false
            }
        );
        s.conn = connected(Some(Phase::Running));
        assert_eq!(
            s.controls(),
            Controls {
                can_start: false,
                can_stop: true,
                can_focus: true
            }
        );
        s.conn = connected(Some(Phase::Stopping));
        assert!(!s.controls().can_stop);
    }

    #[test]
    fn buttons_are_disabled_while_a_command_runs_and_back_after() {
        let mut s = State::new();
        s.conn = connected(Some(Phase::Running));
        s.begin_command("Turning video focus off");
        assert!(!s.controls().can_focus && !s.controls().can_stop);
        assert!(!s.notice.as_ref().unwrap().is_error);
        s.apply(Update::Done(Err("boom".into())));
        assert!(s.controls().can_focus);
        assert_eq!(
            s.notice,
            Some(Notice {
                text: "boom".into(),
                is_error: true
            })
        );
    }

    #[test]
    fn done_without_a_running_command_does_not_underflow() {
        let mut s = State::new();
        s.apply(Update::Done(Ok("x".into())));
        assert_eq!(s.busy, 0);
    }

    #[test]
    fn the_log_drops_the_oldest_lines() {
        let mut s = State::new();
        for i in 0..(LOG_CAP + 5) {
            s.apply(Update::Event(LogLine {
                time: String::new(),
                kind: format!("k{i}"),
                detail: String::new(),
            }));
        }
        assert_eq!(s.log.len(), LOG_CAP);
        assert_eq!(s.log.front().unwrap().kind, "k5");
    }

    #[test]
    fn log_lines_show_the_clock_and_the_useful_fields_only() {
        let l = log_line(
            "2026-10-16T09:26:26Z",
            "focus.video.accepted",
            &json!({"operationId":"op-1","requested":"off","backendReported":"off","evidence":[]}),
        );
        assert_eq!(l.time, "09:26:26");
        assert_eq!(l.detail, "op-1 requested off backendReported off");
        let o = log_line("", "stream.opened", &json!({}));
        assert_eq!((o.time.as_str(), o.detail.as_str()), ("", ""));
    }

    #[test]
    fn ports_parse_or_name_the_bad_entry() {
        assert_eq!(parse_ports("10101, 8080 9000"), Ok(vec![10101, 8080, 9000]));
        assert_eq!(parse_ports(""), Ok(vec![]));
        for bad in ["0", "70000", "abc", "1,x"] {
            assert!(parse_ports(bad).is_err(), "{bad}");
        }
        assert!(parse_ports("1,x").unwrap_err().contains("'x'"));
    }
}

//! Parsing and formatting for the DHU console (verified against DHU 2.0, build 2022-03-30).
//!
//! The console prints a `> ` prompt with no trailing newline once it is ready for the next
//! command. A reply is therefore "everything up to the next bare prompt".

use autolab_core::FocusState;

/// Printed once the TLS handshake with the phone has finished. This proves the link to the
/// phone is up. It does not prove that projection is visible.
pub const READY_MARKER: &str = "Verify returned: ok";

pub fn video_focus_command(enabled: bool) -> &'static str {
    if enabled {
        "focus video on"
    } else {
        "focus video off"
    }
}

pub const VIDEO_FOCUS_QUERY: &str = "focus video";

/// Remove ANSI CSI escape sequences such as `\x1b[38;5;196m`.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for n in chars.by_ref() {
                if ('@'..='~').contains(&n) {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// True when the buffer ends at a bare prompt, i.e. the console is ready for a command.
pub fn ends_with_prompt(buf: &str) -> bool {
    let clean = strip_ansi(buf);
    clean
        .rsplit('\n')
        .next()
        .is_some_and(|last| last.trim_end_matches('\r').trim() == ">")
}

/// Split raw console output into clean, non-empty lines. A leading `> ` prompt that shares a
/// line with a message is removed. Bare prompts are dropped.
pub fn clean_lines(raw: &str) -> Vec<String> {
    strip_ansi(raw)
        .replace('\r', "")
        .split('\n')
        .map(|l| l.trim_end())
        .map(|l| l.strip_prefix("> ").unwrap_or(l))
        .filter(|l| !l.trim().is_empty() && l.trim() != ">")
        .map(str::to_string)
        .collect()
}

pub fn error_lines(lines: &[String]) -> Vec<&str> {
    lines
        .iter()
        .map(String::as_str)
        .filter(|l| l.starts_with("[E]:"))
        .collect()
}

/// Parse the reply to `focus video`: `[I]: Has video focus: true`.
pub fn parse_video_focus(lines: &[String]) -> FocusState {
    for l in lines {
        if let Some(v) = l.strip_prefix("[I]: Has video focus:") {
            return match v.trim() {
                "true" => FocusState::On,
                "false" => FocusState::Off,
                _ => FocusState::Unknown,
            };
        }
    }
    FocusState::Unknown
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DhuInfo {
    pub build: Option<String>,
    pub version: Option<String>,
}

/// Read the `Build:` and `Version:` banner lines.
pub fn parse_info(lines: &[String]) -> DhuInfo {
    let mut info = DhuInfo::default();
    for l in lines {
        let t = l.trim();
        if let Some(v) = t.strip_prefix("Build:") {
            info.build.get_or_insert_with(|| v.trim().to_string());
        } else if let Some(v) = t.strip_prefix("Version:") {
            info.version.get_or_insert_with(|| v.trim().to_string());
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    // Strings below were captured from a real DHU 2.0 session against a Pixel 6.

    #[test]
    fn video_focus_commands_use_verified_grammar() {
        assert_eq!(video_focus_command(true), "focus video on");
        assert_eq!(video_focus_command(false), "focus video off");
    }

    #[test]
    fn prompt_detection() {
        assert!(ends_with_prompt("> "));
        assert!(ends_with_prompt("[I]: Has video focus: true\n> "));
        assert!(ends_with_prompt("[I]: Has video focus: true\r\n>"));
        assert!(!ends_with_prompt("> Phone reported protocol version 1.7\n"));
        assert!(!ends_with_prompt("[I]: Has video focus: true\n"));
        assert!(!ends_with_prompt(""));
    }

    #[test]
    fn silent_success_reply_has_no_lines() {
        assert!(clean_lines("> ").is_empty());
    }

    #[test]
    fn parses_focus_readback() {
        let on = clean_lines("[I]: Has video focus: true\n> ");
        let off = clean_lines("[I]: Has video focus: false\n> ");
        assert_eq!(parse_video_focus(&on), FocusState::On);
        assert_eq!(parse_video_focus(&off), FocusState::Off);
        assert_eq!(parse_video_focus(&clean_lines("> ")), FocusState::Unknown);
        assert_eq!(
            parse_video_focus(&clean_lines("[I]: Has video focus: maybe\n> ")),
            FocusState::Unknown
        );
    }

    #[test]
    fn error_reply_is_detected() {
        let l = clean_lines("[E]: Invalid state 'bogus'\n> ");
        assert_eq!(error_lines(&l), vec!["[E]: Invalid state 'bogus'"]);
    }

    #[test]
    fn strips_ansi_from_failure_output() {
        let raw = "[E]: \x1b[38;5;196mfailed.\x1b[0;00m\n";
        assert_eq!(clean_lines(raw), vec!["[E]: failed."]);
    }

    #[test]
    fn parses_banner_and_leading_prompt_lines() {
        let raw = "Android Auto - Desktop Head Unit\n  Build: 2022-03-30-438482292\n  Version: 2.0-mac-arm64\n\n> Phone reported protocol version 1.7\nVerify returned: ok\n";
        let lines = clean_lines(raw);
        assert!(lines.contains(&"Phone reported protocol version 1.7".to_string()));
        assert!(lines.iter().any(|l| l.contains(READY_MARKER)));
        let info = parse_info(&lines);
        assert_eq!(info.build.as_deref(), Some("2022-03-30-438482292"));
        assert_eq!(info.version.as_deref(), Some("2.0-mac-arm64"));
    }
}

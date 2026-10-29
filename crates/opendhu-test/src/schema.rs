//! The YAML test format. Unknown keys, unknown requirements, unit-less or oversized durations,
//! and empty expectations are all rejected up front, so a typo cannot turn into a passing test.

use std::fmt;
use std::time::Duration;

use opendhu_core::{Error, FocusState, HeadUnitStatus, Result, SessionState};
use serde::Deserialize;
use serde::de::{self, Deserializer, Visitor};

/// No wait or timeout may exceed this. Tests must not hang.
pub const MAX_DURATION: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestFile {
    pub name: String,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub requirements: Vec<Requirement>,
    pub steps: Vec<Step>,
    /// Author-written `as:` label per step, parallel to `steps`. Filled in by `parse`.
    #[serde(skip)]
    pub labels: Vec<Option<String>>,
}

impl TestFile {
    /// What step `index` (0-based) is for: the author's `as:` label, else a description derived
    /// from the step's arguments.
    pub fn step_label(&self, index: usize) -> Option<String> {
        let step = self.steps.get(index)?;
        Some(match self.labels.get(index).and_then(Option::as_ref) {
            Some(label) => label.clone(),
            None => step.describe(),
        })
    }
}

/// Capabilities a test needs. Only ones the backend trait can report are accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Requirement {
    VideoFocus,
    Screenshots,
    TouchInput,
    KeyInput,
}

impl Requirement {
    pub fn name(self) -> &'static str {
        match self {
            Self::VideoFocus => "videoFocus",
            Self::Screenshots => "screenshots",
            Self::TouchInput => "touchInput",
            Self::KeyInput => "keyInput",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "action", deny_unknown_fields)]
pub enum Step {
    #[serde(rename = "http.request")]
    HttpRequest {
        method: HttpMethod,
        url: String,
        #[serde(default, deserialize_with = "optional_json")]
        json: Option<serde_json::Value>,
        expect: HttpExpect,
        timeout: Dur,
    },
    #[serde(rename = "app.open")]
    AppOpen,
    #[serde(rename = "app.restart")]
    AppRestart,
    /// Stop the app and delete entries from one of its SharedPreferences files, so a journey can
    /// start from state the app has not cached yet. Other entries and app data are kept.
    #[serde(rename = "app.removePreferences")]
    AppRemovePreferences { file: String, keys: Vec<String> },
    #[serde(rename = "keyboard.type")]
    KeyboardType { text: String, timeout: Dur },
    #[serde(rename = "device.inputText")]
    DeviceInputText { text: String },
    #[serde(rename = "ui.waitFor")]
    UiWaitFor { text: String, timeout: Dur },
    #[serde(rename = "ui.expect")]
    UiExpect { text: String, timeout: Dur },
    /// Wait for text in the phone's own UI tree (not the car screen, which only OCR can read).
    #[serde(rename = "phone.expect")]
    PhoneExpect { text: String, timeout: Dur },
    #[serde(rename = "ui.tap")]
    UiTap {
        text: String,
        timeout: Dur,
        /// Retry visible target taps until this result is visible. Omitted means one tap.
        #[serde(default)]
        until: Option<String>,
    },
    #[serde(rename = "focus.video")]
    FocusVideo { value: OnOff },
    #[serde(rename = "wait")]
    Wait { duration: Dur },
    #[serde(rename = "screenshot")]
    Screenshot { path: String },
    /// Touch a pixel of the screenshot (origin top left). The phone does not acknowledge touch.
    #[serde(rename = "tap")]
    Tap { x: u32, y: u32 },
    /// Drag from one pixel to another. `duration` defaults to 300ms.
    #[serde(rename = "swipe")]
    Swipe {
        from: [u32; 2],
        to: [u32; 2],
        duration: Option<Dur>,
    },
    /// Press a car button: home, back, up, down, left, right, enter, search, play_pause, next, previous,
    /// call, end_call. `long: true` holds it for about a second.
    #[serde(rename = "key")]
    Key {
        value: opendhu_core::Key,
        #[serde(default)]
        long: bool,
    },
    #[serde(rename = "assert.status")]
    AssertStatus { expect: StatusExpect },
    #[serde(rename = "waitFor")]
    WaitFor {
        condition: StatusExpect,
        timeout: Dur,
    },
}

impl Step {
    pub fn action(&self) -> &'static str {
        match self {
            Self::HttpRequest { .. } => "http.request",
            Self::AppOpen => "app.open",
            Self::AppRestart => "app.restart",
            Self::AppRemovePreferences { .. } => "app.removePreferences",
            Self::DeviceInputText { .. } => "device.inputText",
            Self::KeyboardType { .. } => "keyboard.type",
            Self::UiWaitFor { .. } => "ui.waitFor",
            Self::UiExpect { .. } => "ui.expect",
            Self::PhoneExpect { .. } => "phone.expect",
            Self::UiTap { .. } => "ui.tap",
            Self::FocusVideo { .. } => "focus.video",
            Self::Wait { .. } => "wait",
            Self::Screenshot { .. } => "screenshot",
            Self::Tap { .. } => "tap",
            Self::Key { .. } => "key",
            Self::Swipe { .. } => "swipe",
            Self::AssertStatus { .. } => "assert.status",
            Self::WaitFor { .. } => "waitFor",
        }
    }
}

impl Step {
    /// Default human description of the step, used when the test gives no `as:` label.
    pub fn describe(&self) -> String {
        use crate::runner::fmt_dur;
        match self {
            Self::HttpRequest { method, url, .. } => format!("{method:?} {url}"),
            Self::AppOpen => "open the app".into(),
            Self::AppRestart => "restart the app".into(),
            Self::AppRemovePreferences { file, keys } => {
                format!("remove {} from {file}", keys.join(", "))
            }
            Self::KeyboardType { text, .. } | Self::DeviceInputText { text } => {
                format!("type {text:?}")
            }
            Self::UiWaitFor { text, .. } => format!("wait for {text:?}"),
            Self::UiExpect { text, .. } => format!("expect {text:?} visible"),
            Self::PhoneExpect { text, .. } => format!("expect {text:?} on the phone screen"),
            Self::UiTap { text, until, .. } => match until {
                Some(until) => format!("tap {text:?} until {until:?}"),
                None => format!("tap {text:?}"),
            },
            Self::FocusVideo { value } => {
                format!("video focus {}", if value.0 { "on" } else { "off" })
            }
            Self::Wait { duration } => format!("wait {}", fmt_dur(duration.0)),
            Self::Screenshot { path } => format!("screenshot {path}"),
            Self::Tap { x, y } => format!("tap at ({x}, {y})"),
            Self::Swipe { from, to, .. } => {
                format!("swipe ({}, {}) to ({}, {})", from[0], from[1], to[0], to[1])
            }
            Self::Key { value, long } => {
                format!("press {value}{}", if *long { " (long)" } else { "" })
            }
            Self::AssertStatus { .. } => "assert status".into(),
            Self::WaitFor { .. } => "wait for status condition".into(),
        }
    }
}

/// Validate text before passing it through Android's remote input shell command.
pub fn validate_input_text(text: &str) -> Result<()> {
    if text.trim().is_empty()
        || text.len() > 256
        || !text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '/' | '@' | '.' | '-'))
    {
        return Err(Error::InvalidArgs(
            "device.inputText requires 1–256 ASCII letters, digits, spaces or _/@.- characters"
                .into(),
        ));
    }
    Ok(())
}

/// Local fixture API operations. POST is sent once; only GET assertions are polled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize)]
pub enum HttpMethod {
    #[serde(rename = "GET")]
    Get,
    #[serde(rename = "POST")]
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct HttpExpect {
    pub status: u16,
    #[serde(default, deserialize_with = "optional_json")]
    pub json: Option<serde_json::Value>,
}

fn optional_json<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<serde_json::Value>, D::Error> {
    serde_json::Value::deserialize(deserializer).map(Some)
}

/// `on` / `off`. YAML 1.1 readers turn these into booleans, so both spellings are accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnOff(pub bool);

impl OnOff {
    pub fn focus_state(self) -> FocusState {
        FocusState::from_enabled(self.0)
    }
}

impl<'de> Deserialize<'de> for OnOff {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = OnOff;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("`on` or `off` (`unknown` can never be asserted)")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<OnOff, E> {
                Ok(OnOff(v))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<OnOff, E> {
                match v {
                    "on" | "true" => Ok(OnOff(true)),
                    "off" | "false" => Ok(OnOff(false)),
                    other => Err(E::custom(format!(
                        "invalid value `{other}`, expected `on` or `off`"
                    ))),
                }
            }
        }
        d.deserialize_any(V)
    }
}

/// A duration with a mandatory unit: `500ms`, `1s`, `2m`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dur(pub Duration);

pub fn parse_duration(s: &str) -> std::result::Result<Duration, String> {
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: u64 = num.parse().map_err(|_| {
        format!("`{s}` is not a duration; write a number and a unit, e.g. 500ms, 1s, 2m")
    })?;
    match unit {
        "ms" => Ok(Duration::from_millis(n)),
        "s" => Ok(Duration::from_secs(n)),
        "m" => Ok(Duration::from_secs(n.saturating_mul(60))),
        "" => Err(format!("`{s}` has no unit; write e.g. {s}ms, {s}s or {s}m")),
        other => Err(format!(
            "unknown duration unit `{other}` in `{s}`; use ms, s or m"
        )),
    }
}

impl<'de> Deserialize<'de> for Dur {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Dur;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a duration with a unit, e.g. 500ms, 1s, 2m")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Dur, E> {
                parse_duration(v).map(Dur).map_err(E::custom)
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Dur, E> {
                Err(E::custom(format!(
                    "duration `{v}` has no unit; write e.g. {v}ms, {v}s or {v}m"
                )))
            }
        }
        d.deserialize_any(V)
    }
}

/// A partial match against [`HeadUnitStatus`]. Every given field must match exactly.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusExpect {
    #[serde(default)]
    pub state: Option<SessionState>,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub video_focus: Option<VideoFocusExpect>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoFocusExpect {
    #[serde(default)]
    pub requested: Option<OnOff>,
    #[serde(default)]
    pub backend_reported: Option<OnOff>,
    #[serde(default)]
    pub observed: Option<OnOff>,
}

impl StatusExpect {
    fn is_empty(&self) -> bool {
        self.state.is_none()
            && self.backend.is_none()
            && self.video_focus.as_ref().is_none_or(|v| {
                v.requested.is_none() && v.backend_reported.is_none() && v.observed.is_none()
            })
    }

    /// Mismatches, empty when the status satisfies the expectation. `unknown` never matches.
    pub fn check(&self, st: &HeadUnitStatus) -> Vec<String> {
        let mut bad = Vec::new();
        if let Some(want) = self.state
            && want != st.state
        {
            bad.push(format!("state: expected {want:?}, actual {:?}", st.state));
        }
        if let Some(want) = &self.backend
            && *want != st.backend
        {
            bad.push(format!("backend: expected {want}, actual {}", st.backend));
        }
        if let Some(v) = &self.video_focus {
            let mut cmp = |name: &str, want: Option<OnOff>, actual: FocusState| {
                if let Some(w) = want
                    && w.focus_state() != actual
                {
                    bad.push(format!(
                        "videoFocus.{name}: expected {}, actual {}",
                        word(w.focus_state()),
                        word(actual)
                    ));
                }
            };
            cmp("requested", v.requested, st.video_focus.requested);
            cmp(
                "backendReported",
                v.backend_reported,
                st.video_focus.backend_reported,
            );
            cmp("observed", v.observed, st.video_focus.observed);
        }
        bad
    }
}

pub fn word(s: FocusState) -> &'static str {
    match s {
        FocusState::On => "on",
        FocusState::Off => "off",
        FocusState::Unknown => "unknown",
    }
}

/// Parse and validate a test file. Every failure is an `InvalidArgs` error (exit code 2).
pub fn parse(text: &str) -> Result<TestFile> {
    let invalid = |e: serde_norway::Error| Error::InvalidArgs(format!("invalid test file: {e}"));
    let mut value: serde_norway::Value = serde_norway::from_str(text).map_err(invalid)?;
    let labels = take_labels(&mut value)?;
    let mut file: TestFile = serde_norway::from_value(value).map_err(invalid)?;
    file.labels = labels;
    validate(&file)?;
    Ok(file)
}

/// Remove each step's optional `as:` label so the strict step schema never sees it.
fn take_labels(value: &mut serde_norway::Value) -> Result<Vec<Option<String>>> {
    let Some(steps) = value.get_mut("steps").and_then(|s| s.as_sequence_mut()) else {
        return Ok(Vec::new());
    };
    let mut labels = Vec::with_capacity(steps.len());
    for (n, step) in steps.iter_mut().enumerate() {
        let label = match step.as_mapping_mut().and_then(|m| m.remove("as")) {
            None => None,
            Some(serde_norway::Value::String(s)) if !s.trim().is_empty() => Some(s),
            Some(_) => {
                return Err(Error::InvalidArgs(format!(
                    "step {}: `as` must be a non-empty string",
                    n + 1
                )));
            }
        };
        labels.push(label);
    }
    Ok(labels)
}

fn bad(step: usize, action: &str, msg: impl fmt::Display) -> Error {
    Error::InvalidArgs(format!("step {step} ({action}): {msg}"))
}

fn check_duration(i: usize, action: &str, what: &str, d: Dur) -> Result<()> {
    if d.0.is_zero() {
        return Err(bad(i, action, format!("{what} must be greater than zero")));
    }
    if d.0 > MAX_DURATION {
        return Err(bad(
            i,
            action,
            format!("{what} may not exceed {}m", MAX_DURATION.as_secs() / 60),
        ));
    }
    Ok(())
}

fn validate(t: &TestFile) -> Result<()> {
    if t.name.trim().is_empty() {
        return Err(Error::InvalidArgs(
            "invalid test file: `name` is empty".into(),
        ));
    }
    if t.steps.is_empty() {
        return Err(Error::InvalidArgs(
            "invalid test file: a test with no steps would pass without checking anything".into(),
        ));
    }
    if let Some(b) = &t.backend
        && b.trim().is_empty()
    {
        return Err(Error::InvalidArgs(
            "invalid test file: `backend` is empty".into(),
        ));
    }
    for (n, step) in t.steps.iter().enumerate() {
        let i = n + 1;
        let a = step.action();
        match step {
            Step::HttpRequest {
                method,
                url,
                json,
                expect,
                timeout,
            } => {
                check_duration(i, a, "timeout", *timeout)?;
                crate::http::validate(*method, url, json.as_ref(), expect)
                    .map_err(|error| bad(i, a, error.to_string()))?;
            }
            Step::KeyboardType { text, timeout } => {
                check_duration(i, a, "timeout", *timeout)?;
                crate::keyboard::validate_text(text)
                    .map_err(|error| bad(i, a, error.to_string()))?;
            }
            Step::AppOpen | Step::AppRestart => {}
            Step::AppRemovePreferences { file, keys } => {
                let name_ok = |name: &str| {
                    !name.is_empty()
                        && name
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                };
                if keys.is_empty() {
                    return Err(bad(i, a, "`keys` has nothing to remove"));
                }
                if !name_ok(file) || !keys.iter().all(|k| name_ok(k)) {
                    return Err(bad(
                        i,
                        a,
                        "file and keys may only use letters, digits, '.', '_' and '-'",
                    ));
                }
            }
            Step::DeviceInputText { text } => validate_input_text(text)?,
            Step::UiWaitFor { text, timeout }
            | Step::UiExpect { text, timeout }
            | Step::PhoneExpect { text, timeout }
            | Step::UiTap { text, timeout, .. } => {
                check_duration(i, a, "timeout", *timeout)?;
                if let Step::UiTap {
                    until: Some(until), ..
                } = step
                    && until.trim().is_empty()
                {
                    return Err(bad(i, a, "until must not be empty"));
                }
                if text.trim().is_empty() {
                    return Err(bad(i, a, "text must not be empty"));
                }
            }
            Step::FocusVideo { .. } => {}
            Step::Wait { duration } => check_duration(i, a, "duration", *duration)?,
            Step::WaitFor { condition, timeout } => {
                check_duration(i, a, "timeout", *timeout)?;
                if condition.is_empty() {
                    return Err(bad(i, a, "`condition` has nothing to check"));
                }
            }
            Step::AssertStatus { expect } => {
                if expect.is_empty() {
                    return Err(bad(i, a, "`expect` has nothing to check"));
                }
            }
            Step::Screenshot { path } => validate_screenshot_path(i, a, path)?,
            Step::Tap { .. } | Step::Key { .. } => {}
            Step::Swipe { duration, .. } => {
                if let Some(d) = duration {
                    check_duration(i, a, "duration", *d)?;
                    let ms = d.0.as_millis();
                    if !(50..=10_000).contains(&ms) {
                        return Err(bad(i, a, "`duration` must be between 50ms and 10s"));
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_screenshot_path(i: usize, a: &str, path: &str) -> Result<()> {
    if path.starts_with('/') {
        return Err(bad(
            i,
            a,
            "path must be relative to the artifacts directory",
        ));
    }
    if path.split('/').any(|c| c == "..") {
        return Err(bad(i, a, "path may not contain `..`"));
    }
    if path.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(bad(
            i,
            a,
            "path may not contain whitespace (the DHU console cannot take it)",
        ));
    }
    if !path.to_ascii_lowercase().ends_with(".png") {
        return Err(bad(i, a, "path must end in .png"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PRD section 12.1, verbatim.
    const PRD_EXAMPLE: &str = r#"
name: Video focus recovery

backend: dhu

requirements:
  - videoFocus
  - screenshots

steps:
  - action: focus.video
    value: off

  - action: wait
    duration: 1s

  - action: focus.video
    value: on

  - action: screenshot
    path: screenshots/restored.png
"#;

    fn err(text: &str) -> String {
        parse(text).unwrap_err().to_string()
    }

    #[test]
    fn steps_use_their_as_label_or_a_derived_description() {
        let t = parse(
            "name: t\nsteps:\n  - action: ui.tap\n    as: save the cafe\n    text: Save\n    timeout: 1s\n  - action: tap\n    x: 5\n    y: 6\n",
        )
        .unwrap();
        assert_eq!(t.step_label(0).unwrap(), "save the cafe");
        assert_eq!(t.step_label(1).unwrap(), "tap at (5, 6)");
        assert!(t.step_label(2).is_none());
    }

    #[test]
    fn an_empty_or_non_string_label_is_rejected() {
        let bad = "name: t\nsteps:\n  - action: wait\n    as: \"\"\n    duration: 1s\n";
        assert!(err(bad).contains("`as` must be a non-empty string"));
    }

    #[test]
    fn parses_the_prd_example_verbatim() {
        let t = parse(PRD_EXAMPLE).unwrap();
        assert_eq!(t.name, "Video focus recovery");
        assert_eq!(t.backend.as_deref(), Some("dhu"));
        assert_eq!(
            t.requirements,
            vec![Requirement::VideoFocus, Requirement::Screenshots]
        );
        assert_eq!(t.steps.len(), 4);
        assert_eq!(
            t.steps[0],
            Step::FocusVideo {
                value: OnOff(false)
            }
        );
        assert_eq!(
            t.steps[1],
            Step::Wait {
                duration: Dur(Duration::from_secs(1))
            }
        );
        assert_eq!(t.steps[2], Step::FocusVideo { value: OnOff(true) });
        assert_eq!(
            t.steps[3],
            Step::Screenshot {
                path: "screenshots/restored.png".into()
            }
        );
    }

    #[test]
    fn on_and_off_work_as_words_booleans_and_quoted_strings() {
        for (yaml, want) in [
            ("off", false),
            ("on", true),
            ("false", false),
            ("true", true),
            ("\"off\"", false),
            ("\"on\"", true),
        ] {
            let t = parse(&format!(
                "name: t\nsteps:\n  - action: focus.video\n    value: {yaml}\n"
            ))
            .unwrap();
            assert_eq!(
                t.steps[0],
                Step::FocusVideo { value: OnOff(want) },
                "{yaml}"
            );
        }
    }

    #[test]
    fn a_typo_in_a_step_key_is_rejected_not_ignored() {
        let e = err("name: t\nsteps:\n  - action: focus.video\n    vaule: off\n");
        assert!(e.contains("vaule") || e.contains("unknown field"), "{e}");
    }

    #[test]
    fn an_extra_key_next_to_valid_ones_is_rejected_by_deny_unknown_fields() {
        // `value` is present and valid, so only the unknown key can be the cause.
        let e = err("name: t\nsteps:\n  - action: focus.video\n    value: off\n    vaule: on\n");
        assert!(e.contains("unknown field") && e.contains("vaule"), "{e}");
        let e = err("name: t\nsteps:\n  - action: wait\n    duration: 1s\n    extra: 1\n");
        assert!(e.contains("unknown field") && e.contains("extra"), "{e}");
    }

    #[test]
    fn unknown_keys_are_rejected_at_every_level() {
        assert!(
            err("name: t\nstpes: []\nsteps:\n  - action: wait\n    duration: 1s\n")
                .contains("stpes")
        );
        assert!(
            err("name: t\nsteps:\n  - action: assert.status\n    expect:\n      stat: running\n")
                .contains("stat")
        );
        assert!(err("name: t\nsteps:\n  - action: assert.status\n    expect:\n      videoFocus:\n        observd: on\n").contains("observd"));
    }

    #[test]
    fn unknown_action_and_requirement_are_rejected() {
        assert!(err("name: t\nsteps:\n  - action: teleport\n").contains("teleport"));
        let e =
            err("name: t\nrequirements: [nightMode]\nsteps:\n  - action: wait\n    duration: 1s\n");
        assert!(e.contains("nightMode"), "{e}");
    }

    #[test]
    fn empty_tests_and_names_are_rejected() {
        assert!(err("name: t\nsteps: []\n").contains("no steps"));
        assert!(
            err("name: \"  \"\nsteps:\n  - action: wait\n    duration: 1s\n")
                .contains("`name` is empty")
        );
    }

    #[test]
    fn durations_need_units_and_a_sane_range() {
        assert!(err("name: t\nsteps:\n  - action: wait\n    duration: 5\n").contains("no unit"));
        assert!(
            err("name: t\nsteps:\n  - action: wait\n    duration: \"5\"\n").contains("no unit")
        );
        assert!(
            err("name: t\nsteps:\n  - action: wait\n    duration: 5h\n")
                .contains("unknown duration unit")
        );
        assert!(
            err("name: t\nsteps:\n  - action: wait\n    duration: 0s\n")
                .contains("greater than zero")
        );
        assert!(
            err("name: t\nsteps:\n  - action: wait\n    duration: 11m\n")
                .contains("may not exceed")
        );
        assert!(parse("name: t\nsteps:\n  - action: wait\n    duration: 250ms\n").is_ok());
        assert!(parse("name: t\nsteps:\n  - action: wait\n    duration: 10m\n").is_ok());
    }

    #[test]
    fn screenshot_paths_stay_inside_the_artifacts_dir() {
        for (p, why) in [
            ("/etc/x.png", "relative"),
            ("../x.png", ".."),
            ("a/../../x.png", ".."),
            ("a b.png", "whitespace"),
            ("x.jpg", ".png"),
        ] {
            let e = err(&format!(
                "name: t\nsteps:\n  - action: screenshot\n    path: \"{p}\"\n"
            ));
            assert!(e.contains(why), "{p}: {e}");
        }
        assert!(parse("name: t\nsteps:\n  - action: screenshot\n    path: a/b/C.PNG\n").is_ok());
    }

    #[test]
    fn expectations_must_check_something_and_unknown_cannot_be_asserted() {
        assert!(
            err("name: t\nsteps:\n  - action: assert.status\n    expect: {}\n")
                .contains("nothing to check")
        );
        assert!(
            err("name: t\nsteps:\n  - action: assert.status\n    expect:\n      videoFocus: {}\n")
                .contains("nothing to check")
        );
        let e = err(
            "name: t\nsteps:\n  - action: assert.status\n    expect:\n      videoFocus:\n        observed: unknown\n",
        );
        assert!(e.contains("unknown"), "{e}");
        assert!(
            err("name: t\nsteps:\n  - action: waitFor\n    condition: {state: running}\n")
                .contains("timeout")
        );
    }

    fn status(requested: FocusState, reported: FocusState, observed: FocusState) -> HeadUnitStatus {
        HeadUnitStatus {
            backend: "dhu".into(),
            state: SessionState::Running,
            serial: None,
            backend_version: None,
            backend_build: None,
            certificate_source: None,
            screen_width: None,
            screen_height: None,
            video_focus: opendhu_core::VideoFocusStatus {
                requested,
                backend_reported: reported,
                observed,
            },
        }
    }

    #[test]
    fn unknown_never_satisfies_an_expectation() {
        let t = parse("name: t\nsteps:\n  - action: assert.status\n    expect:\n      videoFocus:\n        observed: on\n").unwrap();
        let Step::AssertStatus { expect } = &t.steps[0] else {
            panic!()
        };
        let st = status(FocusState::On, FocusState::On, FocusState::Unknown);
        assert_eq!(
            expect.check(&st),
            vec!["videoFocus.observed: expected on, actual unknown"]
        );
        let st = status(FocusState::On, FocusState::On, FocusState::On);
        assert!(expect.check(&st).is_empty());
    }

    #[test]
    fn partial_expectations_report_every_mismatch() {
        let t = parse("name: t\nsteps:\n  - action: assert.status\n    expect:\n      backend: native\n      videoFocus: {requested: off, backendReported: off}\n").unwrap();
        let Step::AssertStatus { expect } = &t.steps[0] else {
            panic!()
        };
        let st = status(FocusState::On, FocusState::On, FocusState::Unknown);
        let bad = expect.check(&st);
        assert_eq!(bad.len(), 3, "{bad:?}");
    }
}

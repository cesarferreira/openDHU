//! Executes a parsed test against a [`HeadUnitBackend`].
//!
//! Semantics worth knowing:
//! - `focus.video` passes when the backend *accepted* the command and its readback does not
//!   contradict the request. It does not mean the phone confirmed anything.
//! - `unknown` never satisfies an expectation.
//! - Every test ends by restoring video focus if the test may have left it off.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use autolab_core::{Error, FocusState, HeadUnitBackend, HeadUnitStatus, OpStatus, Support};

use crate::model::{Outcome, StepResult, TestResult};
use crate::schema::{self, Requirement, Step, TestFile};

/// Time source, injected so waits and timeouts are testable without real delays.
pub trait Clock {
    /// Monotonic time since an arbitrary start.
    fn now(&self) -> Duration;
    fn sleep(&self, d: Duration);
    /// True once the user asked to stop (Ctrl+C or SIGTERM). Sleeps must then return early.
    fn cancelled(&self) -> bool {
        false
    }
}

pub struct SystemClock {
    start: Instant,
    cancel: Arc<AtomicBool>,
}

impl SystemClock {
    pub fn new() -> Self {
        Self::with_cancel(Arc::new(AtomicBool::new(false)))
    }

    /// A clock whose sleeps end early, and which reports `cancelled`, once `cancel` is set.
    pub fn with_cancel(cancel: Arc<AtomicBool>) -> Self {
        Self {
            start: Instant::now(),
            cancel,
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }
    fn sleep(&self, d: Duration) {
        // Short slices, so an interrupt is noticed within ~100 ms even in a 10 minute wait.
        let end = Instant::now() + d;
        while !self.cancelled() {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            std::thread::sleep(left.min(Duration::from_millis(100)));
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

pub struct RunConfig<'a> {
    pub app_label: Option<&'a str>,
    /// Screenshot paths in tests are relative to this directory.
    pub artifacts_dir: &'a Path,
    /// Skip, rather than fail, a test whose required capabilities the backend lacks.
    pub skip_unsupported: bool,
    pub clock: &'a dyn Clock,
}

const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Exit code for a run stopped by Ctrl+C or SIGTERM (the shell convention for SIGINT).
pub const INTERRUPTED: u8 = 130;

struct End {
    outcome: Outcome,
    detail: String,
    code: u8,
}

impl End {
    fn pass(detail: impl Into<String>) -> Self {
        Self {
            outcome: Outcome::Passed,
            detail: detail.into(),
            code: 0,
        }
    }
    fn assertion(detail: impl Into<String>) -> Self {
        Self {
            outcome: Outcome::Failed,
            detail: detail.into(),
            code: 7,
        }
    }
    fn timeout(detail: impl Into<String>) -> Self {
        Self {
            outcome: Outcome::Failed,
            detail: detail.into(),
            code: 6,
        }
    }
    fn error(e: &Error) -> Self {
        Self {
            outcome: Outcome::Error,
            detail: e.message().to_string(),
            code: e.exit_code(),
        }
    }
    fn interrupted(detail: impl Into<String>) -> Self {
        Self {
            outcome: Outcome::Error,
            detail: detail.into(),
            code: INTERRUPTED,
        }
    }
    fn other(detail: impl Into<String>, code: u8) -> Self {
        Self {
            outcome: Outcome::Error,
            detail: detail.into(),
            code,
        }
    }
}

pub fn fmt_dur(d: Duration) -> String {
    let ms = d.as_millis();
    if ms.is_multiple_of(1000) {
        format!("{}s", ms / 1000)
    } else {
        format!("{ms}ms")
    }
}

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

pub fn run_test(
    test: &TestFile,
    file: &str,
    backend: &mut dyn HeadUnitBackend,
    cfg: &RunConfig,
) -> TestResult {
    let started = cfg.clock.now();
    let mut steps: Vec<StepResult> = Vec::new();
    let mut first_bad: Option<(usize, &'static str, End)> = None;

    // Preflight: backend identity and required capabilities. Always recorded, so reports have a
    // uniform first entry.
    let pre_started = cfg.clock.now();
    let pre = preflight(test, backend, cfg.skip_unsupported);
    let (pre_outcome, pre_detail, pre_code) = match &pre {
        Ok(detail) => (Outcome::Passed, detail.clone(), 0),
        Err(end) => (end.outcome, end.detail.clone(), end.code),
    };
    steps.push(StepResult {
        index: 0,
        action: "preflight".into(),
        outcome: pre_outcome,
        detail: pre_detail,
        duration_ms: ms(cfg.clock.now() - pre_started),
        exit_code: (pre_code != 0).then_some(pre_code),
    });
    match pre {
        Ok(_) => {}
        Err(end) if end.outcome == Outcome::Skipped => {
            return TestResult {
                name: test.name.clone(),
                file: file.into(),
                outcome: Outcome::Skipped,
                message: Some(end.detail),
                steps,
                duration_ms: ms(cfg.clock.now() - started),
                exit_code: 0,
            };
        }
        Err(end) => first_bad = Some((0, "preflight", end)),
    }

    // Steps. The first non-passing step ends the run; the rest are recorded as not run.
    let mut maybe_off = false;
    if first_bad.is_none() {
        for (n, step) in test.steps.iter().enumerate() {
            let index = n + 1;
            if first_bad.is_some() {
                steps.push(StepResult {
                    index,
                    action: step.action().into(),
                    outcome: Outcome::Skipped,
                    detail: "not run: an earlier step did not pass".into(),
                    duration_ms: 0,
                    exit_code: None,
                });
                continue;
            }
            let t0 = cfg.clock.now();
            let end = if cfg.clock.cancelled() {
                End::interrupted("interrupted before this step ran")
            } else {
                if let Step::FocusVideo { value } = step
                    && !value.0
                {
                    maybe_off = true;
                }
                exec(step, backend, cfg)
            };
            if matches!(step, Step::FocusVideo { value } if value.0)
                && end.outcome == Outcome::Passed
            {
                maybe_off = false;
            }
            steps.push(StepResult {
                index,
                action: step.action().into(),
                outcome: end.outcome,
                detail: end.detail.clone(),
                duration_ms: ms(cfg.clock.now() - t0),
                exit_code: (end.code != 0).then_some(end.code),
            });
            if end.outcome != Outcome::Passed {
                first_bad = Some((index, step.action(), end));
            }
        }
    }

    // Isolation: never leave the next test (or the user) with video focus off.
    let mut cleanup_failure: Option<String> = None;
    if maybe_off {
        let t0 = cfg.clock.now();
        let end = restore_focus(backend);
        steps.push(StepResult {
            index: test.steps.len() + 1,
            action: "cleanup.focus.video".into(),
            outcome: end.outcome,
            detail: end.detail.clone(),
            duration_ms: ms(cfg.clock.now() - t0),
            exit_code: (end.code != 0).then_some(end.code),
        });
        if end.outcome != Outcome::Passed {
            cleanup_failure = Some(end.detail.clone());
            if first_bad.is_none() {
                first_bad = Some((steps.len(), "cleanup.focus.video", end));
            }
        }
    }

    let (outcome, message, exit_code) = match first_bad {
        None => (Outcome::Passed, None, 0),
        Some((index, action, end)) => {
            let mut msg = format!("step {index} ({action}): {}", end.detail);
            if let Some(c) = &cleanup_failure
                && action != "cleanup.focus.video"
            {
                msg.push_str(&format!("; cleanup also failed: {c}"));
            }
            (end.outcome, Some(msg), end.code)
        }
    };
    TestResult {
        name: test.name.clone(),
        file: file.into(),
        outcome,
        message,
        steps,
        duration_ms: ms(cfg.clock.now() - started),
        exit_code,
    }
}

fn preflight(
    test: &TestFile,
    backend: &mut dyn HeadUnitBackend,
    skip_unsupported: bool,
) -> std::result::Result<String, End> {
    let status = backend.status().map_err(|e| End::error(&e))?;
    if let Some(want) = &test.backend
        && *want != status.backend
    {
        return Err(End::other(
            format!(
                "test is written for backend `{want}` but the session uses `{}`",
                status.backend
            ),
            2,
        ));
    }
    let caps = backend.capabilities().map_err(|e| End::error(&e))?;
    let missing: Vec<&str> = test
        .requirements
        .iter()
        .filter(|r| {
            let have = match r {
                Requirement::VideoFocus => caps.video_focus,
                Requirement::Screenshots => caps.screenshots,
                Requirement::TouchInput => caps.touch_input,
                Requirement::KeyInput => caps.key_input,
            };
            have != Support::Supported
        })
        .map(|r| r.name())
        .collect();
    if missing.is_empty() {
        return Ok(if test.requirements.is_empty() {
            format!("backend `{}` ready", status.backend)
        } else {
            let names: Vec<&str> = test.requirements.iter().map(|r| r.name()).collect();
            format!(
                "backend `{}` ready; requirements met: {}",
                status.backend,
                names.join(", ")
            )
        });
    }
    let detail = format!(
        "backend `{}` does not support: {}",
        caps.backend,
        missing.join(", ")
    );
    if skip_unsupported {
        Err(End {
            outcome: Outcome::Skipped,
            detail: format!("skipped: {detail}"),
            code: 0,
        })
    } else {
        Err(End::other(detail, 5))
    }
}

fn restore_focus(backend: &mut dyn HeadUnitBackend) -> End {
    match backend.set_video_focus(true) {
        Err(e) => End::other(
            format!("could not restore video focus: {}", e.message()),
            e.exit_code(),
        ),
        Ok(r) if r.status.is_success() && r.backend_reported != FocusState::Off => {
            End::pass("video focus restored to on")
        }
        Ok(r) => End::other(
            format!(
                "could not restore video focus ({:?}; backend reports {})",
                r.status,
                schema::word(r.backend_reported)
            ),
            1,
        ),
    }
}

fn exec(step: &Step, backend: &mut dyn HeadUnitBackend, cfg: &RunConfig) -> End {
    match step {
        Step::AppOpen => open_app(backend, cfg),
        Step::UiWaitFor { text, timeout } | Step::UiExpect { text, timeout } => {
            screen_text(text, timeout.0, false, false, None, backend, cfg)
        }
        Step::UiTap {
            text,
            timeout,
            until,
        } => screen_text(text, timeout.0, true, false, until.as_deref(), backend, cfg),
        Step::FocusVideo { value } => focus_video(value.focus_state(), backend),
        Step::Wait { duration } => {
            cfg.clock.sleep(duration.0);
            if cfg.clock.cancelled() {
                return End::interrupted(format!(
                    "interrupted during a {} wait",
                    fmt_dur(duration.0)
                ));
            }
            End::pass(format!("waited {}", fmt_dur(duration.0)))
        }
        Step::Screenshot { path } => screenshot(path, backend, cfg.artifacts_dir),
        Step::Key { value, long } => match backend.press_key(*value, *long) {
            Ok(r) => End::pass(format!(
                "{}{} pressed; {}",
                if *long { "long press of " } else { "" },
                value.name(),
                touch_note(&r.status)
            )),
            Err(e) => End::error(&e),
        },
        Step::Tap { x, y } => match backend.tap(*x, *y) {
            Ok(r) => End::pass(format!("tap at ({x}, {y}) sent; {}", touch_note(&r.status))),
            Err(e) => End::error(&e),
        },
        Step::Swipe { from, to, duration } => {
            let d = duration.map_or(Duration::from_millis(300), |d| d.0);
            match backend.swipe((from[0], from[1]), (to[0], to[1]), d) {
                Ok(r) => End::pass(format!(
                    "swipe ({}, {}) to ({}, {}) over {} sent; {}",
                    from[0],
                    from[1],
                    to[0],
                    to[1],
                    fmt_dur(d),
                    touch_note(&r.status)
                )),
                Err(e) => End::error(&e),
            }
        }
        Step::AssertStatus { expect } => match backend.status() {
            Err(e) => End::error(&e),
            Ok(st) => {
                let bad = expect.check(&st);
                if bad.is_empty() {
                    End::pass("status matches the expectation")
                } else {
                    End::assertion(bad.join("; "))
                }
            }
        },
        Step::WaitFor { condition, timeout } => {
            let deadline = cfg.clock.now() + timeout.0;
            let began = cfg.clock.now();
            loop {
                let st: HeadUnitStatus = match backend.status() {
                    Ok(st) => st,
                    Err(e) => return End::error(&e),
                };
                let bad = condition.check(&st);
                if bad.is_empty() {
                    return End::pass(format!(
                        "condition met after {}",
                        fmt_dur(cfg.clock.now() - began)
                    ));
                }
                let now = cfg.clock.now();
                if now >= deadline {
                    return End::timeout(format!(
                        "condition not met within {}: {}",
                        fmt_dur(timeout.0),
                        bad.join("; ")
                    ));
                }
                if cfg.clock.cancelled() {
                    return End::interrupted("interrupted while waiting for a condition");
                }
                cfg.clock.sleep(POLL_INTERVAL.min(deadline - now));
            }
        }
    }
}

/// Touch is never acknowledged by the phone, so a pass only means it was sent.
fn touch_note(_status: &OpStatus) -> &'static str {
    "not acknowledged by the phone (touch never is); assert on what changed"
}

fn focus_video(requested: FocusState, backend: &mut dyn HeadUnitBackend) -> End {
    let word = schema::word(requested);
    let r = match backend.set_video_focus(requested == FocusState::On) {
        Ok(r) => r,
        Err(e) => return End::error(&e),
    };
    match r.status {
        OpStatus::Failed => End::other(
            format!(
                "backend rejected `focus video {word}`: {}",
                r.evidence.join(" | ")
            ),
            1,
        ),
        OpStatus::Unsupported => End::other(
            format!("`focus video {word}` is unsupported by this backend"),
            5,
        ),
        OpStatus::TimedOut => End::other(format!("`focus video {word}` timed out"), 6),
        OpStatus::Accepted | OpStatus::Acknowledged | OpStatus::Observed => {
            if r.backend_reported != FocusState::Unknown && r.backend_reported != requested {
                return End::assertion(format!(
                    "requested video focus {word} but the backend reports {}",
                    schema::word(r.backend_reported)
                ));
            }
            let readback = match r.backend_reported {
                FocusState::Unknown => "backend readback unavailable".to_string(),
                s => format!("backend reports {}", schema::word(s)),
            };
            let phone = match r.observed {
                FocusState::Unknown => "not confirmed by the phone".to_string(),
                s => format!("phone confirms {}", schema::word(s)),
            };
            End::pass(format!(
                "video focus {word}: command accepted; {readback}; {phone}"
            ))
        }
    }
}

fn screenshot(rel: &str, backend: &mut dyn HeadUnitBackend, artifacts: &Path) -> End {
    let dest = artifacts.join(rel);
    if let Some(parent) = dest.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        return End::other(format!("cannot create {}: {e}", parent.display()), 1);
    }
    match backend.screenshot(&dest) {
        Ok(r) => End::pass(format!("saved {rel} ({}x{})", r.width, r.height)),
        Err(e) => End::error(&e),
    }
}

static SCREEN_ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn screen_text(
    text: &str,
    timeout: Duration,
    tap: bool,
    launcher: bool,
    until: Option<&str>,
    backend: &mut dyn HeadUnitBackend,
    cfg: &RunConfig,
) -> End {
    let id = SCREEN_ID.fetch_add(1, Ordering::Relaxed);
    let dir = cfg.artifacts_dir.join("ui");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return End::error(&Error::Other(e.to_string()));
    }
    let image = dir.join(format!("screen-{id:04}.png"));
    let deadline = cfg.clock.now() + timeout;
    let mut last = String::new();
    loop {
        if cfg.clock.cancelled() {
            return End::interrupted("interrupted while observing projected UI");
        }
        if let Err(e) = backend.screenshot(&image) {
            return End::error(&e);
        }
        let words = match crate::screen::recognize(&image, || cfg.clock.cancelled()) {
            Ok(words) => words,
            Err(e) => return End::error(&e),
        };
        // Completion wins over another tap, including when the desired screen was already open.
        if let Some(result) = until
            && crate::screen::locate(&words, result).is_some()
        {
            return End::pass(format!(
                "projected result {result:?} visible after tapping {text:?}; {}",
                image.display()
            ));
        }
        let found = if launcher {
            crate::screen::locate_in_region(&words, text, 70, 410)
        } else {
            crate::screen::locate(&words, text)
        };
        if let Some((x, y)) = found {
            if tap {
                // Never tap after the observation deadline; a final screenshot can still prove
                // completion, but should not send another action that cannot be observed.
                if until.is_none() || cfg.clock.now() < deadline {
                    if let Err(e) = backend.tap(x, y) {
                        return End::error(&e);
                    }
                    if until.is_none() {
                        return End::pass(format!(
                            "tapped projected text {text:?} at ({x}, {y}); assert on the result"
                        ));
                    }
                }
            }
            if !tap {
                return End::pass(format!(
                    "projected text {text:?} visible; {}",
                    image.display()
                ));
            }
        }
        let seen = crate::screen::text(&words);
        if seen != last {
            last = seen;
        }
        let now = cfg.clock.now();
        if now >= deadline {
            return End::timeout(format!(
                "{}; last OCR: {last}; screenshot: {}",
                until.map_or_else(
                    || format!("projected text {text:?} not visible"),
                    |result| format!(
                        "projected result {result:?} not visible after tapping {text:?}"
                    ),
                ),
                image.display()
            ));
        }
        let pause = if until.is_some() { 500 } else { 250 };
        cfg.clock
            .sleep(Duration::from_millis(pause).min(deadline - now));
    }
}

fn open_app(backend: &mut dyn HeadUnitBackend, cfg: &RunConfig) -> End {
    let Some(label) = cfg.app_label else {
        return End::error(&Error::InvalidArgs(
            "app.open requires a suite environment.appLabel".into(),
        ));
    };
    // Host launcher position is part of the currently supported native 800x480 profile.
    // From an app the first tap opens the dashboard; a second opens its app grid.
    for _ in 0..3 {
        if cfg.clock.cancelled() {
            return End::interrupted("interrupted while opening app");
        }
        if let Err(e) = backend.tap(770, 452) {
            return End::error(&e);
        }
        cfg.clock.sleep(Duration::from_secs(1));
        // Launcher labels may be ellipsized. A stable prefix is sufficient; app content is asserted next.
        let prefix: String = label.chars().take(12).collect();
        let result = screen_text(
            &prefix,
            Duration::from_secs(2),
            true,
            true,
            None,
            backend,
            cfg,
        );
        if result.outcome == Outcome::Passed {
            return result;
        }
        if result.outcome == Outcome::Error {
            return result;
        }
    }
    End::timeout(format!(
        "app {label:?} was not visible in the Android Auto launcher"
    ))
}

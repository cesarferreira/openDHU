//! Runner semantics against a mock backend and a fake clock (no hardware, no real waiting).

use std::path::PathBuf;
use std::time::Duration;

use autolab_core::{FocusState, Support};
use autolab_test::mock::{FakeClock, MockBackend};
use autolab_test::runner::{Clock, RunConfig, run_test};
use autolab_test::{Outcome, TestResult, parse};

fn artifacts(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("al-runner-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(
    yaml: &str,
    backend: &mut MockBackend,
    tag: &str,
    skip: bool,
) -> (TestResult, FakeClock, PathBuf) {
    let dir = artifacts(tag);
    let clock = FakeClock::default();
    let test = parse(yaml).unwrap();
    let r = run_test(
        &test,
        "t.yaml",
        backend,
        &RunConfig {
            artifacts_dir: &dir,
            skip_unsupported: skip,
            clock: &clock,
        },
    );
    (r, clock, dir)
}

const PRD: &str = "name: Video focus recovery\nbackend: dhu\nrequirements: [videoFocus, screenshots]\nsteps:\n  - action: focus.video\n    value: off\n  - action: wait\n    duration: 1s\n  - action: focus.video\n    value: on\n  - action: screenshot\n    path: screenshots/restored.png\n";

#[test]
fn prd_example_passes_without_real_waiting_and_writes_the_screenshot() {
    let mut b = MockBackend::new();
    let started = std::time::Instant::now();
    let (r, clock, dir) = run(PRD, &mut b, "prd", false);
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "fake clock must not really sleep"
    );
    assert_eq!(r.outcome, Outcome::Passed, "{:?}", r.message);
    assert_eq!(r.exit_code, 0);
    assert_eq!(clock.now(), Duration::from_secs(1));
    assert!(dir.join("screenshots/restored.png").is_file());
    // preflight + 4 steps, and no cleanup because the test ended with focus on
    assert_eq!(r.steps.len(), 5);
    assert_eq!(r.steps[0].action, "preflight");
    assert!(r.steps.iter().all(|s| s.outcome == Outcome::Passed));
    assert!(
        r.steps[1].detail.contains("not confirmed by the phone"),
        "{}",
        r.steps[1].detail
    );
}

#[test]
fn a_failed_step_stops_the_run_and_marks_the_rest_not_run() {
    let mut b = MockBackend::new();
    b.screenshot_error = Some((1, "boom".into()));
    let yaml = "name: t\nsteps:\n  - action: screenshot\n    path: a.png\n  - action: wait\n    duration: 1s\n";
    let (r, clock, _) = run(yaml, &mut b, "stop", false);
    assert_eq!(r.outcome, Outcome::Error);
    assert_eq!(r.steps[2].outcome, Outcome::Skipped);
    assert!(r.steps[2].detail.contains("not run"));
    assert_eq!(
        clock.now(),
        Duration::ZERO,
        "the wait after the failure must not run"
    );
}

#[test]
fn video_focus_is_restored_after_a_failing_test() {
    let mut b = MockBackend::new();
    b.screenshot_error = Some((1, "boom".into()));
    let yaml = "name: t\nsteps:\n  - action: focus.video\n    value: off\n  - action: screenshot\n    path: a.png\n";
    let (r, _, _) = run(yaml, &mut b, "restore", false);
    assert_eq!(r.outcome, Outcome::Error);
    assert_eq!(b.calls.last().unwrap(), "focus.video on", "{:?}", b.calls);
    let cleanup = r.steps.last().unwrap();
    assert_eq!(cleanup.action, "cleanup.focus.video");
    assert_eq!(
        cleanup.index, 3,
        "numbered after the test's own 2 steps, not counting preflight"
    );
    assert_eq!(cleanup.outcome, Outcome::Passed);
}

#[test]
fn a_test_that_ends_with_focus_off_is_cleaned_up_and_still_passes() {
    let mut b = MockBackend::new();
    let (r, _, _) = run(
        "name: t\nsteps:\n  - action: focus.video\n    value: off\n",
        &mut b,
        "endoff",
        false,
    );
    assert_eq!(r.outcome, Outcome::Passed);
    assert_eq!(b.calls.last().unwrap(), "focus.video on");
}

#[test]
fn failed_cleanup_fails_an_otherwise_passing_test() {
    let mut b = MockBackend::new();
    b.focus_error_when = Some((true, 3, "link lost".into()));
    let (r, _, _) = run(
        "name: t\nsteps:\n  - action: focus.video\n    value: off\n",
        &mut b,
        "cleanfail",
        false,
    );
    assert_eq!(r.outcome, Outcome::Error);
    assert_eq!(r.exit_code, 3);
    assert!(r.message.unwrap().contains("could not restore video focus"));
}

#[test]
fn readback_that_contradicts_the_request_fails_the_step() {
    let mut b = MockBackend::new();
    b.readback_override = Some(FocusState::On);
    let (r, _, _) = run(
        "name: t\nsteps:\n  - action: focus.video\n    value: off\n",
        &mut b,
        "contra",
        false,
    );
    assert_eq!(r.outcome, Outcome::Failed);
    assert_eq!(r.exit_code, 7);
    assert!(
        r.steps[1].detail.contains("backend reports on"),
        "{}",
        r.steps[1].detail
    );
}

#[test]
fn unavailable_readback_passes_but_says_so() {
    let mut b = MockBackend::new();
    b.readback_override = Some(FocusState::Unknown);
    let (r, _, _) = run(
        "name: t\nsteps:\n  - action: focus.video\n    value: on\n",
        &mut b,
        "unkread",
        false,
    );
    assert_eq!(r.outcome, Outcome::Passed);
    assert!(r.steps[1].detail.contains("readback unavailable"));
}

#[test]
fn a_backend_that_rejects_the_command_is_an_error_not_a_pass() {
    let mut b = MockBackend::new();
    b.reject_focus = true;
    let (r, _, _) = run(
        "name: t\nsteps:\n  - action: focus.video\n    value: on\n",
        &mut b,
        "reject",
        false,
    );
    assert_eq!(r.outcome, Outcome::Error);
    assert_eq!(r.exit_code, 1);
}

#[test]
fn assert_status_cannot_pass_on_unknown() {
    let mut b = MockBackend::new();
    let yaml = "name: t\nsteps:\n  - action: assert.status\n    expect:\n      videoFocus:\n        observed: on\n";
    let (r, _, _) = run(yaml, &mut b, "unknown", false);
    assert_eq!(r.outcome, Outcome::Failed);
    assert_eq!(r.exit_code, 7);
    assert!(r.steps[1].detail.contains("expected on, actual unknown"));
}

#[test]
fn assert_status_passes_on_a_real_match() {
    let mut b = MockBackend::new();
    let yaml = "name: t\nsteps:\n  - action: focus.video\n    value: off\n  - action: assert.status\n    expect:\n      state: running\n      backend: dhu\n      videoFocus: {requested: off, backendReported: off}\n  - action: focus.video\n    value: on\n";
    let (r, _, _) = run(yaml, &mut b, "match", false);
    assert_eq!(r.outcome, Outcome::Passed, "{:?}", r.message);
}

#[test]
fn wait_for_succeeds_when_the_condition_becomes_true() {
    let mut b = MockBackend::new();
    b.observed_on_after_status_calls = Some(4); // preflight is call 1
    let yaml = "name: t\nsteps:\n  - action: waitFor\n    condition:\n      videoFocus: {observed: on}\n    timeout: 5s\n";
    let (r, clock, _) = run(yaml, &mut b, "waitok", false);
    assert_eq!(r.outcome, Outcome::Passed, "{:?}", r.message);
    assert_eq!(
        clock.now(),
        Duration::from_millis(500),
        "two polls of 250ms"
    );
}

#[test]
fn wait_for_times_out_deterministically_with_exit_code_6() {
    let mut b = MockBackend::new();
    let yaml = "name: t\nsteps:\n  - action: waitFor\n    condition:\n      videoFocus: {observed: on}\n    timeout: 1s\n";
    let started = std::time::Instant::now();
    let (r, clock, _) = run(yaml, &mut b, "waittimeout", false);
    assert!(started.elapsed() < Duration::from_millis(500));
    assert_eq!(r.outcome, Outcome::Failed);
    assert_eq!(r.exit_code, 6);
    assert_eq!(
        clock.now(),
        Duration::from_secs(1),
        "polling stops exactly at the deadline"
    );
    assert!(r.steps[1].detail.contains("not met within 1s"));
}

#[test]
fn unsupported_requirement_fails_with_5_or_is_skipped_by_configuration() {
    let yaml = "name: t\nrequirements: [screenshots]\nsteps:\n  - action: wait\n    duration: 1s\n";
    let mut b = MockBackend::new();
    b.caps.screenshots = Support::Unsupported;
    let (r, _, _) = run(yaml, &mut b, "unsup", false);
    assert_eq!((r.outcome, r.exit_code), (Outcome::Error, 5));
    assert!(r.message.unwrap().contains("screenshots"));

    let mut b = MockBackend::new();
    b.caps.screenshots = Support::Unverified;
    let (r, clock, _) = run(yaml, &mut b, "skip", true);
    assert_eq!((r.outcome, r.exit_code), (Outcome::Skipped, 0));
    assert_eq!(clock.now(), Duration::ZERO, "a skipped test runs no steps");
    assert!(!b.calls.iter().any(|c| c.starts_with("focus.video")));
}

#[test]
fn backend_mismatch_and_a_down_backend_are_errors() {
    let mut b = MockBackend::new();
    let (r, _, _) = run(
        "name: t\nbackend: native\nsteps:\n  - action: wait\n    duration: 1s\n",
        &mut b,
        "mismatch",
        false,
    );
    assert_eq!((r.outcome, r.exit_code), (Outcome::Error, 2));

    let mut b = MockBackend::new();
    b.status_error = Some((3, "no AutoLab session is running".into()));
    let (r, _, _) = run(
        "name: t\nsteps:\n  - action: wait\n    duration: 1s\n",
        &mut b,
        "down",
        false,
    );
    assert_eq!((r.outcome, r.exit_code), (Outcome::Error, 3));
}

//! Process-lifecycle and console round-trip tests against a scripted fake DHU.
//! These prove AutoLab's handling of the console contract. They do not prove the real DHU:
//! see `hardware.rs` for that.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use autolab_core::{Error, FocusState, HeadUnitBackend, OpStatus};
use autolab_dhu::{DhuBackend, DhuProcess};

fn fake() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/fake-dhu.sh"
    ))
}

fn spawn(mode: &str) -> DhuProcess {
    DhuProcess::spawn::<&str>(&fake(), &[], &[("FAKE_DHU_MODE", mode)]).unwrap()
}

fn backend() -> DhuBackend {
    DhuBackend::from_process(spawn("normal"), Duration::from_secs(10)).unwrap()
}

fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .unwrap()
        .status
        .success()
}

#[test]
fn ready_reports_dhu_version_info() {
    let b = backend();
    assert_eq!(b.info().version.as_deref(), Some("0.0-fake"));
    assert_eq!(b.info().build.as_deref(), Some("fake-build-1"));
}

#[test]
fn video_focus_off_then_on_is_accepted_but_never_observed() {
    let mut b = backend();
    let off = b.set_video_focus(false).unwrap();
    assert_eq!(off.status, OpStatus::Accepted);
    assert_eq!(off.requested, FocusState::Off);
    assert_eq!(off.backend_reported, FocusState::Off);
    assert_eq!(off.observed, FocusState::Unknown);
    assert_eq!(b.requested_video(), FocusState::Off);

    let on = b.set_video_focus(true).unwrap();
    assert_eq!(on.backend_reported, FocusState::On);
    assert_eq!(on.observed, FocusState::Unknown);
}

#[test]
fn repeated_calls_are_stable() {
    let mut b = backend();
    for _ in 0..5 {
        assert_eq!(
            b.set_video_focus(false).unwrap().backend_reported,
            FocusState::Off
        );
        assert_eq!(
            b.set_video_focus(true).unwrap().backend_reported,
            FocusState::On
        );
    }
}

#[test]
fn console_error_reply_is_a_failure_not_success() {
    let mut p = spawn("normal");
    p.wait_ready(Duration::from_secs(10)).unwrap();
    let lines = p
        .round_trip("focus video bogus", Duration::from_secs(5))
        .unwrap();
    assert_eq!(lines, vec!["[E]: Invalid state 'bogus'"]);
}

#[test]
fn newline_in_command_is_rejected_before_reaching_the_console() {
    let mut p = spawn("normal");
    p.wait_ready(Duration::from_secs(10)).unwrap();
    let err = p
        .round_trip("focus video on\nquit", Duration::from_secs(5))
        .unwrap_err();
    assert!(matches!(err, Error::InvalidArgs(_)));
    assert!(p.exit_status().is_none(), "injected quit must not have run");
}

#[test]
fn crash_is_detected_and_later_commands_fail_fast() {
    let mut p = spawn("normal");
    p.wait_ready(Duration::from_secs(10)).unwrap();
    let err = p.round_trip("crash", Duration::from_secs(5)).unwrap_err();
    assert!(matches!(err, Error::BackendUnavailable(_)), "{err}");
    assert!(err.to_string().contains("exit status: 3"), "{err}");
    let started = Instant::now();
    let again = p
        .round_trip("focus video", Duration::from_secs(5))
        .unwrap_err();
    assert!(matches!(again, Error::BackendUnavailable(_)));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn startup_failure_surfaces_cleaned_dhu_output() {
    let mut p = spawn("fail-start");
    let err = p.wait_ready(Duration::from_secs(10)).unwrap_err();
    assert!(matches!(err, Error::BackendUnavailable(_)));
    let msg = err.to_string();
    assert!(
        msg.contains("Failed to start Google Automotive Link."),
        "{msg}"
    );
    assert!(
        !msg.contains('\x1b'),
        "ANSI escapes must be stripped: {msg:?}"
    );
}

#[test]
fn replies_on_stderr_are_never_lost_to_a_prompt_on_stdout() {
    // Regression: the real DHU prints replies on stderr and the prompt on stdout. Reading them
    // from two pipes let the prompt win the race and the reply was dropped as stale output.
    let mut p = spawn("normal");
    p.wait_ready(Duration::from_secs(10)).unwrap();
    for i in 0..300 {
        let lines = p.round_trip("focus video", Duration::from_secs(5)).unwrap();
        assert_eq!(lines, vec!["[I]: Has video focus: true"], "iteration {i}");
    }
}

#[test]
fn early_stderr_close_does_not_hide_later_stdout_output() {
    // Regression: closing one output stream early must not be taken as "DHU exited".
    let mut p = spawn("late-fail-start");
    let err = p.wait_ready(Duration::from_secs(10)).unwrap_err();
    assert!(
        err.to_string().contains("(late)"),
        "stdout output was lost: {err}"
    );
}

#[test]
fn startup_hang_times_out_and_child_is_killed() {
    let mut p = spawn("hang-start");
    let pid = p.id();
    let started = Instant::now();
    let err = p.wait_ready(Duration::from_millis(600)).unwrap_err();
    assert!(matches!(err, Error::Timeout(_)), "{err}");
    assert!(started.elapsed() < Duration::from_secs(3));
    drop(p);
    assert!(!alive(pid), "child must not survive drop");
}

#[test]
fn command_hang_times_out_instead_of_blocking() {
    let mut p = spawn("normal");
    p.wait_ready(Duration::from_secs(10)).unwrap();
    let pid = p.id();
    let started = Instant::now();
    let err = p
        .round_trip("hang", Duration::from_millis(500))
        .unwrap_err();
    assert!(matches!(err, Error::Timeout(_)), "{err}");
    assert!(started.elapsed() < Duration::from_secs(3));
    drop(p);
    assert!(!alive(pid));
}

#[test]
fn stop_is_graceful_and_leaves_no_process() {
    let mut b = backend();
    let pid = b.pid();
    b.stop().unwrap();
    assert!(!alive(pid));
    assert!(b.exit_status().is_some());
}

#[test]
fn dropping_the_backend_does_not_orphan_the_dhu() {
    let b = backend();
    let pid = b.pid();
    assert!(alive(pid));
    drop(b);
    assert!(!alive(pid));
}

#[test]
fn spawn_of_missing_binary_is_backend_unavailable() {
    let err = DhuProcess::spawn::<&str>(std::path::Path::new("/definitely/not/here"), &[], &[])
        .err()
        .unwrap();
    assert!(matches!(err, Error::BackendUnavailable(_)));
}

//! `NativeBackend` against a scripted fake head unit. Real-phone behaviour is covered by the
//! hardware test in autolab-cli.

use std::path::PathBuf;
use std::time::Duration;

use autolab_core::{Error, FocusState, HeadUnitBackend, OpStatus, Support};
use autolab_native::{NativeBackend, NativeLaunch};

fn fake() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/fake-native-hu.sh"
    ))
}

fn launch(mode: &str, ready: Duration) -> Result<NativeBackend, Error> {
    // The mode reaches the child through the environment it inherits.
    // SAFETY: each test sets its own variable name before spawning and nothing else reads it.
    let launch = NativeLaunch {
        program: fake(),
        serial: Some("FAKE0001".into()),
    };
    unsafe { std::env::set_var("FAKE_NATIVE_MODE", mode) };
    let r = NativeBackend::launch_with_ready_timeout(&launch, &[] as &[&str], ready);
    unsafe { std::env::remove_var("FAKE_NATIVE_MODE") };
    r
}

/// The tests change one process-wide variable, so they take turns.
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn start(mode: &str) -> NativeBackend {
    launch(mode, Duration::from_secs(10)).expect("fake head unit starts")
}

#[test]
fn status_focus_and_screenshot_follow_the_phone_not_the_request() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut b = start("");
    let st = b.status().unwrap();
    assert_eq!(st.backend, "native");
    assert_eq!(st.backend_version.as_deref(), Some("fake 0.0"));
    assert_eq!(st.serial.as_deref(), Some("FAKE0001"));
    assert_eq!(st.video_focus.observed, FocusState::On);

    let r = b.set_video_focus(false).unwrap();
    assert_eq!(r.status, OpStatus::Observed);
    assert_eq!(
        (r.requested, r.observed),
        (FocusState::Off, FocusState::Off)
    );
    assert_eq!(b.status().unwrap().video_focus.observed, FocusState::Off);

    let dir = std::env::temp_dir().join(format!("al-native-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let shot = dir.join("a.png");
    let refused = b.screenshot(&shot).unwrap_err();
    assert!(
        refused.to_string().contains("nothing to screenshot"),
        "{refused}"
    );
    assert!(!shot.exists());

    b.set_video_focus(true).unwrap();
    let ok = b.screenshot(&shot).unwrap();
    assert_eq!((ok.width, ok.height), (800, 480));
    assert!(shot.is_file());
    let _ = std::fs::remove_dir_all(&dir);

    let caps = b.capabilities().unwrap();
    assert_eq!(caps.backend, "native");
    assert_eq!(caps.video_focus, Support::Supported);
    assert_eq!(caps.touch_input, Support::Unverified);
    b.stop().unwrap();
}

#[test]
fn a_focus_change_the_phone_does_not_confirm_stays_a_timeout() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut b = start("no-confirm");
    let r = b.set_video_focus(false).unwrap();
    assert_eq!(r.status, OpStatus::TimedOut);
    assert!(!r.status.is_success());
    assert_eq!(
        r.observed,
        FocusState::On,
        "observed stays the phone's word"
    );
}

#[test]
fn a_head_unit_that_never_announces_ready_is_a_timeout_and_is_cleaned_up() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let err = launch("never-ready", Duration::from_millis(400))
        .err()
        .expect("must time out");
    assert!(matches!(err, Error::Timeout(_)), "{err}");
    assert_eq!(err.exit_code(), 6);
}

#[test]
fn early_exit_reports_the_exit_status_and_what_the_child_printed() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let early = launch("exit-early", Duration::from_secs(5)).err().unwrap();
    assert!(matches!(early, Error::BackendUnavailable(_)), "{early}");
    assert!(early.to_string().contains("exit status: 4"), "{early}");
    let junk = launch("junk-then-exit", Duration::from_secs(5))
        .err()
        .unwrap();
    assert!(junk.to_string().contains("usb exploded"), "{junk}");
}

#[test]
fn a_head_unit_that_dies_mid_session_is_unavailable_never_a_hang() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut b = start("crash-on-focus");
    let err = b.set_video_focus(false).unwrap_err();
    assert!(matches!(err, Error::BackendUnavailable(_)), "{err}");
    let status = b.exit_status().expect("exit is noticed");
    assert!(
        status.contains("exit") || status.contains("closed"),
        "{status}"
    );
    assert!(matches!(
        b.status().unwrap_err(),
        Error::BackendUnavailable(_)
    ));
}

#[test]
fn a_missing_program_is_backend_unavailable() {
    let err = NativeBackend::launch(&NativeLaunch {
        program: "/no/such/autolab-native-hu".into(),
        serial: None,
    })
    .err()
    .unwrap();
    assert!(matches!(err, Error::BackendUnavailable(_)), "{err}");
}

#[test]
fn dropping_the_backend_ends_the_child() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let b = start("");
    let pid = b.pid();
    drop(b);
    std::thread::sleep(Duration::from_millis(500));
    let alive = std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .unwrap()
        .status
        .success();
    assert!(!alive, "the child must not outlive the backend");
}

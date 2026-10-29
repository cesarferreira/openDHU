use opendhu_test::parse;

#[test]
fn application_actions_validate_text_before_touching_a_device() {
    for action in [
        "action: app.restart",
        "action: device.inputText\n    text: openDHU",
        "action: device.inputText\n    text: 'openDHU driver_1@test.dev'",
        "action: device.inputText\n    text: 'Driver-1'",
    ] {
        let yaml = format!("name: Production journey\nsteps:\n  - {action}\n");
        assert!(parse(&yaml).is_ok(), "{yaml}");
    }
    for text in [
        "",
        "   ",
        "driver;reboot",
        "$(id)",
        "driver%20",
        "one\ntwo",
        "日本",
        &"a".repeat(257),
    ] {
        let yaml = format!(
            "name: Invalid typing\nsteps:\n  - action: device.inputText\n    text: {}\n",
            serde_json::to_string(text).unwrap()
        );
        assert!(parse(&yaml).is_err(), "unsafe input accepted: {text:?}");
    }
}

use opendhu_core::{
    Capabilities, Error, HeadUnitBackend, HeadUnitStatus, InputResult, OperationResult, Result,
    ScreenshotResult,
};
use opendhu_test::Outcome;
use opendhu_test::mock::{FakeClock, MockBackend};
use opendhu_test::runner::{ApplicationControl, RunConfig, run_test};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

struct Application(Rc<RefCell<Vec<String>>>);
impl ApplicationControl for Application {
    fn restart(&self) -> Result<()> {
        self.0.borrow_mut().push("stop application".into());
        Ok(())
    }
    fn input_text(&self, text: &str) -> Result<()> {
        self.0.borrow_mut().push(format!("type {text}"));
        Ok(())
    }
}
struct Projection {
    inner: MockBackend,
    events: Rc<RefCell<Vec<String>>>,
}
impl HeadUnitBackend for Projection {
    fn capabilities(&mut self) -> Result<Capabilities> {
        self.inner.capabilities()
    }
    fn status(&mut self) -> Result<HeadUnitStatus> {
        self.inner.status()
    }
    fn set_video_focus(&mut self, enabled: bool) -> Result<OperationResult> {
        self.inner.set_video_focus(enabled)
    }
    fn stop(&mut self) -> Result<()> {
        self.inner.stop()
    }
    fn tap(&mut self, x: u32, y: u32) -> Result<InputResult> {
        self.events.borrow_mut().push("tap launcher".into());
        self.inner.tap(x, y)
    }
    fn screenshot(&mut self, _: &Path) -> Result<ScreenshotResult> {
        // End this test at the external screenshot/OCR boundary, after the real launcher action.
        Err(Error::Unsupported(
            "projection observation unavailable".into(),
        ))
    }
}

#[test]
fn typing_dispatches_once_and_restart_stops_before_projected_launcher() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let app = Application(Rc::clone(&events));
    let mut projection = Projection {
        inner: MockBackend::new(),
        events: Rc::clone(&events),
    };
    let test = parse("name: Persisted journey\nsteps:\n  - action: device.inputText\n    text: openDHU\n  - action: app.restart\n").unwrap();
    let clock = FakeClock::default();
    let result = run_test(
        &test,
        "app.yaml",
        &mut projection,
        &RunConfig {
            application: Some(&app),
            app_label: Some("Robot UI"),
            artifacts_dir: Path::new("/tmp"),
            skip_unsupported: false,
            clock: &clock,
        },
    );
    assert_eq!(result.steps[1].outcome, Outcome::Passed);
    assert_eq!(
        *events.borrow(),
        vec!["type openDHU", "stop application", "tap launcher"]
    );
    assert_eq!(result.steps[2].outcome, Outcome::Error);
}

#[test]
fn application_actions_require_suite_context_without_sending_input() {
    for action in [
        "action: app.restart",
        "action: device.inputText\n    text: openDHU",
    ] {
        let test = parse(&format!("name: No suite\nsteps:\n  - {action}\n")).unwrap();
        let mut backend = MockBackend::new();
        let clock = FakeClock::default();
        let result = run_test(
            &test,
            "app.yaml",
            &mut backend,
            &RunConfig {
                application: None,
                app_label: Some("Robot UI"),
                artifacts_dir: Path::new("/tmp"),
                skip_unsupported: false,
                clock: &clock,
            },
        );
        assert_eq!(result.outcome, Outcome::Error);
        assert_eq!(result.exit_code, 2);
        assert!(result.steps[1].detail.contains("suite application context"));
        assert!(!backend.calls.iter().any(|call| call.starts_with("tap")));
    }
}

struct RestartCrash(std::cell::Cell<bool>);
impl ApplicationControl for RestartCrash {
    fn check_health(&self) -> Result<Option<String>> {
        Ok(self
            .0
            .get()
            .then(|| "unexpected app crash during relaunch".into()))
    }
    fn restart(&self) -> Result<()> {
        self.0.set(true);
        Ok(())
    }
    fn input_text(&self, _: &str) -> Result<()> {
        Ok(())
    }
}

#[test]
fn a_crash_during_restart_is_not_suppressed_and_stops_launcher_input() {
    let app = RestartCrash(std::cell::Cell::new(false));
    let test = parse("name: Relaunch\nsteps: [{action: app.restart}]\n").unwrap();
    let mut backend = MockBackend::new();
    let clock = FakeClock::default();
    let result = run_test(
        &test,
        "restart.yaml",
        &mut backend,
        &RunConfig {
            application: Some(&app),
            app_label: Some("Test"),
            artifacts_dir: Path::new("/tmp"),
            skip_unsupported: false,
            clock: &clock,
        },
    );
    assert_eq!(result.outcome, Outcome::Failed);
    assert_eq!(result.exit_code, 7);
    assert!(
        !backend.calls.iter().any(|call| call.starts_with("tap")),
        "{:?}",
        backend.calls
    );
}

#[test]
fn an_app_crash_stops_a_long_wait_and_skips_following_input() {
    struct CrashingApp<'a>(&'a FakeClock);
    impl ApplicationControl for CrashingApp<'_> {
        fn check_health(&self) -> Result<Option<String>> {
            use opendhu_test::runner::Clock;
            Ok((self.0.now() >= std::time::Duration::from_secs(1))
                .then(|| "injected fatal crash".into()))
        }
        fn restart(&self) -> Result<()> {
            Ok(())
        }
        fn input_text(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }
    let test = parse("name: Crash during wait\nsteps: [{action: wait, duration: 10m}, {action: tap, x: 1, y: 1}]\n").unwrap();
    let clock = FakeClock::default();
    let app = CrashingApp(&clock);
    let mut backend = MockBackend::new();
    let result = run_test(
        &test,
        "wait.yaml",
        &mut backend,
        &RunConfig {
            application: Some(&app),
            app_label: None,
            artifacts_dir: Path::new("/tmp"),
            skip_unsupported: false,
            clock: &clock,
        },
    );
    assert_eq!(result.outcome, Outcome::Failed);
    assert_eq!(result.steps[2].outcome, Outcome::Skipped);
    use opendhu_test::runner::Clock;
    assert_eq!(clock.now(), std::time::Duration::from_secs(1));
    assert!(!backend.calls.iter().any(|call| call.starts_with("tap")));
}

#[test]
fn health_checks_that_cross_the_wait_deadline_do_not_panic() {
    struct SlowHealth<'a> {
        clock: &'a FakeClock,
        calls: std::cell::Cell<usize>,
    }
    impl ApplicationControl for SlowHealth<'_> {
        fn check_health(&self) -> Result<Option<String>> {
            let count = self.calls.get() + 1;
            self.calls.set(count);
            if count == 2 {
                use opendhu_test::runner::Clock;
                self.clock.sleep(std::time::Duration::from_millis(2));
            }
            Ok(None)
        }
        fn restart(&self) -> Result<()> {
            Ok(())
        }
        fn input_text(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }
    let clock = FakeClock::default();
    let app = SlowHealth {
        clock: &clock,
        calls: std::cell::Cell::new(0),
    };
    let test = parse("name: Deadline\nsteps: [{action: wait, duration: 1ms}]\n").unwrap();
    let result = run_test(
        &test,
        "wait.yaml",
        &mut MockBackend::new(),
        &RunConfig {
            application: Some(&app),
            app_label: None,
            artifacts_dir: Path::new("/tmp"),
            skip_unsupported: false,
            clock: &clock,
        },
    );
    assert_eq!(result.outcome, Outcome::Passed);
}

#[test]
fn remove_preferences_requires_safe_names_and_at_least_one_key() {
    let ok = "name: Fresh\nsteps:\n  - action: app.removePreferences\n    file: APPLICATION_SHARED_PREFERENCES\n    keys: [vehicleMetadata]\n";
    assert!(parse(ok).is_ok());
    for step in [
        "file: p\n    keys: []",
        "file: ../x\n    keys: [a]",
        "file: p\n    keys: ['a b']",
        "file: ''\n    keys: [a]",
        "file: p",
    ] {
        let yaml = format!("name: Fresh\nsteps:\n  - action: app.removePreferences\n    {step}\n");
        assert!(parse(&yaml).is_err(), "{yaml}");
    }
}

use autolab_test::parse;

#[test]
fn application_actions_validate_text_before_touching_a_device() {
    for action in [
        "action: app.restart",
        "action: device.inputText\n    text: AutoLab",
        "action: device.inputText\n    text: 'AutoLab driver_1@test.dev'",
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

use autolab_core::{
    Capabilities, Error, HeadUnitBackend, HeadUnitStatus, InputResult, OperationResult, Result,
    ScreenshotResult,
};
use autolab_test::Outcome;
use autolab_test::mock::{FakeClock, MockBackend};
use autolab_test::runner::{ApplicationControl, RunConfig, run_test};
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
    let test = parse("name: Persisted journey\nsteps:\n  - action: device.inputText\n    text: AutoLab\n  - action: app.restart\n").unwrap();
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
        vec!["type AutoLab", "stop application", "tap launcher"]
    );
    assert_eq!(result.steps[2].outcome, Outcome::Error);
}

#[test]
fn application_actions_require_suite_context_without_sending_input() {
    for action in [
        "action: app.restart",
        "action: device.inputText\n    text: AutoLab",
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

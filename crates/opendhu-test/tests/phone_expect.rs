use opendhu_core::Result;
use opendhu_test::mock::{FakeClock, MockBackend};
use opendhu_test::runner::{ApplicationControl, RunConfig, run_test};
use opendhu_test::{Outcome, parse};
use std::cell::Cell;
use std::path::Path;

/// Shows `Harbour Cafe` on the phone from the `appears_on`th dump onward.
struct Phone {
    dumps: Cell<usize>,
    appears_on: usize,
}
impl ApplicationControl for Phone {
    fn restart(&self) -> Result<()> {
        Ok(())
    }
    fn input_text(&self, _: &str) -> Result<()> {
        Ok(())
    }
    fn phone_ui_dump(&self) -> Result<String> {
        let n = self.dumps.get() + 1;
        self.dumps.set(n);
        let favorite = if n >= self.appears_on {
            r#"<node text="Harbour Cafe" content-desc="" />"#
        } else {
            r#"<node text="None" content-desc="" />"#
        };
        Ok(format!(
            r#"<hierarchy><node text="SAVED PLACE" content-desc="" />{favorite}</hierarchy>"#
        ))
    }
}

fn run(app: Option<&Phone>, yaml: &str) -> opendhu_test::TestResult {
    let clock = FakeClock::default();
    run_test(
        &parse(yaml).unwrap(),
        "phone.yaml",
        &mut MockBackend::new(),
        &RunConfig {
            application: app.map(|a| a as &dyn ApplicationControl),
            app_label: None,
            artifacts_dir: Path::new("/tmp"),
            skip_unsupported: false,
            clock: &clock,
        },
    )
}

const STEP: &str =
    "name: Phone\nsteps:\n  - action: phone.expect\n    text: harbour cafe\n    timeout: 5s\n";

#[test]
fn phone_expect_waits_for_text_in_the_phone_tree() {
    let phone = Phone {
        dumps: Cell::new(0),
        appears_on: 3,
    };
    let result = run(Some(&phone), STEP);
    assert_eq!(result.steps[1].outcome, Outcome::Passed, "{result:?}");
    assert_eq!(phone.dumps.get(), 3);
}

#[test]
fn phone_expect_times_out_and_reports_what_the_phone_shows() {
    let phone = Phone {
        dumps: Cell::new(0),
        appears_on: usize::MAX,
    };
    let result = run(Some(&phone), STEP);
    assert_eq!(result.steps[1].outcome, Outcome::Failed);
    let detail = &result.steps[1].detail;
    assert!(detail.contains("SAVED PLACE | None"), "{detail}");
}

#[test]
fn phone_expect_needs_a_suite_application() {
    let result = run(None, STEP);
    assert_eq!(result.steps[1].outcome, Outcome::Error);
}

#[test]
fn phone_expect_rejects_empty_text() {
    assert!(
        parse("name: x\nsteps:\n  - action: phone.expect\n    text: ' '\n    timeout: 1s\n")
            .is_err()
    );
}

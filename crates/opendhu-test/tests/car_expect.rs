use opendhu_core::Result;
use opendhu_test::mock::{FakeClock, MockBackend};
use opendhu_test::runner::{ApplicationControl, RunConfig, run_test};
use opendhu_test::{Outcome, parse};
use std::cell::Cell;
use std::path::Path;

/// Reports the saved place from the `appears_on`th query onward.
struct Hooked {
    queries: Cell<usize>,
    appears_on: usize,
    has_hook: bool,
}
impl ApplicationControl for Hooked {
    fn restart(&self) -> Result<()> {
        Ok(())
    }
    fn input_text(&self, _: &str) -> Result<()> {
        Ok(())
    }
    fn car_template(&self) -> Result<String> {
        if !self.has_hook {
            return Ok("No result found.\n".into());
        }
        let n = self.queries.get() + 1;
        self.queries.set(n);
        let saved = if n >= self.appears_on {
            "Saved place: Harbour Cafe"
        } else {
            "No saved place"
        };
        Ok(format!(
            r#"Row: 0 json={{"type":"ListTemplate","title":"Demo","rows":[{{"title":"Browse places"}},{{"title":"{saved}"}}]}}"#
        ))
    }
}

fn run(app: &Hooked) -> opendhu_test::TestResult {
    let clock = FakeClock::default();
    run_test(
        &parse("name: Car\nsteps:\n  - action: car.expect\n    text: 'saved place: harbour'\n    timeout: 5s\n")
            .unwrap(),
        "car.yaml",
        &mut MockBackend::new(),
        &RunConfig {
            application: Some(app),
            app_label: None,
            artifacts_dir: Path::new("/tmp"),
            skip_unsupported: false,
            clock: &clock,
        },
    )
}

#[test]
fn car_expect_waits_for_text_in_the_reported_template() {
    let app = Hooked {
        queries: Cell::new(0),
        appears_on: 3,
        has_hook: true,
    };
    let result = run(&app);
    assert_eq!(result.steps[1].outcome, Outcome::Passed, "{result:?}");
    assert_eq!(app.queries.get(), 3);
}

#[test]
fn car_expect_times_out_listing_what_the_template_shows() {
    let app = Hooked {
        queries: Cell::new(0),
        appears_on: usize::MAX,
        has_hook: true,
    };
    let result = run(&app);
    assert_eq!(result.steps[1].outcome, Outcome::Failed);
    assert!(
        result.steps[1]
            .detail
            .contains("Demo | Browse places | No saved place"),
        "{}",
        result.steps[1].detail
    );
}

#[test]
fn car_expect_says_when_the_app_has_no_hook() {
    let app = Hooked {
        queries: Cell::new(0),
        appears_on: 1,
        has_hook: false,
    };
    let result = run(&app);
    assert_eq!(result.steps[1].outcome, Outcome::Error);
    assert!(result.steps[1].detail.contains("no openDHU template hook"));
}

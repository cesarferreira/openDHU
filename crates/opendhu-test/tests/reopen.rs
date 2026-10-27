//! `reopen` restarts and reopens the app when a `ui.waitFor`/`ui.expect` text never shows up.
use opendhu_core::{
    Capabilities, HeadUnitBackend, HeadUnitStatus, InputResult, OperationResult, Result,
    ScreenshotResult,
};
use opendhu_test::mock::{FakeClock, MockBackend};
use opendhu_test::runner::{ApplicationControl, RunConfig, run_test};
use opendhu_test::{Outcome, parse};
use std::cell::Cell;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::rc::Rc;

/// Counts restarts; the projection reads the count to decide what the car shows.
struct App(Rc<Cell<usize>>);
impl ApplicationControl for App {
    fn restart(&self) -> Result<()> {
        self.0.set(self.0.get() + 1);
        Ok(())
    }
    fn input_text(&self, _: &str) -> Result<()> {
        Ok(())
    }
}

/// Always shows the launcher label; shows the app's content only after `needs` restarts.
struct Projection {
    inner: MockBackend,
    restarts: Rc<Cell<usize>>,
    needs: usize,
}
impl HeadUnitBackend for Projection {
    fn capabilities(&mut self) -> Result<Capabilities> {
        self.inner.capabilities()
    }
    fn status(&mut self) -> Result<HeadUnitStatus> {
        self.inner.status()
    }
    fn set_video_focus(&mut self, value: bool) -> Result<OperationResult> {
        self.inner.set_video_focus(value)
    }
    fn stop(&mut self) -> Result<()> {
        self.inner.stop()
    }
    fn tap(&mut self, x: u32, y: u32) -> Result<InputResult> {
        self.inner.tap(x, y)
    }
    fn screenshot(&mut self, path: &Path) -> Result<ScreenshotResult> {
        let header = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n";
        let mut tsv = format!("{header}5\t1\t1\t1\t1\t1\t20\t200\t100\t20\t99\tRobot UI\n");
        if self.restarts.get() >= self.needs {
            tsv.push_str("5\t1\t2\t1\t1\t1\t20\t300\t100\t20\t99\t1 available\n");
        }
        std::fs::write(path, tsv).unwrap();
        Ok(ScreenshotResult {
            path: path.into(),
            width: 800,
            height: 480,
        })
    }
}

fn step(extra: &str) -> String {
    format!(
        "name: Reopen\nsteps:\n  - action: app.open\n  - action: ui.expect\n    text: 1 available\n    timeout: 20s\n{extra}"
    )
}

#[test]
fn reopen_recovers_a_launch_that_never_showed_the_app() {
    let dir = std::env::temp_dir().join(format!("aa-reopen-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, script) in [
        (
            "ffmpeg",
            "#!/bin/sh\nwhile [ $# -gt 0 ]; do if [ \"$1\" = -i ]; then shift; input=$1; fi; output=$1; shift; done\ncp \"$input\" \"$output\"\n",
        ),
        ("tesseract", "#!/bin/sh\ncp \"$1\" \"$2.tsv\"\n"),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let old_path = std::env::var_os("PATH").unwrap();
    let old_ocr = std::env::var_os("OPENDHU_TESSERACT");
    // One test function, because only the external OCR processes see this PATH.
    unsafe {
        std::env::set_var(
            "PATH",
            format!("{}:{}", dir.display(), old_path.to_string_lossy()),
        );
        std::env::set_var("OPENDHU_TESSERACT", dir.join("tesseract"));
    }

    // (extra step fields, restarts the app needs, expected outcome, expected restarts)
    let cases: [(&str, usize, Outcome, usize); 5] = [
        // Recovers on the first reopen.
        (
            "    reopen: 2\n    reopenAfter: 5s\n",
            1,
            Outcome::Passed,
            1,
        ),
        // Never recovers: gives up after exactly `reopen` restarts.
        (
            "    reopen: 2\n    reopenAfter: 5s\n",
            usize::MAX,
            Outcome::Failed,
            2,
        ),
        // Without `reopen` nothing changes: no restart, the step times out.
        ("", 1, Outcome::Failed, 0),
        // Text already there: no restart.
        ("    reopen: 2\n", 0, Outcome::Passed, 0),
        // Recovers on the second reopen.
        (
            "    reopen: 3\n    reopenAfter: 5s\n",
            2,
            Outcome::Passed,
            2,
        ),
    ];
    for (extra, needs, outcome, restarts) in cases {
        let count = Rc::new(Cell::new(0));
        let app = App(Rc::clone(&count));
        let mut projection = Projection {
            inner: MockBackend::new(),
            restarts: Rc::clone(&count),
            needs,
        };
        let clock = FakeClock::default();
        let result = run_test(
            &parse(&step(extra)).unwrap(),
            "reopen.yaml",
            &mut projection,
            &RunConfig {
                application: Some(&app),
                app_label: Some("Robot UI"),
                artifacts_dir: &dir,
                skip_unsupported: false,
                clock: &clock,
            },
        );
        let detail = &result.steps[2].detail;
        assert_eq!(result.outcome, outcome, "{extra:?}: {detail}");
        assert_eq!(count.get(), restarts, "{extra:?}: {detail}");
        if restarts > 0 {
            assert!(detail.contains("after reopening the app"), "{detail}");
        } else {
            assert!(!detail.contains("reopening"), "{detail}");
        }
        if outcome == Outcome::Failed && restarts > 0 {
            assert_eq!(
                result.exit_code, 6,
                "a failed recovery still ends as a timeout"
            );
        }
    }

    // Reopening needs the suite's application.
    let count = Rc::new(Cell::new(0));
    let mut projection = Projection {
        inner: MockBackend::new(),
        restarts: Rc::clone(&count),
        needs: usize::MAX,
    };
    let clock = FakeClock::default();
    let result = run_test(
        &parse(&step("    reopen: 1\n    reopenAfter: 2s\n")).unwrap(),
        "reopen.yaml",
        &mut projection,
        &RunConfig {
            application: None,
            app_label: Some("Robot UI"),
            artifacts_dir: &dir,
            skip_unsupported: false,
            clock: &clock,
        },
    );
    assert_eq!(result.steps[2].outcome, Outcome::Error);
    assert!(result.steps[2].detail.contains("suite application context"));

    unsafe {
        std::env::set_var("PATH", old_path);
        match old_ocr {
            Some(value) => std::env::set_var("OPENDHU_TESSERACT", value),
            None => std::env::remove_var("OPENDHU_TESSERACT"),
        };
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn reopen_options_are_validated() {
    let wait = |extra: &str, action: &str| {
        parse(&format!(
            "name: x\nsteps:\n  - action: {action}\n    text: hi\n    timeout: 5s\n{extra}"
        ))
    };
    for action in ["ui.expect", "ui.waitFor"] {
        assert!(wait("    reopen: 2\n    reopenAfter: 8s\n", action).is_ok());
        assert!(wait("    reopen: 5\n", action).is_ok());
        assert!(wait("    reopen: 6\n", action).is_err(), "too many reopens");
        assert!(
            wait("    reopenAfter: 8s\n", action).is_err(),
            "reopenAfter needs reopen"
        );
        assert!(wait("    reopen: 1\n    reopenAfter: 0s\n", action).is_err());
    }
    // Only waits can reopen: a tap must not be replayed against a fresh app.
    assert!(
        parse(
            "name: x\nsteps:\n  - action: ui.tap\n    text: hi\n    timeout: 5s\n    reopen: 1\n"
        )
        .is_err()
    );
}

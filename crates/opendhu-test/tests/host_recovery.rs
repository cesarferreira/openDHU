//! A USB phone's Android Auto host can keep its "Waiting..." card up after the app restarts.
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

const HEADER: &str = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n";

/// What the host shows once the app's icon is tapped.
#[derive(Clone, Copy, PartialEq)]
enum Opens {
    App,
    /// The card for this many frames, then the app.
    CardFor(usize),
    StuckUntilRecovered,
    Stuck,
}

struct Projection {
    inner: MockBackend,
    opens: Opens,
    recovered: Rc<Cell<bool>>,
    app_tapped: bool,
    frames_after_tap: usize,
    launcher_taps: usize,
}
impl Projection {
    fn shows_card(&self) -> bool {
        match self.opens {
            Opens::App => false,
            Opens::CardFor(frames) => self.frames_after_tap <= frames,
            Opens::StuckUntilRecovered => !self.recovered.get(),
            Opens::Stuck => true,
        }
    }
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
        if (x, y) == (770, 452) {
            self.launcher_taps += 1;
            self.app_tapped = false;
            self.frames_after_tap = 0;
        } else {
            self.app_tapped = true;
        }
        self.inner.tap(x, y)
    }
    fn screenshot(&mut self, path: &Path) -> Result<ScreenshotResult> {
        let rows: Vec<&str> = if !self.app_tapped {
            vec!["Robot UI"]
        } else {
            self.frames_after_tap += 1;
            if self.shows_card() {
                vec!["Waiting...", "Exit"]
            } else {
                vec!["VSO", "Drive"]
            }
        };
        let mut tsv = String::from(HEADER);
        for (n, row) in rows.iter().enumerate() {
            let line = n + 1;
            let top = 200 + 40 * n;
            tsv.push_str(&format!(
                "5\t1\t1\t1\t{line}\t1\t20\t{top}\t100\t20\t99\t{row}\n"
            ));
        }
        std::fs::write(path, tsv).unwrap();
        Ok(ScreenshotResult {
            path: path.into(),
            width: 800,
            height: 480,
        })
    }
}

struct Application {
    can_recover: bool,
    recovered: Rc<Cell<bool>>,
    recoveries: Cell<usize>,
}
impl ApplicationControl for Application {
    fn restart(&self) -> Result<()> {
        Ok(())
    }
    fn input_text(&self, _: &str) -> Result<()> {
        Ok(())
    }
    fn recover_projection(&self) -> Result<bool> {
        if !self.can_recover {
            return Ok(false);
        }
        self.recoveries.set(self.recoveries.get() + 1);
        self.recovered.set(true);
        Ok(true)
    }
}

#[test]
fn a_host_stuck_on_waiting_is_recovered_by_a_new_session_once() {
    let dir = std::env::temp_dir().join(format!("aa-host-recovery-{}", std::process::id()));
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
    // This integration test has one test; only external OCR processes use the fixture PATH.
    unsafe {
        std::env::set_var(
            "PATH",
            format!("{}:{}", dir.display(), old_path.to_string_lossy()),
        );
        std::env::set_var("OPENDHU_TESSERACT", dir.join("tesseract"));
    }
    let test = parse("name: Relaunch\nsteps:\n  - action: app.restart\n").unwrap();
    for (opens, can_recover, outcome, recoveries, launcher_taps, detail) in [
        (Opens::App, true, Outcome::Passed, 0, 1, "Robot UI"),
        (Opens::CardFor(3), true, Outcome::Passed, 0, 1, "Robot UI"),
        (
            Opens::StuckUntilRecovered,
            true,
            Outcome::Passed,
            1,
            2,
            "recovered after Android Auto kept showing",
        ),
        (
            Opens::StuckUntilRecovered,
            false,
            Outcome::Passed,
            0,
            1,
            "only a `--usb` suite can restart",
        ),
        (
            Opens::Stuck,
            true,
            Outcome::Failed,
            1,
            2,
            "again after restarting the head-unit session",
        ),
    ] {
        let recovered = Rc::new(Cell::new(false));
        let app = Application {
            can_recover,
            recovered: Rc::clone(&recovered),
            recoveries: Cell::new(0),
        };
        let mut projection = Projection {
            inner: MockBackend::new(),
            opens,
            recovered,
            app_tapped: false,
            frames_after_tap: 0,
            launcher_taps: 0,
        };
        let clock = FakeClock::default();
        let result = run_test(
            &test,
            "restart.yaml",
            &mut projection,
            &RunConfig {
                application: Some(&app),
                app_label: Some("Robot UI"),
                artifacts_dir: &dir,
                skip_unsupported: false,
                clock: &clock,
            },
        );
        let step = &result.steps[1];
        assert_eq!(step.outcome, outcome, "{}", step.detail);
        assert!(step.detail.contains(detail), "{}", step.detail);
        assert_eq!(app.recoveries.get(), recoveries);
        assert_eq!(projection.launcher_taps, launcher_taps);
    }
    unsafe {
        std::env::set_var("PATH", old_path);
        match old_ocr {
            Some(value) => std::env::set_var("OPENDHU_TESSERACT", value),
            None => std::env::remove_var("OPENDHU_TESSERACT"),
        };
    }
    std::fs::remove_dir_all(dir).unwrap();
}

//! Deterministic projected observations exercise retry, timeout, and single-tap behavior.
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use autolab_core::{
    Capabilities, HeadUnitBackend, HeadUnitStatus, InputResult, OperationResult, Result,
    ScreenshotResult,
};
use autolab_test::mock::{FakeClock, MockBackend};
use autolab_test::runner::{Clock, RunConfig, run_test};
use autolab_test::{Outcome, parse};

struct Projection {
    inner: MockBackend,
    taps: usize,
    ready_after: usize,
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
        self.taps += 1;
        self.inner.tap(x, y)
    }
    fn screenshot(&mut self, path: &Path) -> Result<ScreenshotResult> {
        let text = if self.taps >= self.ready_after {
            "Exit map"
        } else {
            "Active route"
        };
        std::fs::write(path, format!(
            "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n5\t1\t1\t1\t1\t1\t20\t20\t100\t20\t99\t{text}\n"
        )).unwrap();
        Ok(ScreenshotResult {
            path: path.into(),
            width: 800,
            height: 480,
        })
    }
}

#[test]
fn tapping_until_observes_completion_retries_and_preserves_timeout_and_cancellation() {
    let dir = std::env::temp_dir().join(format!("aa-ui-tap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // OCR and image scaling are external dependencies. Feed deterministic TSV observations through
    // their process boundary; the production runner, phrase matching, and tap decisions run normally.
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
    let old_ocr = std::env::var_os("AUTOLAB_TESSERACT");
    // This integration-test process contains only this test; child processes inherit these tools.
    unsafe {
        std::env::set_var(
            "PATH",
            format!("{}:{}", dir.display(), old_path.to_string_lossy()),
        );
        std::env::set_var("AUTOLAB_TESSERACT", dir.join("tesseract"));
    }
    for (until, ready_after, cancel_after, outcome, taps) in [
        (true, 2, None, Outcome::Passed, 2),
        (true, 0, None, Outcome::Passed, 0),
        (true, usize::MAX, None, Outcome::Failed, 2),
        (false, usize::MAX, None, Outcome::Passed, 1),
        (true, usize::MAX, Some(1), Outcome::Error, 1),
    ] {
        let completion = if until { "\n    until: Exit map" } else { "" };
        let test = parse(&format!("name: Tap navigation\nsteps:\n  - action: ui.tap\n    text: Active route\n    timeout: 1s{completion}\n")).unwrap();
        let mut projection = Projection {
            inner: MockBackend::new(),
            taps: 0,
            ready_after,
        };
        let clock = FakeClock::default();
        clock.cancel_after_sleeps.set(cancel_after);
        let result = run_test(
            &test,
            "tap.yaml",
            &mut projection,
            &RunConfig {
                app_label: None,
                artifacts_dir: &dir,
                skip_unsupported: false,
                clock: &clock,
            },
        );
        assert_eq!(result.outcome, outcome, "{:?}", result.message);
        assert_eq!(projection.taps, taps);
        if until && ready_after == usize::MAX && cancel_after.is_none() {
            assert_eq!(result.exit_code, 6);
            assert_eq!(clock.now(), Duration::from_secs(1));
        }
        if cancel_after.is_some() {
            assert_eq!(result.exit_code, 130);
        }
    }
    unsafe {
        std::env::set_var("PATH", old_path);
        match old_ocr {
            Some(value) => std::env::set_var("AUTOLAB_TESSERACT", value),
            None => std::env::remove_var("AUTOLAB_TESSERACT"),
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn empty_until_is_rejected_before_execution() {
    let error = parse("name: Tap\nsteps:\n  - action: ui.tap\n    text: Active route\n    timeout: 1s\n    until: '  '\n").unwrap_err();
    assert!(
        error.to_string().contains("until must not be empty"),
        "{error}"
    );
}

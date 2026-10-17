//! Android Auto can recreate an app session before it starts accepting launcher touches.
use opendhu_core::{
    Capabilities, HeadUnitBackend, HeadUnitStatus, InputResult, OperationResult, Result,
    ScreenshotResult,
};
use opendhu_test::mock::{FakeClock, MockBackend};
use opendhu_test::runner::{Clock, RunConfig, run_test};
use opendhu_test::{Outcome, parse};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

struct Projection {
    inner: MockBackend,
    launcher_probes: usize,
    ready_after: usize,
    pending_text: &'static str,
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
            self.launcher_probes += 1;
        }
        self.inner.tap(x, y)
    }
    fn screenshot(&mut self, path: &Path) -> Result<ScreenshotResult> {
        let text = if self.launcher_probes >= self.ready_after {
            "Robot UI"
        } else {
            self.pending_text
        };
        std::fs::write(path,format!("level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n5\t1\t1\t1\t1\t1\t20\t200\t100\t20\t99\t{text}\n")).unwrap();
        Ok(ScreenshotResult {
            path: path.into(),
            width: 800,
            height: 480,
        })
    }
}

#[test]
fn app_open_waits_for_recreated_session_with_a_deadline_and_cancellation() {
    let dir = std::env::temp_dir().join(format!("aa-app-open-{}", std::process::id()));
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
    let test =
        parse("name: Wait for Android Auto restart\nsteps:\n  - action: app.open\n").unwrap();
    for (pending_text, ready_after, cancel_after, outcome, code) in [
        ("Home screen", 5, None, Outcome::Passed, 0),
        (
            "Robot UI has encountered an unexpected error",
            5,
            None,
            Outcome::Passed,
            0,
        ),
        ("Robot UI Storybook", 5, None, Outcome::Passed, 0),
        (
            "Robot UI has encountered an unexpected error",
            usize::MAX,
            None,
            Outcome::Failed,
            6,
        ),
        ("Home screen", usize::MAX, None, Outcome::Failed, 6),
        ("Home screen", usize::MAX, Some(2), Outcome::Error, 130),
    ] {
        let mut projection = Projection {
            inner: MockBackend::new(),
            launcher_probes: 0,
            ready_after,
            pending_text,
        };
        let clock = FakeClock::default();
        clock.cancel_after_sleeps.set(cancel_after);
        let result = run_test(
            &test,
            "open.yaml",
            &mut projection,
            &RunConfig {
                application: None,
                app_label: Some("Robot UI"),
                artifacts_dir: &dir,
                skip_unsupported: false,
                clock: &clock,
            },
        );
        assert_eq!(result.outcome, outcome, "{:?}", result.message);
        assert_eq!(result.exit_code, code);
        if ready_after == 5 {
            assert_eq!(projection.launcher_probes, 5);
        }
        if ready_after == usize::MAX && cancel_after.is_none() {
            assert_eq!(clock.now(), Duration::from_secs(30));
        }
        if cancel_after.is_some() {
            assert_eq!(projection.launcher_probes, 1);
        }
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

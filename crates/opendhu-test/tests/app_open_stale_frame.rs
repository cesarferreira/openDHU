//! A launcher label read by OCR can be stale by the time the tap is sent: Android Auto may have
//! resumed the app meanwhile, and the tap then lands on the app's own screen.
use opendhu_core::{
    Capabilities, HeadUnitBackend, HeadUnitStatus, InputResult, OperationResult, Result,
    ScreenshotResult,
};
use opendhu_test::mock::{FakeClock, MockBackend};
use opendhu_test::runner::{RunConfig, run_test};
use opendhu_test::{Outcome, parse};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Shows `frames[i]` on the i-th screenshot (the last one repeats) and records every tap with the
/// number of screenshots taken before it.
struct Projection {
    inner: MockBackend,
    frames: Vec<&'static str>,
    shots: usize,
    taps: Vec<((u32, u32), usize)>,
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
        self.taps.push(((x, y), self.shots));
        self.inner.tap(x, y)
    }
    fn screenshot(&mut self, path: &Path) -> Result<ScreenshotResult> {
        let text = self.frames[self.shots.min(self.frames.len() - 1)];
        self.shots += 1;
        std::fs::write(path, format!("level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n5\t1\t1\t1\t1\t1\t20\t200\t100\t20\t99\t{text}\n")).unwrap();
        Ok(ScreenshotResult {
            path: path.into(),
            width: 800,
            height: 480,
        })
    }
}

#[test]
fn launcher_label_is_not_tapped_from_a_frame_the_screen_already_left() {
    let dir = std::env::temp_dir().join(format!("aa-app-open-stale-{}", std::process::id()));
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
    // This integration test file has one test; only external OCR processes use the fixture PATH.
    unsafe {
        std::env::set_var(
            "PATH",
            format!("{}:{}", dir.display(), old_path.to_string_lossy()),
        );
        std::env::set_var("OPENDHU_TESSERACT", dir.join("tesseract"));
    }
    let test = parse("name: Open\nsteps:\n  - action: app.open\n").unwrap();
    let run = |frames: Vec<&'static str>| {
        let mut projection = Projection {
            inner: MockBackend::new(),
            frames,
            shots: 0,
            taps: Vec::new(),
        };
        let clock = FakeClock::default();
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
        let label_taps: Vec<usize> = projection
            .taps
            .iter()
            .filter(|((x, y), _)| (*x, *y) != (770, 452))
            .map(|(_, shots)| *shots)
            .collect();
        (result, label_taps)
    };

    // Given the label is steady in two frames, but the app replaced the launcher before the tap
    let (result, label_taps) = run(vec!["Robot UI", "Robot UI", "Home screen", "Robot UI"]);
    // Then the label is tapped only once the screen shows it again, never from the stale frames
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.message);
    assert_eq!(label_taps.len(), 1);
    assert!(
        label_taps[0] > 3,
        "tapped after only {} screenshots",
        label_taps[0]
    );

    // Given a screen that is the same at the re-check, then the label is tapped right after it
    let (result, label_taps) = run(vec!["Robot UI"]);
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.message);
    assert_eq!(label_taps, vec![3]);

    unsafe {
        std::env::set_var("PATH", old_path);
        match old_ocr {
            Some(value) => std::env::set_var("OPENDHU_TESSERACT", value),
            None => std::env::remove_var("OPENDHU_TESSERACT"),
        };
    }
    std::fs::remove_dir_all(dir).unwrap();
}

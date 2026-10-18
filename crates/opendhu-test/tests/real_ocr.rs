//! Captured projected UI regressions. Requires ffmpeg and English Tesseract on PATH.
use opendhu_test::screen;

#[test]
#[ignore = "requires ffmpeg and English Tesseract; run explicitly with --ignored"]
fn recognizes_dimmed_robotaxi_status_from_a_real_projected_frame() {
    let dir = std::env::temp_dir().join(format!("aa-real-ocr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let image = dir.join("robotaxi.png");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/robotaxi-accepting-jobs.png"
        ),
        &image,
    )
    .unwrap();
    let observed = screen::recognize(&image, || false).unwrap();
    let found = screen::locate(&observed, "Accepting jobs");
    assert!(
        found.is_some(),
        "dimmed status omitted from projected OCR: {}",
        screen::text(&observed)
    );
    let (x, y) = found.unwrap();
    assert!(
        (150..250).contains(&x) && (90..130).contains(&y),
        "status bounds must remain in the original 800x480 image: {x},{y}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "requires ffmpeg and English Tesseract; run explicitly with --ignored"]
fn types_after_search_results_change_ocr_segmentation_in_real_keyboard_frames() {
    use opendhu_core::{
        Capabilities, HeadUnitBackend, HeadUnitStatus, InputResult, OperationResult, Result,
        ScreenshotResult,
    };
    use opendhu_test::mock::{FakeClock, MockBackend};
    use opendhu_test::runner::{RunConfig, run_test};
    use opendhu_test::{Outcome, parse};
    use std::path::{Path, PathBuf};

    struct CapturedKeyboard {
        inner: MockBackend,
        image: PathBuf,
        taps: Vec<(u32, u32)>,
    }
    impl HeadUnitBackend for CapturedKeyboard {
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
            self.taps.push((x, y));
            self.inner.tap(x, y)
        }
        fn screenshot(&mut self, path: &Path) -> Result<ScreenshotResult> {
            std::fs::copy(&self.image, path).unwrap();
            Ok(ScreenshotResult {
                path: path.into(),
                width: 800,
                height: 480,
            })
        }
    }
    let dir = std::env::temp_dir().join(format!("aa-keyboard-real-ocr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let test = parse("name: Captured keyboard\nsteps:\n  - action: keyboard.type\n    text: b\n    timeout: 1s\n").unwrap();
    for fixture in [
        "keyboard-model-search-au.png",
        "keyboard-model-search-l.png",
    ] {
        let mut inner = MockBackend::new();
        inner.state.backend = "native".into();
        inner.state.screen_width = Some(800);
        inner.state.screen_height = Some(480);
        let mut backend = CapturedKeyboard {
            inner,
            image: Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(fixture),
            taps: vec![],
        };
        let clock = FakeClock::default();
        let result = run_test(
            &test,
            "keyboard.yaml",
            &mut backend,
            &RunConfig {
                application: None,
                app_label: None,
                artifacts_dir: &dir,
                skip_unsupported: false,
                clock: &clock,
            },
        );
        assert_eq!(
            result.outcome,
            Outcome::Passed,
            "{fixture}: {:?}",
            result.message
        );
        assert_eq!(
            backend.taps,
            [(399, 359)],
            "crop bounds must map back to full-frame key coordinates"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "requires ffmpeg and English Tesseract; run explicitly with --ignored"]
fn recognizes_start_inside_outlined_and_filled_controls_from_real_projected_frames() {
    let dir = std::env::temp_dir().join(format!("aa-start-real-ocr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut missing = vec![];
    for (fixture, expected_x, expected_y) in [
        ("assignment-header-start.png", 737_u32, 34_u32),
        ("assignment-preview-start.png", 159, 383),
        ("assignment-preview-start-updated.png", 159, 383),
        ("assignment-preview-start-dc.png", 159, 383),
    ] {
        let image = dir.join(fixture);
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(fixture),
            &image,
        )
        .unwrap();
        let words = screen::recognize(&image, || false).unwrap();
        if let Some((x, y)) = screen::locate(&words, "Start") {
            assert!(
                x.abs_diff(expected_x) <= 8 && y.abs_diff(expected_y) <= 8,
                "{fixture}: matched another label or failed to map bounds to full frame: {x},{y}"
            );
        } else {
            missing.push(fixture);
        }
        assert!(screen::locate(&words, "Never visible action").is_none());
    }
    assert!(
        missing.is_empty(),
        "visible Start controls omitted from OCR: {missing:?}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

//! A synthetic projected frame exercises the real keyboard layout guard and touch decisions.
use opendhu_core::{
    Capabilities, HeadUnitBackend, HeadUnitStatus, InputResult, OperationResult, Result,
    ScreenshotResult,
};
use opendhu_test::mock::{FakeClock, MockBackend};
use opendhu_test::runner::{Clock, RunConfig, run_test};
use opendhu_test::{Outcome, parse};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[derive(Clone, Copy)]
enum Layout {
    Lowercase,
    Uppercase,
    Qwertz,
}

struct Projection {
    inner: MockBackend,
    markers: usize,
    layout: Layout,
    disappear: bool,
    taps: Vec<(u32, u32)>,
    frames: usize,
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
        self.taps.push((x, y));
        self.inner.tap(x, y)
    }
    fn screenshot(&mut self, path: &Path) -> Result<ScreenshotResult> {
        self.frames += 1;
        let mut text = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n".to_string();
        if !self.disappear || self.taps.is_empty() {
            for (index, (glyph, x, y)) in [
                ("q", 164, 283),
                ("e", 258, 283),
                ("t", 352, 283),
                ("u", 447, 283),
                ("a", 188, 320),
                ("1", 164, 245),
                ("y", 399, 283),
                ("z", 211, 359),
            ]
            .into_iter()
            .take(self.markers)
            .enumerate()
            {
                let glyph = match self.layout {
                    Layout::Uppercase => glyph.to_uppercase(),
                    Layout::Qwertz if glyph == "y" => "z".into(),
                    Layout::Qwertz if glyph == "z" => "y".into(),
                    _ => glyph.into(),
                };
                text.push_str(&format!(
                    "5\t1\t1\t1\t{}\t1\t{}\t{}\t12\t16\t99\t{glyph}\n",
                    index + 1,
                    x - 6,
                    y - 8
                ));
            }
        }
        std::fs::write(path, text).unwrap();
        Ok(ScreenshotResult {
            path: path.into(),
            width: 800,
            height: 480,
        })
    }
}

#[test]
fn projected_keyboard_is_guarded_per_character_and_types_only_the_supported_profile() {
    let dir = std::env::temp_dir().join(format!("aa-keyboard-{}", std::process::id()));
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
    // This test binary has one process-boundary test; no other test uses these external tools.
    unsafe {
        std::env::set_var(
            "PATH",
            format!("{}:{}", dir.display(), old_path.to_string_lossy()),
        );
        std::env::set_var("OPENDHU_TESSERACT", dir.join("tesseract"));
    }
    for (text, timeout, markers, layout, disappear, width, cancel, outcome, expected) in [
        (
            "aut 10z",
            "2s",
            8,
            Layout::Lowercase,
            false,
            800,
            None,
            Outcome::Passed,
            vec![
                (188, 320),
                (446, 283),
                (352, 283),
                (392, 397),
                (164, 245),
                (587, 245),
                (211, 359),
            ],
        ),
        (
            "aut",
            "1s",
            8,
            Layout::Qwertz,
            false,
            800,
            None,
            Outcome::Error,
            vec![],
        ),
        (
            "aut",
            "1s",
            2,
            Layout::Lowercase,
            false,
            800,
            None,
            Outcome::Error,
            vec![],
        ),
        (
            "aut",
            "1s",
            8,
            Layout::Uppercase,
            false,
            800,
            None,
            Outcome::Error,
            vec![],
        ),
        (
            "aut",
            "1s",
            8,
            Layout::Lowercase,
            true,
            800,
            None,
            Outcome::Error,
            vec![(188, 320)],
        ),
        (
            "aut",
            "1s",
            8,
            Layout::Lowercase,
            false,
            1280,
            None,
            Outcome::Error,
            vec![],
        ),
        (
            "aut",
            "200ms",
            8,
            Layout::Lowercase,
            false,
            800,
            None,
            Outcome::Error,
            vec![(188, 320), (446, 283)],
        ),
        (
            "aut",
            "1s",
            8,
            Layout::Lowercase,
            false,
            800,
            Some(1),
            Outcome::Error,
            vec![(188, 320)],
        ),
    ] {
        let mut inner = MockBackend::new();
        inner.state.backend = "native".into();
        inner.state.screen_width = Some(width);
        inner.state.screen_height = Some(480);
        let mut projection = Projection {
            inner,
            markers,
            layout,
            disappear,
            taps: vec![],
            frames: 0,
        };
        let test=parse(&format!("name: Keyboard\nsteps:\n  - action: keyboard.type\n    text: {}\n    timeout: {timeout}\n",serde_json::to_string(text).unwrap())).unwrap();
        let clock = FakeClock::default();
        clock.cancel_after_sleeps.set(cancel);
        let result = run_test(
            &test,
            "keyboard.yaml",
            &mut projection,
            &RunConfig {
                application: None,
                app_label: None,
                artifacts_dir: &dir,
                skip_unsupported: false,
                clock: &clock,
            },
        );
        assert_eq!(result.outcome, outcome, "{:?}", result.message);
        assert_eq!(projection.taps, expected, "{:?}", result.message);
        if width != 800 {
            assert_eq!(result.exit_code, 5);
            assert_eq!(projection.frames, 0);
        } else if cancel.is_some() {
            assert_eq!(result.exit_code, 130);
        } else if outcome != Outcome::Passed {
            assert_eq!(result.exit_code, 6);
            assert!(clock.now() <= std::time::Duration::from_secs(1));
        }
    }
    unsafe {
        std::env::set_var("PATH", old_path);
        match old_ocr {
            Some(value) => std::env::set_var("OPENDHU_TESSERACT", value),
            None => std::env::remove_var("OPENDHU_TESSERACT"),
        }
    }
    assert!(std::fs::read_dir(dir.join("keyboard")).unwrap().count() > 0);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unsupported_keyboard_input_is_rejected_before_execution() {
    for text in ["", "Aut", "a-b", "a_b", "a\nb", "日本", &"a".repeat(129)] {
        let error = parse(&format!(
            "name: Keyboard\nsteps:\n  - action: keyboard.type\n    text: {}\n    timeout: 1s\n",
            serde_json::to_string(text).unwrap()
        ))
        .unwrap_err();
        assert!(error.to_string().contains("lowercase ASCII"), "{error}");
    }
}

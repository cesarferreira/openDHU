//! Suite validation must be useful in CI before an APK or Android device exists.
mod common;
use common::*;

#[test]
fn validates_suite_relative_globs_and_ui_actions_without_a_device_or_apk() {
    let s = Sandbox::new();
    let dir = s.scratch("suite");
    std::fs::create_dir_all(dir.join("tests")).unwrap();
    std::fs::write(dir.join("suite.yaml"), "version: 1\nenvironment:\n  emulator: medium_phone\n  apk: missing.apk\n  package: ai.wayve.robotui.storybook\n  appLabel: RobotUI Storybook\ntests: [tests/*.yaml]\n").unwrap();
    std::fs::write(dir.join("tests/catalog.yaml"), "name: catalog\nbackend: native\nsteps:\n  - action: app.open\n  - action: ui.waitFor\n    text: RobotUI Storybook\n    timeout: 15s\n").unwrap();
    let output = s.autolab_env(
        &[
            "test",
            dir.join("suite.yaml").to_str().unwrap(),
            "--validate",
            "--json",
        ],
        &[("PATH", "/no/tools")],
    );
    assert_eq!(code(&output), 0, "{}", text(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["valid"], true);
    assert_eq!(value["tests"], 1);
    assert!(!s.dir.exists(), "validation must not start a daemon");
}

#[test]
fn invalid_suite_test_is_rejected_before_device_setup() {
    let s = Sandbox::new();
    let dir = s.scratch("suite");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("suite.yaml"), "version: 1\nenvironment: {emulator: medium_phone, apk: missing.apk, package: test.app, appLabel: Test}\ntests: [bad.yaml]\n").unwrap();
    std::fs::write(
        dir.join("bad.yaml"),
        "name: bad\nsteps:\n  - action: ui.expect\n    text: ''\n    timeout: 0s\n",
    )
    .unwrap();
    let output = s.autolab_env(
        &["test", dir.join("suite.yaml").to_str().unwrap()],
        &[("PATH", "/no/tools")],
    );
    assert_eq!(code(&output), 2, "{}", text(&output));
    assert!(text(&output).contains("timeout"));
    assert!(!s.dir.exists(), "invalid tests must not start a daemon");
}

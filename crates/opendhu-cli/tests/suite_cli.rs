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
    let output = s.opendhu_env(
        &[
            "test",
            dir.join("suite.yaml").to_str().unwrap(),
            "--validate",
            "--show-head-unit",
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
    let output = s.opendhu_env(
        &["test", dir.join("suite.yaml").to_str().unwrap()],
        &[("PATH", "/no/tools")],
    );
    assert_eq!(code(&output), 2, "{}", text(&output));
    assert!(text(&output).contains("timeout"));
    assert!(!s.dir.exists(), "invalid tests must not start a daemon");
}

#[test]
fn validates_managed_services_and_device_setup_without_running_commands() {
    let s = Sandbox::new();
    let dir = s.scratch("services-suite");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("test.yaml"),
        "name: managed\nsteps: [{action: app.open}]\n",
    )
    .unwrap();
    let setup = "version: 1\nenvironment:\n  emulator: medium_phone\n  apk: missing.apk\n  package: test.app\n  appLabel: Test\n  permissions: [android.permission.ACCESS_FINE_LOCATION]\n  launchActivity: test.app.MainActivity\n  reversePorts: [{device: 10101, host: 19101}]\nservices:\n  - name: backend\n    command: [does-not-exist]\n    cwd: backend\n    readyPort: 56551\n    timeout: 10s\ntests: [test.yaml]\n";
    std::fs::write(dir.join("suite.yaml"), setup).unwrap();
    let output = s.opendhu_env(
        &[
            "test",
            dir.join("suite.yaml").to_str().unwrap(),
            "--validate",
        ],
        &[("PATH", "/no/tools")],
    );
    assert_eq!(code(&output), 0, "{}", text(&output));
    std::fs::write(
        dir.join("suite.yaml"),
        setup.replace("host: 19101", "host: 0"),
    )
    .unwrap();
    let output = s.opendhu_env(
        &[
            "test",
            dir.join("suite.yaml").to_str().unwrap(),
            "--validate",
        ],
        &[("PATH", "/no/tools")],
    );
    assert_eq!(code(&output), 2);
    assert!(text(&output).contains("port"));
}

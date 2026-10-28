mod common;
use common::{Sandbox, code, text};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn executable(path: &Path, body: &str) {
    write(path, body);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn inherited_setup_preserves_declaring_paths_and_child_test_selection() {
    let s = Sandbox::new();
    let root = s.scratch("extends");
    write(
        &root.join("shared/setup.yaml"),
        "version: 1\nenvironment: {emulator: test_phone, apk: app.apk, package: example.app, appLabel: Example}\ntests: [base.yaml]\n",
    );
    write(
        &root.join("shared/base.yaml"),
        "name: Base\nsteps: [{action: wait, duration: 1ms}]\n",
    );
    write(
        &root.join("child/test.yaml"),
        "name: Child\nsteps: [{action: wait, duration: 1ms}]\n",
    );
    write(
        &root.join("child/suite.yaml"),
        "extends: ../shared/setup.yaml\nenvironment: {appLabel: Child}\ntests: [test.yaml]\n",
    );
    let o = s.opendhu(&[
        "test",
        root.join("child/suite.yaml").to_str().unwrap(),
        "--list",
        "--json",
    ]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(text(&o).contains("Child"));
    assert!(!text(&o).contains("Base"));
    assert!(!s.dir.exists(), "listing must not create a runtime");
}

#[test]
fn inheritance_rejects_cycles_missing_parents_and_unknown_fields_before_setup() {
    let s = Sandbox::new();
    let root = s.scratch("invalid-extends");
    write(&root.join("a.yaml"), "extends: b.yaml\n");
    write(&root.join("b.yaml"), "extends: a.yaml\n");
    for (source, expected) in [
        ("extends: b.yaml\n", "cycle"),
        ("extends: absent.yaml\n", "absent.yaml"),
        ("extends: b.yaml\ntypo: true\n", "unknown field"),
    ] {
        write(&root.join("a.yaml"), source);
        if source.contains("typo") {
            write(
                &root.join("b.yaml"),
                "version: 1\nenvironment: {emulator: test_phone, apk: app.apk, package: example.app, appLabel: Test}\ntests: [test.yaml]\n",
            );
            write(
                &root.join("test.yaml"),
                "name: Test\nsteps: [{action: wait, duration: 1ms}]\n",
            );
        }
        let o = s.opendhu(&["test", root.join("a.yaml").to_str().unwrap(), "--validate"]);
        assert_eq!(code(&o), 2, "{}", text(&o));
        assert!(text(&o).contains(expected), "{}", text(&o));
    }
    assert!(!s.dir.exists());
}

#[test]
fn suite_doctor_reports_inherited_apk_and_busy_port_without_starting_services() {
    let s = Sandbox::new();
    let root = s.scratch("doctor");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    write(
        &root.join("shared/setup.yaml"),
        &format!(
            "version: 1\nenvironment: {{emulator: test_phone, apk: missing.apk, package: example.app, appLabel: Test}}\nservices: [{{name: backend, command: [./start], readyPort: {port}}}]\n"
        ),
    );
    executable(
        &root.join("shared/start"),
        "#!/bin/sh\nprintf started > should-not-start\n",
    );
    write(
        &root.join("suite.yaml"),
        "extends: shared/setup.yaml\ntests: [test.yaml]\n",
    );
    write(
        &root.join("test.yaml"),
        "name: Test\nsteps: [{action: wait, duration: 1ms}]\n",
    );
    let o = s.opendhu_env(
        &[
            "doctor",
            root.join("suite.yaml").to_str().unwrap(),
            "--json",
        ],
        &[
            ("PATH", "/missing-tools"),
            ("ANDROID_HOME", "/missing-sdk"),
            ("ANDROID_SDK_ROOT", "/missing-sdk"),
            ("OPENDHU_NATIVE_HU", "/missing-hu"),
        ],
    );
    assert_eq!(code(&o), 3, "{}", text(&o));
    let checks: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let checks = checks.as_array().unwrap();
    assert!(checks.iter().any(|c| c["name"] == "apk"
        && c["level"] == "fail"
        && c["detail"].as_str().unwrap().contains("shared/missing.apk")));
    assert!(
        checks
            .iter()
            .any(|c| c["name"] == "service backend port" && c["level"] == "fail")
    );
    assert!(
        checks
            .iter()
            .any(|c| c["name"] == "service backend command" && c["level"] == "ok")
    );
    assert!(!root.join("shared/should-not-start").exists());
    assert!(!s.dir.exists());
    assert_eq!(listener.local_addr().unwrap().port(), port);
}

#[test]
fn named_suite_doctor_passes_for_available_avd_without_a_running_device_or_google_dhu() {
    let s = Sandbox::new();
    let root = s.scratch("doctor-ok");
    write(&root.join("opendhu.yaml"), "suites: {sample: suite.yaml}\n");
    write(
        &root.join("suite.yaml"),
        "version: 1\nenvironment: {emulator: test_phone, apk: app.apk, package: example.app, appLabel: Test}\ntests: [test.yaml]\n",
    );
    write(
        &root.join("test.yaml"),
        "name: Test\nsteps: [{action: wait, duration: 1ms}]\n",
    );
    write(&root.join("app.apk"), "apk");
    let bin = root.join("bin");
    executable(
        &bin.join("emulator"),
        "#!/bin/sh\n[ \"$*\" = '-list-avds' ] || exit 1\necho test_phone\n",
    );
    executable(
        &bin.join("tesseract"),
        "#!/bin/sh\n[ \"$*\" = '--list-langs' ] || exit 1\necho eng\n",
    );
    for name in ["adb", "ffmpeg", "opendhu-native-hu"] {
        executable(&bin.join(name), "#!/bin/sh\nexit 0\n");
    }
    let run = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_aa"))
            .args(["doctor", "sample", "--json"])
            .current_dir(&root)
            .env("PATH", &bin)
            .env("ANDROID_HOME", root.join("no-sdk"))
            .env("ANDROID_SDK_ROOT", root.join("no-sdk"))
            .env("XDG_CONFIG_HOME", root.join("no-config"))
            .env("OPENDHU_RUNTIME_DIR", &s.dir)
            .env("OPENDHU_NATIVE_HU", bin.join("opendhu-native-hu"))
            .env_remove("OPENDHU_CONFIG")
            .output()
            .unwrap()
    };
    let o = run();
    assert_eq!(code(&o), 0, "{}", text(&o));
    let checks: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert!(
        checks
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "avd" && c["level"] == "ok")
    );
    assert!(!s.dir.exists());
    std::fs::set_permissions(
        bin.join("opendhu-native-hu"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let o = run();
    assert_eq!(
        code(&o),
        3,
        "a non-executable native head unit must fail: {}",
        text(&o)
    );
    let checks: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert!(
        checks
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "native head unit" && c["level"] == "fail")
    );
}

#[test]
fn usb_doctor_ignores_avd_and_never_installs_or_launches_the_app() {
    let s = Sandbox::new();
    let root = s.scratch("doctor-usb");
    write(
        &root.join("suite.yaml"),
        "version: 1\nenvironment: {emulator: absent_avd, apk: app.apk, package: example.app, appLabel: Test}\ntests: [test.yaml]\n",
    );
    write(
        &root.join("test.yaml"),
        "name: Test\nsteps: [{action: wait, duration: 1ms}]\n",
    );
    write(&root.join("app.apk"), "apk");
    let env = s.adb_env("normal");
    let o = s.opendhu_with(
        &[
            "doctor",
            root.join("suite.yaml").to_str().unwrap(),
            "--usb",
            "--json",
        ],
        &env,
    );
    let checks: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let checks = checks.as_array().unwrap();
    assert!(
        checks
            .iter()
            .any(|c| c["name"] == "usb phone" && c["level"] == "ok")
    );
    assert!(!checks.iter().any(|c| c["name"] == "avd"));
    assert_eq!(s.adb_calls().trim(), "devices -l");
    assert!(!s.dir.exists());
}

#[test]
fn setup_gearhead_installs_a_supplied_apk_only_when_the_app_is_missing() {
    let s = Sandbox::new();
    let root = s.scratch("gearhead");
    write(&root.join("auto.apk"), "apk");
    let apk = root.join("auto.apk");
    let args = [
        "setup",
        "gearhead",
        "--apk",
        apk.to_str().unwrap(),
        "--serial",
        "FAKE0001",
    ];

    let o = s.opendhu_with(&args, &s.adb_env("no-gearhead"));
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(
        s.adb_calls()
            .contains(&format!("install -r {}", apk.display()))
    );

    let before = s.adb_calls();
    let o = s.opendhu_with(&args, &s.adb_env("normal"));
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(text(&o).contains("already installed"));
    assert_eq!(
        s.adb_calls().matches("install").count(),
        before.matches("install").count()
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn setup_gearhead_download_requires_a_pinned_version_and_checksum() {
    let s = Sandbox::new();
    let o = s.opendhu(&["setup", "gearhead", "--download"]);
    assert_eq!(code(&o), 2, "{}", text(&o));
}

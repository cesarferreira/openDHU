//! GUI launch through the CLI, using executables that need no display or phone.
mod common;

use common::*;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn isolated_cli(s: &Sandbox) -> PathBuf {
    let dir = s.scratch("gui-bin");
    std::fs::create_dir_all(&dir).unwrap();
    let cli = dir.join("aa");
    std::fs::copy(env!("CARGO_BIN_EXE_aa"), &cli).unwrap();
    cli
}

fn fake_gui(dir: &Path, script: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let gui = dir.join("opendhu-gui");
    std::fs::write(&gui, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(gui, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn launch(cli: &Path, path: &Path) -> Output {
    Command::new(cli)
        .arg("gui")
        .env("PATH", path)
        .env("OPENDHU_CONFIG", "/no/such/config.toml")
        .output()
        .unwrap()
}

#[test]
fn gui_prefers_the_sibling_and_preserves_its_exit_code() {
    let s = Sandbox::new();
    let cli = isolated_cli(&s);
    let path = s.scratch("path");
    fake_gui(cli.parent().unwrap(), "echo sibling-gui; exit 17");
    fake_gui(&path, "echo wrong-gui; exit 1");
    let output = launch(&cli, &path);
    assert_eq!(code(&output), 17, "{}", text(&output));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "sibling-gui"
    );
}

#[test]
fn gui_falls_back_to_path() {
    let s = Sandbox::new();
    let cli = isolated_cli(&s);
    let path = s.scratch("path");
    fake_gui(&path, "echo path-gui");
    let output = launch(&cli, &path);
    assert_eq!(code(&output), 0, "{}", text(&output));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "path-gui");
}

#[test]
fn missing_gui_explains_how_to_install_it() {
    let s = Sandbox::new();
    let cli = isolated_cli(&s);
    let output = launch(&cli, &s.scratch("empty-path"));
    assert_eq!(code(&output), 3, "{}", text(&output));
    assert!(
        text(&output).contains("make install-gui"),
        "{}",
        text(&output)
    );
}

#[test]
fn test_viewer_opens_by_default_uses_the_session_runtime_and_is_closed_after_the_run() {
    let s = Sandbox::new();
    let native = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../opendhu-native/tests/fixtures/fake-native-hu.sh"
    );
    let start = s.opendhu_env(
        &["start", "--backend", "native", "--serial", "FAKE0001"],
        &[("OPENDHU_NATIVE_HU", native)],
    );
    assert_eq!(code(&start), 0, "{}", text(&start));
    let cli = isolated_cli(&s);
    let record = s.scratch("viewer-runtime");
    let pid = s.scratch("viewer-pid");
    fake_gui(
        cli.parent().unwrap(),
        "printf '%s' \"$OPENDHU_RUNTIME_DIR\" > \"$VIEWER_RECORD\"\necho $$ > \"$VIEWER_PID\"\nexec /bin/sleep 30",
    );
    let file = s.scratch("viewer.yaml");
    std::fs::write(
        &file,
        "name: Visible test\nsteps: [{action: wait, duration: 500ms}]\n",
    )
    .unwrap();
    let output = Command::new(&cli)
        .args([
            "test",
            file.to_str().unwrap(),
            "--artifacts-dir",
            s.scratch("art").to_str().unwrap(),
        ])
        .env("OPENDHU_RUNTIME_DIR", &s.dir)
        .env_remove("OPENDHU_CONFIG")
        .env("XDG_CONFIG_HOME", s.dir.join("no-xdg"))
        .env("VIEWER_RECORD", &record)
        .env("VIEWER_PID", &pid)
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", text(&output));
    assert_eq!(
        std::fs::read_to_string(&record).unwrap(),
        s.dir.to_string_lossy()
    );
    let pid: u64 = std::fs::read_to_string(pid)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(!alive(pid), "the test-owned viewer must be stopped");
    assert_eq!(
        s.status_json()["backend"],
        "native",
        "standalone tests must leave the user's session running"
    );
}

#[test]
fn headless_test_does_not_require_or_start_a_viewer() {
    let s = Sandbox::new();
    let native = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../opendhu-native/tests/fixtures/fake-native-hu.sh"
    );
    let start = s.opendhu_env(
        &["start", "--backend", "native", "--serial", "FAKE0001"],
        &[("OPENDHU_NATIVE_HU", native)],
    );
    assert_eq!(code(&start), 0, "{}", text(&start));
    let cli = isolated_cli(&s);
    let file = s.scratch("headless.yaml");
    let art = s.scratch("art");
    std::fs::write(
        &file,
        "name: Headless test\nsteps: [{action: wait, duration: 10ms}]\n",
    )
    .unwrap();
    let output = Command::new(cli)
        .args([
            "test",
            file.to_str().unwrap(),
            "--headless",
            "--artifacts-dir",
            art.to_str().unwrap(),
        ])
        .env("OPENDHU_RUNTIME_DIR", &s.dir)
        .env_remove("OPENDHU_CONFIG")
        .env("XDG_CONFIG_HOME", s.dir.join("no-xdg"))
        .env("PATH", s.scratch("no-gui"))
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", text(&output));
    assert!(!art.join("head-unit-viewer.log").exists());
    assert!(text(&output).contains("PASS  1 test"), "{}", text(&output));
}

#[test]
fn contradictory_test_presentation_flags_are_rejected_before_startup() {
    let s = Sandbox::new();
    for flags in [
        ["--headless", "--show-head-unit"],
        ["--interactive", "--no-interactive"],
    ] {
        let output = s.opendhu(&["test", "missing.yaml", flags[0], flags[1]]);
        assert_eq!(code(&output), 2, "{}", text(&output));
        assert!(
            text(&output).contains("cannot be used with"),
            "{}",
            text(&output)
        );
    }
    assert!(!s.dir.exists());
}

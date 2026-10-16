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
    let gui = dir.join("autolab-gui");
    std::fs::write(&gui, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(gui, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn launch(cli: &Path, path: &Path) -> Output {
    Command::new(cli)
        .arg("gui")
        .env("PATH", path)
        .env("AUTOLAB_CONFIG", "/no/such/config.toml")
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

//! `aa tui` and `aa web` with and without a running session, using a fake viewer and the fake DHU.
mod common;

use common::*;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A copy of `aa` with a fake `opendhu-tui` next to it. The fake records the session it sees.
fn cli_with_fake_tui(s: &Sandbox, script: &str) -> PathBuf {
    let dir = s.scratch("viewer-bin");
    std::fs::create_dir_all(&dir).unwrap();
    let cli = dir.join("aa");
    std::fs::copy(env!("CARGO_BIN_EXE_aa"), &cli).unwrap();
    let tui = dir.join("opendhu-tui");
    std::fs::write(&tui, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(tui, std::fs::Permissions::from_mode(0o755)).unwrap();
    cli
}

fn run(s: &Sandbox, cli: &Path, args: &[&str], record: &Path) -> Output {
    Command::new(cli)
        .args(args)
        .env("OPENDHU_RUNTIME_DIR", &s.dir)
        .env("XDG_CONFIG_HOME", s.dir.join("no-xdg"))
        .env_remove("OPENDHU_CONFIG")
        .env_remove("OPENDHU_DHU_PATH")
        .env("VIEWER_RECORD", record)
        .env(
            "PATH",
            format!(
                "{}:{}",
                fake_adb_dir(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .output()
        .unwrap()
}

const RECORD_SESSION: &str =
    "\"$(dirname \"$0\")/aa\" session status --json > \"$VIEWER_RECORD\"\nexit 7";

fn has_session(s: &Sandbox) -> bool {
    code(&s.opendhu(&["session", "status"])) == 0
}

#[test]
fn tui_starts_a_session_when_none_is_running_and_stops_it_on_exit() {
    let s = Sandbox::new();
    let cli = cli_with_fake_tui(&s, RECORD_SESSION);
    let record = s.scratch("seen");
    let dhu = fake_dhu();
    let output = run(
        &s,
        &cli,
        &["tui", "--dhu", "--dhu-path", &dhu, "--serial", "FAKE0001"],
        &record,
    );
    assert_eq!(
        code(&output),
        7,
        "the viewer's exit code: {}",
        text(&output)
    );
    let seen: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&record).unwrap()).unwrap();
    assert_eq!(seen["backend"], "dhu", "the viewer saw the new session");
    assert!(
        text(&output).contains("Session stopped"),
        "{}",
        text(&output)
    );
    assert!(
        !has_session(&s),
        "the session started for the viewer must stop with it"
    );
}

#[test]
fn tui_attaches_to_a_running_session_and_leaves_it_running() {
    let s = Sandbox::new();
    let start = s.start_fake();
    assert_eq!(code(&start), 0, "{}", text(&start));
    let before = s.status_json()["session_id"].clone();
    let cli = cli_with_fake_tui(&s, RECORD_SESSION);
    let record = s.scratch("seen");
    let output = run(&s, &cli, &["tui"], &record);
    assert_eq!(code(&output), 7, "{}", text(&output));
    assert!(!text(&output).contains("starting one"), "{}", text(&output));
    assert_eq!(
        s.status_json()["session_id"],
        before,
        "the user's session keeps running"
    );
}

#[test]
fn start_options_are_ignored_with_a_note_when_a_session_is_running() {
    let s = Sandbox::new();
    let start = s.start_fake();
    assert_eq!(code(&start), 0, "{}", text(&start));
    let cli = cli_with_fake_tui(&s, "exit 0");
    let output = run(&s, &cli, &["tui", "--serial", "OTHER"], &s.scratch("seen"));
    assert_eq!(code(&output), 0, "{}", text(&output));
    assert!(
        text(&output).contains("start options are ignored"),
        "{}",
        text(&output)
    );
}

#[test]
fn a_missing_viewer_fails_before_starting_a_session() {
    let s = Sandbox::new();
    let dir = s.scratch("bare-bin");
    std::fs::create_dir_all(&dir).unwrap();
    let cli = dir.join("aa");
    std::fs::copy(env!("CARGO_BIN_EXE_aa"), &cli).unwrap();
    let dhu = fake_dhu();
    let output = Command::new(&cli)
        .args(["web", "--no-open", "--dhu", "--dhu-path", &dhu])
        .args(["--serial", "FAKE0001"])
        .env("OPENDHU_RUNTIME_DIR", &s.dir)
        .env("XDG_CONFIG_HOME", s.dir.join("no-xdg"))
        .env_remove("OPENDHU_CONFIG")
        .env("PATH", fake_adb_dir())
        .output()
        .unwrap();
    assert_eq!(code(&output), 3, "{}", text(&output));
    assert!(
        text(&output).contains("make install-web"),
        "{}",
        text(&output)
    );
    assert!(!has_session(&s), "no session without a viewer to show it");
}

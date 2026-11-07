//! The optional TOML config file, through the real binary and the fake DHU.

mod common;

use common::*;
use std::path::Path;
use std::process::{Command, Output};

fn write_config(s: &Sandbox, body: &str) -> String {
    // Next to the runtime dir, which must stay untouched until the daemon creates it with mode 0700.
    let dir = s.scratch("cfg");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, body).unwrap();
    path.display().to_string()
}

fn in_project(s: &Sandbox, cwd: &Path, xdg: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_aa"))
        .args(args)
        .current_dir(cwd)
        .env("OPENDHU_RUNTIME_DIR", &s.dir)
        .env("XDG_CONFIG_HOME", xdg)
        .env_remove("OPENDHU_CONFIG")
        .env_remove("OPENDHU_DHU_PATH")
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

#[test]
fn project_config_starts_dhu_with_its_relative_ini_and_user_dhu_path() {
    let s = Sandbox::new();
    let project = s.scratch("project");
    let nested = project.join("app");
    let xdg = s.scratch("xdg");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::create_dir_all(xdg.join("opendhu")).unwrap();
    std::fs::create_dir_all(project.join("head-unit")).unwrap();
    std::fs::write(
        project.join("head-unit/nissan_leaf.ini"),
        "[general]\nresolution = 1920x1080\n",
    )
    .unwrap();
    std::fs::write(
        xdg.join("opendhu/config.toml"),
        format!(
            "[dhu]\npath = \"{}\"\n[session]\nserial = \"FAKE0001\"",
            fake_dhu()
        ),
    )
    .unwrap();
    std::fs::write(
        project.join("opendhu.toml"),
        "[session]\nbackend = \"dhu\"\n[dhu]\nini = \"head-unit/nissan_leaf.ini\"\n",
    )
    .unwrap();
    let args_file = s.scratch("dhu-args");

    let o = Command::new(env!("CARGO_BIN_EXE_aa"))
        .args(["session", "start"])
        .current_dir(&nested)
        .env("OPENDHU_RUNTIME_DIR", &s.dir)
        .env("XDG_CONFIG_HOME", &xdg)
        .env("FAKE_DHU_ARGS_FILE", &args_file)
        .env_remove("OPENDHU_CONFIG")
        .env_remove("OPENDHU_DHU_PATH")
        .env(
            "PATH",
            format!(
                "{}:{}",
                fake_adb_dir(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .output()
        .unwrap();
    assert_eq!(code(&o), 0, "{}", text(&o));
    let args = std::fs::read_to_string(&args_file).unwrap();
    assert!(
        args.lines().any(|line| line
            == format!(
                "--config={}",
                project.join("head-unit/nissan_leaf.ini").display()
            )),
        "{args}"
    );
    assert_eq!(s.status_json()["serial"], "FAKE0001");
}

#[test]
fn doctor_lists_both_loaded_config_files() {
    let s = Sandbox::new();
    let project = s.scratch("project");
    let xdg = s.scratch("xdg");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(xdg.join("opendhu")).unwrap();
    std::fs::write(project.join("opendhu.toml"), "[session]\nbackend = \"dhu\"").unwrap();
    std::fs::write(
        xdg.join("opendhu/config.toml"),
        "[session]\nserial = \"FAKE0001\"",
    )
    .unwrap();

    let o = in_project(&s, &project, &xdg, &["doctor", "--dhu-path", &fake_dhu()]);
    assert!(text(&o).contains("config.toml"), "{}", text(&o));
    assert!(text(&o).contains("opendhu.toml"), "{}", text(&o));
}

#[test]
fn config_file_supplies_dhu_path_and_serial_so_start_needs_no_flags() {
    let s = Sandbox::new();
    let cfg = write_config(
        &s,
        &format!(
            "[dhu]\npath = \"{}\"\n[session]\nbackend = \"dhu\"\nserial = \"FAKE0001\"\n",
            fake_dhu()
        ),
    );
    let o = s.opendhu_env(&["session", "start"], &[("OPENDHU_CONFIG", &cfg)]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert_eq!(s.status_json()["serial"], "FAKE0001");
}

#[test]
fn flag_beats_config_file() {
    let s = Sandbox::new();
    let cfg = write_config(
        &s,
        &format!(
            "[dhu]\npath = \"{}\"\n[session]\nbackend = \"dhu\"\nserial = \"WRONG\"\n",
            fake_dhu()
        ),
    );
    let o = s.opendhu_env(
        &["session", "start", "--serial", "FAKE0001"],
        &[("OPENDHU_CONFIG", &cfg)],
    );
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert_eq!(s.status_json()["serial"], "FAKE0001");
}

#[test]
fn env_var_beats_config_file_for_the_dhu_path() {
    let s = Sandbox::new();
    let cfg = write_config(
        &s,
        "[dhu]\npath = \"/no/such/dhu\"\n[session]\nbackend = \"dhu\"\nserial = \"FAKE0001\"\n",
    );
    let o = s.opendhu_env(
        &["session", "start"],
        &[("OPENDHU_CONFIG", &cfg), ("OPENDHU_DHU_PATH", &fake_dhu())],
    );
    assert_eq!(code(&o), 0, "{}", text(&o));
}

#[test]
fn a_broken_config_file_fails_start_with_exit_2_but_not_stop_or_status() {
    let s = Sandbox::new();
    let cfg = write_config(&s, "[dhu]\npth = \"x\"\n");
    let o = s.opendhu_env(&["session", "start"], &[("OPENDHU_CONFIG", &cfg)]);
    assert_eq!(code(&o), 2, "{}", text(&o));
    assert!(text(&o).contains("config.toml"), "{}", text(&o));
    let st = s.opendhu_env(&["session", "status"], &[("OPENDHU_CONFIG", &cfg)]);
    assert_eq!(
        code(&st),
        3,
        "status must not read the config: {}",
        text(&st)
    );
}

#[test]
fn a_named_config_file_that_does_not_exist_is_an_error() {
    let s = Sandbox::new();
    let o = s.opendhu_env(
        &["session", "start"],
        &[("OPENDHU_CONFIG", "/no/such/config.toml")],
    );
    assert_eq!(code(&o), 2, "{}", text(&o));
}

#[test]
fn doctor_names_the_config_file_or_says_there_is_none() {
    let s = Sandbox::new();
    let none = s.opendhu(&["doctor", "--dhu-path", &fake_dhu()]);
    assert!(text(&none).contains("none loaded"), "{}", text(&none));
    let cfg = write_config(&s, "");
    let some = s.opendhu_env(
        &["doctor", "--dhu-path", &fake_dhu()],
        &[("OPENDHU_CONFIG", &cfg)],
    );
    assert!(text(&some).contains("config.toml"), "{}", text(&some));
    let bad = write_config(&s, "oops = 1");
    let o = s.opendhu_env(&["doctor", "--json"], &[("OPENDHU_CONFIG", &bad)]);
    assert_eq!(code(&o), 3, "{}", text(&o));
    assert!(text(&o).contains("\"config file\""), "{}", text(&o));
}

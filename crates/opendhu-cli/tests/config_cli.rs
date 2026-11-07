//! Project YAML configuration, through the real binary and the fake DHU.

mod common;

use common::*;
use std::path::Path;
use std::process::{Command, Output};

fn write_config(s: &Sandbox, body: &str) -> String {
    // Next to the runtime dir, which must stay untouched until the daemon creates it with mode 0700.
    let dir = s.scratch("cfg");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.yaml");
    std::fs::write(&path, body).unwrap();
    path.display().to_string()
}

fn in_project(s: &Sandbox, cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_aa"))
        .args(args)
        .current_dir(cwd)
        .env("OPENDHU_RUNTIME_DIR", &s.dir)
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
fn project_config_starts_dhu_with_its_relative_ini() {
    let s = Sandbox::new();
    let project = s.scratch("project");
    let nested = project.join("app");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::create_dir_all(project.join("head-unit")).unwrap();
    std::fs::write(
        project.join("head-unit/nissan_leaf.ini"),
        "[general]\nresolution = 1920x1080\n",
    )
    .unwrap();
    std::fs::write(
        project.join("opendhu.yaml"),
        format!("suites: {{robotaxi: e2e/robotaxi.yaml}}\nsession:\n  backend: dhu\n  serial: FAKE0001\ndhu:\n  path: {}\n  ini: head-unit/nissan_leaf.ini\n", fake_dhu()),
    )
    .unwrap();
    let args_file = s.scratch("dhu-args");

    let o = Command::new(env!("CARGO_BIN_EXE_aa"))
        .args(["session", "start"])
        .current_dir(&nested)
        .env("OPENDHU_RUNTIME_DIR", &s.dir)
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
fn doctor_names_the_project_config_file() {
    let s = Sandbox::new();
    let project = s.scratch("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("opendhu.yaml"), "session:\n  backend: dhu\n").unwrap();

    let o = in_project(&s, &project, &["doctor", "--dhu-path", &fake_dhu()]);
    assert!(text(&o).contains("opendhu.yaml"), "{}", text(&o));
}

#[test]
fn config_file_supplies_dhu_path_and_serial_so_start_needs_no_flags() {
    let s = Sandbox::new();
    let cfg = write_config(
        &s,
        &format!(
            "dhu:\n  path: {}\nsession:\n  backend: dhu\n  serial: FAKE0001\n",
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
            "dhu:\n  path: {}\nsession:\n  backend: dhu\n  serial: WRONG\n",
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
        "dhu:\n  path: /no/such/dhu\nsession:\n  backend: dhu\n  serial: FAKE0001\n",
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
    let cfg = write_config(&s, "dhu:\n  pth: x\n");
    let o = s.opendhu_env(&["session", "start"], &[("OPENDHU_CONFIG", &cfg)]);
    assert_eq!(code(&o), 2, "{}", text(&o));
    assert!(text(&o).contains("config.yaml"), "{}", text(&o));
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
        &[("OPENDHU_CONFIG", "/no/such/config.yaml")],
    );
    assert_eq!(code(&o), 2, "{}", text(&o));
}

#[test]
fn doctor_names_the_config_file_or_says_there_is_none() {
    let s = Sandbox::new();
    let empty = s.scratch("empty-project");
    std::fs::create_dir_all(&empty).unwrap();
    let none = in_project(&s, &empty, &["doctor", "--dhu-path", &fake_dhu()]);
    assert!(text(&none).contains("none loaded"), "{}", text(&none));
    let cfg = write_config(&s, "suites: {}\n");
    let some = s.opendhu_env(
        &["doctor", "--dhu-path", &fake_dhu()],
        &[("OPENDHU_CONFIG", &cfg)],
    );
    assert!(text(&some).contains("config.yaml"), "{}", text(&some));
    let bad = write_config(&s, "oops: 1\n");
    let o = s.opendhu_env(&["doctor", "--json"], &[("OPENDHU_CONFIG", &bad)]);
    assert_eq!(code(&o), 3, "{}", text(&o));
    assert!(text(&o).contains("\"config file\""), "{}", text(&o));
}

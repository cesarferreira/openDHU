mod common;
use common::*;
use std::path::Path;
use std::process::{Command, Output};

fn run(s: &Sandbox, cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_aa"))
        .args(args)
        .current_dir(cwd)
        .env("OPENDHU_RUNTIME_DIR", &s.dir)
        .env("XDG_CONFIG_HOME", s.dir.join("no-xdg"))
        .env_remove("OPENDHU_CONFIG")
        .output()
        .unwrap()
}

#[test]
fn named_suite_resolves_from_a_subdirectory_and_filters_before_device_setup() {
    let s = Sandbox::new();
    let root = s.scratch("project");
    std::fs::create_dir_all(root.join("e2e/tests")).unwrap();
    std::fs::write(
        root.join("opendhu.yaml"),
        "suites:\n  robotaxi: e2e/production.yaml\n  storybook: e2e/storybook.yaml\n",
    )
    .unwrap();
    std::fs::write(root.join("e2e/production.yaml"), "version: 1\nenvironment: {emulator: medium_phone, apk: absent.apk, package: test.app, appLabel: Test}\ntests: [tests/*.yaml]\n").unwrap();
    std::fs::write(
        root.join("e2e/tests/driver.yaml"),
        "name: Driver persistence\nsteps: [{action: app.open}]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("e2e/tests/stream.yaml"),
        "name: Backend stream update\nsteps: [{action: app.open}]\n",
    )
    .unwrap();
    let output = run(
        &s,
        &root.join("e2e"),
        &[
            "test",
            "robotaxi",
            "--filter",
            "PERSISTENCE",
            "--validate",
            "--json",
        ],
    );
    assert_eq!(code(&output), 0, "{}", text(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["tests"], 1);
    assert!(
        !s.dir.exists(),
        "selection/validation must not acquire devices"
    );
    let none = run(
        &s,
        &root,
        &["test", "robotaxi", "--filter", "absent", "--validate"],
    );
    assert_eq!(code(&none), 2, "{}", text(&none));
    assert!(text(&none).contains("matched no tests"), "{}", text(&none));
    let unknown = run(&s, &root, &["test", "typo", "--validate"]);
    assert_eq!(code(&unknown), 2);
    assert!(
        text(&unknown).contains("robotaxi") && text(&unknown).contains("storybook"),
        "{}",
        text(&unknown)
    );
}

#[test]
fn existing_paths_win_over_aliases_and_only_selected_tests_execute() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    let root = s.scratch("collection");
    std::fs::create_dir_all(root.join("robotaxi")).unwrap();
    std::fs::write(
        root.join("opendhu.yaml"),
        "suites: {robotaxi: missing.yaml}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("robotaxi/kept.yaml"),
        "name: Selected journey\nsteps: [{action: wait, duration: 10ms}]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("robotaxi/rejected.yaml"),
        "name: Must not execute\nbackend: native\nsteps: [{action: wait, duration: 10ms}]\n",
    )
    .unwrap();
    let art = s.scratch("art");
    let output = run(
        &s,
        &root,
        &[
            "test",
            "robotaxi",
            "--filter",
            "kept.yaml",
            "--artifacts-dir",
            art.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(code(&output), 0, "{}", text(&output));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["tests"].as_array().unwrap().len(), 1);
    assert_eq!(report["tests"][0]["name"], "Selected journey");
}

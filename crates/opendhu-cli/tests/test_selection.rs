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

fn registered_suites(s: &Sandbox) -> std::path::PathBuf {
    let root = s.scratch("registry");
    std::fs::create_dir_all(root.join("nested")).unwrap();
    std::fs::write(
        root.join("opendhu.yaml"),
        "suites:\n  robotaxi: robotaxi.yaml\n  drivers: drivers.yaml\n",
    )
    .unwrap();
    for name in ["robotaxi", "drivers"] {
        std::fs::write(root.join(format!("{name}.yaml")), format!("version: 1\nenvironment: {{emulator: medium_phone, apk: missing.apk, package: test.app, appLabel: Test}}\ntests: [{name}-test.yaml]\n")).unwrap();
        std::fs::write(
            root.join(format!("{name}-test.yaml")),
            format!("name: {name} journey\nsteps: [{{action: wait, duration: 1ms}}]\n"),
        )
        .unwrap();
    }
    root
}

#[test]
fn lists_registered_suites_from_ancestor_without_touching_devices() {
    let s = Sandbox::new();
    let root = registered_suites(&s);
    let output = run(&s, &root.join("nested"), &["test", "--list", "--json"]);
    assert_eq!(code(&output), 0, "{}", text(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["suites"][0]["name"], "drivers");
    assert_eq!(value["suites"][1]["name"], "robotaxi");
    assert_eq!(
        value["suites"][1]["path"],
        root.join("robotaxi.yaml")
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
    );
    assert!(!s.dir.exists());
    let text_output = run(&s, &root, &["test", "--list"]);
    assert_eq!(code(&text_output), 0, "{}", text(&text_output));
    assert!(text(&text_output).contains("drivers"));
    assert!(text(&text_output).contains("robotaxi.yaml"));
}

#[test]
fn lists_filtered_tests_in_a_named_suite_without_an_apk() {
    let s = Sandbox::new();
    let root = registered_suites(&s);
    let output = run(
        &s,
        &root,
        &[
            "test", "robotaxi", "--list", "--filter", "journey", "--json",
        ],
    );
    assert_eq!(code(&output), 0, "{}", text(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["tests"].as_array().unwrap().len(), 1);
    assert_eq!(value["tests"][0]["name"], "robotaxi journey");
    assert!(!s.dir.exists());
}

#[test]
fn validates_all_registered_suites_and_skips_suites_without_filter_matches() {
    let s = Sandbox::new();
    let root = registered_suites(&s);
    for (extra, count) in [(vec![], 2), (vec!["--filter", "robotaxi"], 1)] {
        let mut args = vec!["test", "all", "--usb", "--validate", "--json"];
        args.extend(extra);
        let output = run(&s, &root.join("nested"), &args);
        assert_eq!(code(&output), 0, "{}", text(&output));
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["valid"], true);
        assert_eq!(value["suites"].as_array().unwrap().len(), count);
        assert_eq!(value["tests"], count);
        assert!(!s.dir.exists());
    }
    let output = run(
        &s,
        &root,
        &["test", "all", "--filter", "absent", "--validate"],
    );
    assert_eq!(code(&output), 2);
    assert!(
        text(&output).contains("matched no tests"),
        "{}",
        text(&output)
    );
}

#[test]
fn registry_diagnostics_explain_missing_empty_and_nearest_project() {
    let s = Sandbox::new();
    let root = registered_suites(&s);
    std::fs::write(root.join("nested/opendhu.yaml"), "suites: {}\n").unwrap();
    let empty = run(&s, &root.join("nested"), &["test", "all", "--validate"]);
    assert_eq!(code(&empty), 2);
    assert!(text(&empty).contains("no suites"), "{}", text(&empty));
    std::fs::remove_file(root.join("opendhu.yaml")).unwrap();
    for args in [
        vec!["test", "--list"],
        vec!["test", "all"],
        vec!["test", "robotaxi"],
    ] {
        let output = run(&s, &root, &args);
        assert_eq!(code(&output), 2);
        assert!(text(&output).contains("opendhu.yaml"), "{}", text(&output));
    }
}

#[test]
fn all_usb_skips_unmatched_ordinary_tests_before_checking_device_compatibility() {
    let s = Sandbox::new();
    let root = registered_suites(&s);
    std::fs::write(
        root.join("opendhu.yaml"),
        "suites: {robotaxi: robotaxi.yaml, ordinary: ordinary.yaml}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("ordinary.yaml"),
        "name: Ordinary test\nsteps: [{action: wait, duration: 1ms}]\n",
    )
    .unwrap();
    let selected = run(
        &s,
        &root,
        &[
            "test",
            "all",
            "--usb",
            "--filter",
            "robotaxi",
            "--validate",
            "--json",
        ],
    );
    assert_eq!(code(&selected), 0, "{}", text(&selected));
    let value: serde_json::Value = serde_json::from_slice(&selected.stdout).unwrap();
    assert_eq!(value["suites"].as_array().unwrap().len(), 1);
    let unsupported = run(
        &s,
        &root,
        &["test", "all", "--usb", "--filter", "ordinary", "--validate"],
    );
    assert_eq!(code(&unsupported), 2, "{}", text(&unsupported));
    assert!(text(&unsupported).contains("--usb requires a managed suite"));
}

#[test]
fn list_all_and_global_filters_enumerate_tests_without_running_suites() {
    let s = Sandbox::new();
    let root = registered_suites(&s);
    for args in [
        vec!["test", "all", "--list", "--filter", "robotaxi", "--json"],
        vec!["test", "--list", "--filter", "robotaxi", "--json"],
    ] {
        let output = run(&s, &root, &args);
        assert_eq!(code(&output), 0, "{}", text(&output));
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["tests"].as_array().unwrap().len(), 1);
        assert_eq!(value["tests"][0]["name"], "robotaxi journey");
        assert!(!s.dir.exists());
    }
}

//! `autolab test` end to end against the fake DHU: real binary, real socket, real report files.

mod common;

use std::path::{Path, PathBuf};

use common::*;

fn example(name: &str) -> String {
    format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

fn json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

const FAILING: &str = "name: Phone must confirm\nsteps:\n  - action: focus.video\n    value: off\n  - action: assert.status\n    expect:\n      videoFocus:\n        observed: on\n";

#[test]
fn the_prd_example_passes_and_produces_the_documented_artifacts() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    let art = s.scratch("art");
    let o = s.autolab(&[
        "test",
        &example("video-focus.yaml"),
        "--artifacts-dir",
        art.to_str().unwrap(),
        "--report",
        "junit",
        "--report",
        "json",
    ]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let t = text(&o);
    assert!(
        t.contains("AutoLab Test Runner") && t.contains("Test: Video focus recovery"),
        "{t}"
    );
    assert!(t.contains("Result: PASSED"), "{t}");
    assert!(
        t.contains("1 test: 1 passed, 0 failed, 0 errors, 0 skipped"),
        "{t}"
    );

    for f in [
        "results.json",
        "results.xml",
        "metadata.json",
        "screenshots/restored.png",
    ] {
        assert!(art.join(f).is_file(), "missing {f}");
    }
    assert!(std::fs::read_dir(art.join("logs")).unwrap().count() == 1);
    let r = json(&art.join("results.json"));
    assert_eq!(r["exitCode"], 0);
    assert_eq!(r["tests"][0]["outcome"], "passed");
    assert_eq!(r["tests"][0]["steps"].as_array().unwrap().len(), 5); // preflight + 4
    let m = json(&art.join("metadata.json"));
    assert_eq!(m["backend"]["name"], "dhu");
    assert_eq!(m["backend"]["version"], "0.0-fake");
    assert_eq!(
        m["androidVersion"],
        serde_json::Value::Null,
        "unobservable values are null"
    );
    assert!(
        std::fs::read_to_string(art.join("results.xml"))
            .unwrap()
            .contains("<testsuite name=\"Video focus recovery\"")
    );
}

#[test]
fn a_failed_assertion_exits_7_and_still_restores_video_focus() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    let tests = s.scratch("tests");
    let file = write(&tests, "fail.yaml", FAILING);
    let art = s.scratch("art");
    let o = s.autolab(&[
        "test",
        file.to_str().unwrap(),
        "--artifacts-dir",
        art.to_str().unwrap(),
    ]);
    assert_eq!(code(&o), 7, "{}", text(&o));
    assert!(
        text(&o).contains("Result: FAILED") && text(&o).contains("expected on, actual unknown"),
        "{}",
        text(&o)
    );

    let r = json(&art.join("results.json"));
    assert_eq!(r["tests"][0]["outcome"], "failed");
    assert_eq!(r["exitCode"], 7);
    let steps = r["tests"][0]["steps"].as_array().unwrap();
    assert_eq!(steps.last().unwrap()["action"], "cleanup.focus.video");

    let st = s.status_json();
    assert_eq!(
        st["videoFocus"]["backendReported"], "on",
        "focus must not be left off"
    );
}

#[test]
fn a_schema_error_exits_2_without_touching_the_session_or_writing_artifacts() {
    let s = Sandbox::new(); // no session: a 3 would mean parsing happened too late
    let tests = s.scratch("tests");
    let bad = write(
        &tests,
        "typo.yaml",
        "name: t\nsteps:\n  - action: focus.video\n    value: off\n    vaule: on\n",
    );
    let art = s.scratch("art");
    let o = s.autolab(&[
        "test",
        bad.to_str().unwrap(),
        "--artifacts-dir",
        art.to_str().unwrap(),
    ]);
    assert_eq!(code(&o), 2, "{}", text(&o));
    assert!(
        text(&o).contains("typo.yaml") && text(&o).contains("vaule"),
        "{}",
        text(&o)
    );
    assert!(
        !art.exists(),
        "nothing may be written when input is invalid"
    );
}

#[test]
fn without_a_session_a_valid_test_exits_3() {
    let s = Sandbox::new();
    let o = s.autolab(&[
        "test",
        &example("video-focus.yaml"),
        "--artifacts-dir",
        s.scratch("art").to_str().unwrap(),
    ]);
    assert_eq!(code(&o), 3, "{}", text(&o));
}

#[test]
fn a_directory_runs_every_test_in_sorted_order_and_reports_the_worst_code() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    let tests = s.scratch("tests");
    write(&tests, "b_fail.yaml", FAILING);
    write(
        &tests.join("sub"),
        "a_pass.yml",
        "name: Quick pass\nsteps:\n  - action: wait\n    duration: 10ms\n",
    );
    write(&tests, "notes.txt", "ignored");
    let art = s.scratch("art");
    let o = s.autolab(&[
        "test",
        tests.to_str().unwrap(),
        "--artifacts-dir",
        art.to_str().unwrap(),
    ]);
    assert_eq!(code(&o), 7, "{}", text(&o));
    let r = json(&art.join("results.json"));
    let names: Vec<&str> = r["tests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["Phone must confirm", "Quick pass"],
        "sorted by path: b_fail.yaml before sub/"
    );
    assert_eq!(r["metadata"]["summary"]["passed"], 1);
    assert_eq!(r["metadata"]["summary"]["failed"], 1);
}

#[test]
fn a_timeout_exits_6() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    let tests = s.scratch("tests");
    let f = write(
        &tests,
        "slow.yaml",
        "name: Never confirmed\nsteps:\n  - action: waitFor\n    condition:\n      videoFocus:\n        observed: on\n    timeout: 1s\n",
    );
    let o = s.autolab(&[
        "test",
        f.to_str().unwrap(),
        "--artifacts-dir",
        s.scratch("art").to_str().unwrap(),
    ]);
    assert_eq!(code(&o), 6, "{}", text(&o));
}

#[test]
fn json_flag_prints_the_report_and_nothing_else_on_stdout() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    let o = s.autolab(&[
        "test",
        &example("screenshots.yaml"),
        "--json",
        "--artifacts-dir",
        s.scratch("art").to_str().unwrap(),
    ]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).expect("stdout must be pure JSON");
    assert_eq!(v["tests"][0]["outcome"], "passed");
}

#[test]
fn artifacts_dir_with_whitespace_is_refused_when_screenshots_are_used() {
    let s = Sandbox::new();
    let o = s.autolab(&[
        "test",
        &example("screenshots.yaml"),
        "--artifacts-dir",
        "/tmp/has space/out",
    ]);
    assert_eq!(code(&o), 2, "{}", text(&o));
    assert!(text(&o).contains("whitespace"), "{}", text(&o));
}

#[test]
fn an_old_report_in_another_format_is_not_left_behind() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    let art = s.scratch("art");
    let run = |fmt: &str| {
        s.autolab(&[
            "test",
            &example("focus-status.yaml"),
            "--report",
            fmt,
            "--artifacts-dir",
            art.to_str().unwrap(),
        ])
    };
    assert_eq!(code(&run("junit")), 0);
    assert!(art.join("results.xml").is_file());
    assert_eq!(code(&run("json")), 0);
    assert!(art.join("results.json").is_file());
    assert!(
        !art.join("results.xml").exists(),
        "stale results.xml would mislead CI"
    );
}

#[test]
fn missing_paths_and_empty_directories_are_argument_errors() {
    let s = Sandbox::new();
    assert_eq!(code(&s.autolab(&["test", "/no/such/test.yaml"])), 2);
    let empty = s.scratch("tests");
    std::fs::create_dir_all(&empty).unwrap();
    assert_eq!(code(&s.autolab(&["test", empty.to_str().unwrap()])), 2);
    assert_eq!(code(&s.autolab(&["test"])), 2, "a path is required");
}

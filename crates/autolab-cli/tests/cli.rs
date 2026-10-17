//! End-to-end CLI tests against a scripted fake DHU. They exercise AutoLab's own behaviour:
//! session ownership, exit codes, cleanup. Real-DHU behaviour is covered by `hardware.rs`.

mod common;

use std::time::Duration;

use common::*;

#[test]
fn full_lifecycle_start_focus_status_stop() {
    let s = Sandbox::new();
    let o = s.start_fake();
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(
        text(&o).contains("Backend:      dhu (0.0-fake, build fake-build-1)"),
        "{}",
        text(&o)
    );

    let off = s.autolab(&["focus", "video", "off"]);
    assert_eq!(code(&off), 0, "{}", text(&off));
    let t = text(&off);
    assert!(t.contains("Video focus off command accepted"), "{t}");
    assert!(t.contains("DHU reports:     off"), "{t}");
    assert!(t.contains("Phone confirmed: unknown"), "{t}");

    let st = s.status_json();
    assert_eq!(st["videoFocus"]["requested"], "off");
    assert_eq!(st["videoFocus"]["backendReported"], "off");
    assert_eq!(st["videoFocus"]["observed"], "unknown");
    assert_eq!(st["backend"], "dhu");

    let on = s.autolab(&["focus", "video", "on", "--json"]);
    assert_eq!(code(&on), 0);
    let r: serde_json::Value = serde_json::from_slice(&on.stdout).unwrap();
    assert_eq!(r["status"], "accepted");
    assert_eq!(r["requested"], "on");
    assert_eq!(r["backendReported"], "on");
    assert_eq!(r["observed"], "unknown");

    let (sup, dhu) = (
        st["daemonPid"].as_u64().unwrap(),
        st["dhuPid"].as_u64().unwrap(),
    );
    let stop = s.autolab(&["stop"]);
    assert_eq!(code(&stop), 0, "{}", text(&stop));
    assert!(
        wait_until(Duration::from_secs(5), || !alive(dhu) && !alive(sup)),
        "processes must be gone"
    );
    assert_eq!(code(&s.autolab(&["status"])), 3);
}

#[test]
fn focus_without_session_is_backend_unavailable() {
    let s = Sandbox::new();
    let o = s.autolab(&["focus", "video", "off"]);
    assert_eq!(code(&o), 3);
    assert!(text(&o).contains("aa start"), "{}", text(&o));
}

#[test]
fn second_start_is_a_session_conflict() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    let again = s.start_fake();
    assert_eq!(code(&again), 8, "{}", text(&again));
    assert_eq!(
        code(&s.autolab(&["status"])),
        0,
        "first session must be unharmed"
    );
}

#[test]
fn start_reports_dhu_startup_failure_with_its_output() {
    let s = Sandbox::new();
    let o = s.autolab_env(
        &[
            "start",
            "--dhu",
            "--dhu-path",
            &fake_dhu(),
            "--serial",
            "FAKE0001",
        ],
        &[("FAKE_DHU_MODE", "fail-start")],
    );
    assert_eq!(code(&o), 3, "{}", text(&o));
    assert!(
        text(&o).contains("Failed to start Google Automotive Link."),
        "{}",
        text(&o)
    );
    assert_eq!(
        text(&o).matches("backend unavailable").count(),
        1,
        "no doubled prefix: {}",
        text(&o)
    );
    assert_eq!(code(&s.autolab(&["status"])), 3);
}

#[test]
fn missing_dhu_is_backend_unavailable_and_lists_tried_paths() {
    let s = Sandbox::new();
    let o = s.autolab(&[
        "start",
        "--dhu",
        "--dhu-path",
        "/no/such/dhu",
        "--serial",
        "X",
    ]);
    assert_eq!(code(&o), 3);
    assert!(text(&o).contains("/no/such/dhu"), "{}", text(&o));
}

#[test]
fn invalid_arguments_exit_2() {
    let s = Sandbox::new();
    assert_eq!(code(&s.autolab(&["focus", "video", "sideways"])), 2);
    assert_eq!(
        code(&s.autolab(&["focus", "audio", "on"])),
        2,
        "audio focus is not implemented yet"
    );
}

#[test]
fn killed_dhu_ends_the_session_and_cleans_up() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    let st = s.status_json();
    let (sup, dhu) = (
        st["daemonPid"].as_u64().unwrap(),
        st["dhuPid"].as_u64().unwrap(),
    );
    std::process::Command::new("kill")
        .args(["-9", &dhu.to_string()])
        .status()
        .unwrap();
    assert!(
        wait_until(Duration::from_secs(5), || !alive(sup)),
        "the daemon must exit when the DHU dies (it was started for this one session)"
    );
    assert!(
        !s.dir.join("daemon.json").exists(),
        "daemon.json must be removed"
    );
    let o = s.autolab(&["focus", "video", "on"]);
    assert_eq!(
        code(&o),
        3,
        "no stale session may be reported as live: {}",
        text(&o)
    );
}

#[test]
fn stale_daemon_file_does_not_block_start() {
    let s = Sandbox::new();
    std::fs::create_dir_all(&s.dir).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&s.dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    // What a crashed daemon leaves behind: a daemon.json pointing at a port nobody listens on, with
    // the pid of a live process (this one). The pid must not be trusted.
    let stale = serde_json::json!({
        "port": 1, "pid": std::process::id(), "token": "old", "version": "0.0.0"
    });
    std::fs::write(s.dir.join("daemon.json"), stale.to_string()).unwrap();
    assert_eq!(
        code(&s.autolab(&["status"])),
        3,
        "a stale file is not a running session"
    );
    std::fs::write(s.dir.join("daemon.json"), stale.to_string()).unwrap();
    let o = s.start_fake();
    assert_eq!(code(&o), 0, "{}", text(&o));
    let now: serde_json::Value =
        serde_json::from_slice(&std::fs::read(s.dir.join("daemon.json")).unwrap()).unwrap();
    assert_ne!(
        now["token"], "old",
        "a new daemon must replace the stale file"
    );
}

#[test]
fn runtime_dir_with_loose_permissions_is_refused() {
    let s = Sandbox::new();
    std::fs::create_dir_all(&s.dir).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&s.dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let o = s.autolab(&["status"]);
    assert_eq!(code(&o), 1);
    assert!(text(&o).contains("0700"), "{}", text(&o));
}

#[test]
fn doctor_validates_the_dhu_and_flags_a_missing_one() {
    let s = Sandbox::new();
    let good = s.autolab(&["doctor", "--dhu-path", &fake_dhu()]);
    let t = text(&good);
    assert!(t.contains("dhu executable"), "{t}");
    assert!(t.contains("version 0.0-fake build fake-build-1"), "{t}");

    let bad = s.autolab(&["doctor", "--dhu-path", "/no/such/dhu", "--json"]);
    let checks: serde_json::Value = serde_json::from_slice(&bad.stdout).unwrap();
    assert!(
        checks
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "dhu executable" && c["level"] == "fail"),
        "{checks}"
    );
    assert_eq!(code(&bad), 3);
}

fn shot_file(s: &Sandbox) -> String {
    format!("{}-shot.png", s.dir.display())
}

#[test]
fn screenshot_saves_a_png_and_reports_its_absolute_path_and_size() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    let file = shot_file(&s);

    let o = s.autolab(&["screenshot", &file]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(
        text(&o).contains(&format!("Screenshot saved: {file} (1280x720)")),
        "{}",
        text(&o)
    );
    assert!(std::path::Path::new(&file).is_file());

    let j = s.autolab(&["screenshot", "--output", &file, "--json"]);
    assert_eq!(code(&j), 0, "{}", text(&j));
    let v: serde_json::Value = serde_json::from_slice(&j.stdout).unwrap();
    assert_eq!(v["path"], file.as_str());
    assert_eq!(
        (v["width"].as_u64(), v["height"].as_u64()),
        (Some(1280), Some(720))
    );
    let _ = std::fs::remove_file(&file);
}

#[test]
fn screenshot_argument_errors_exit_2_before_contacting_the_session() {
    let s = Sandbox::new(); // no session running: a 3 here would mean validation ran too late
    for args in [
        vec!["screenshot"],
        vec!["screenshot", "out.jpg"],
        vec!["screenshot", "has space.png"],
        vec!["screenshot", "/no/such/dir/x.png"],
        vec!["screenshot", "a.png", "--output", "b.png"],
    ] {
        let o = s.autolab(&args);
        assert_eq!(code(&o), 2, "{args:?}: {}", text(&o));
    }
}

#[test]
fn screenshot_without_a_session_is_backend_unavailable() {
    let s = Sandbox::new();
    let file = shot_file(&s);
    let o = s.autolab(&["screenshot", &file]);
    assert_eq!(code(&o), 3, "{}", text(&o));
    assert!(!std::path::Path::new(&file).exists());
}

#[test]
fn screenshot_the_dhu_accepts_but_never_writes_is_a_timeout() {
    let s = Sandbox::new();
    let started = s.autolab_env(
        &[
            "start",
            "--dhu",
            "--dhu-path",
            &fake_dhu(),
            "--serial",
            "FAKE0001",
        ],
        &[("FAKE_DHU_MODE", "screenshot-nofile")],
    );
    assert_eq!(code(&started), 0, "{}", text(&started));
    let file = shot_file(&s);
    let o = s.autolab(&["screenshot", &file]);
    assert_eq!(code(&o), 6, "{}", text(&o));
    assert!(text(&o).contains("did not appear"), "{}", text(&o));
    // Regression: the daemon once sent the already-prefixed Display text, so the client
    // printed "timed out: timed out: ...".
    assert_eq!(
        text(&o).matches("timed out").count(),
        1,
        "doubled prefix: {}",
        text(&o)
    );
}

#[test]
fn screenshot_without_video_focus_reports_the_dhu_refusal() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    assert_eq!(code(&s.autolab(&["focus", "video", "off"])), 0);
    let file = shot_file(&s);
    let o = s.autolab(&["screenshot", &file]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("nothing to screenshot"), "{}", text(&o));
    assert!(!std::path::Path::new(&file).exists());
}

#[test]
fn help_uses_the_aa_executable_name() {
    let s = Sandbox::new();
    let output = s.autolab(&["--help"]);
    assert_eq!(code(&output), 0, "{}", text(&output));
    assert!(text(&output).contains("Usage: aa "), "{}", text(&output));
}

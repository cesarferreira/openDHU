//! Hardware-in-the-loop test. Needs the real DHU, one authorized phone with Android Auto, and
//! nothing else holding the phone's USB accessory link. Excluded from plain `cargo test`:
//!
//!   AUTOLAB_HW=1 cargo test -p autolab-cli --test hardware -- --ignored --nocapture

mod common;

use common::*;

#[test]
#[ignore = "requires a real phone and the Google DHU; run with --ignored"]
fn real_dhu_video_focus_round_trip() {
    if std::env::var("AUTOLAB_HW").as_deref() != Ok("1") {
        eprintln!("skipped: set AUTOLAB_HW=1");
        return;
    }
    let s = Sandbox::new();
    // An arbitrary port, so the test never disturbs a tunnel the user relies on.
    let start = s.autolab(&["start", "--headless", "--reverse", "47123"]);
    assert_eq!(code(&start), 0, "{}", text(&start));
    // Independent of AutoLab: ask adb itself.
    let list = std::process::Command::new("adb")
        .args(["reverse", "--list"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&list.stdout).contains("tcp:47123 tcp:47123"),
        "adb does not list the tunnel: {}",
        String::from_utf8_lossy(&list.stdout)
    );

    // Includes repeated same-state calls: those exposed a stdout/stderr ordering bug once.
    // Simulate the phone dropping our (test-only) tunnel; AutoLab must put it back.
    let adb = |args: &[&str]| {
        std::process::Command::new("adb")
            .args(args)
            .output()
            .unwrap()
    };
    adb(&["reverse", "--remove", "tcp:47123"]);
    assert!(
        wait_until(std::time::Duration::from_secs(15), || {
            String::from_utf8_lossy(&adb(&["reverse", "--list"]).stdout)
                .contains("tcp:47123 tcp:47123")
        }),
        "AutoLab did not restore the dropped tunnel"
    );

    for arg in [
        "off", "on", "on", "off", "off", "on", "off", "on", "on", "off", "on", "on",
    ] {
        let want = arg;
        let o = s.autolab(&["focus", "video", arg, "--json"]);
        assert_eq!(code(&o), 0, "{}", text(&o));
        let r: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
        assert_eq!(r["status"], "accepted");
        assert_eq!(r["backendReported"], want);
        assert_eq!(
            r["observed"], "unknown",
            "phone confirmation is not implemented"
        );
    }
    let shot = format!("{}-shot.png", s.dir.display());
    let o = s.autolab(&["screenshot", &shot, "--json"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert!(v["width"].as_u64().unwrap() > 0 && v["height"].as_u64().unwrap() > 0);
    assert!(
        std::fs::metadata(&shot).unwrap().len() > 1000,
        "screenshot looks empty"
    );
    let _ = std::fs::remove_file(&shot);

    // Milestone 3: the YAML runner against the real session, checked through its JSON report.
    for example in ["video-focus.yaml", "focus-status.yaml", "screenshots.yaml"] {
        let art = s.scratch(&format!("art-{example}"));
        let path = format!("{}/../../examples/{example}", env!("CARGO_MANIFEST_DIR"));
        let o = s.autolab(&["test", &path, "--artifacts-dir", art.to_str().unwrap()]);
        assert_eq!(code(&o), 0, "{example}: {}", text(&o));
        let r: serde_json::Value =
            serde_json::from_slice(&std::fs::read(art.join("results.json")).unwrap()).unwrap();
        assert_eq!(r["tests"][0]["outcome"], "passed", "{example}");
        assert!(
            r["metadata"]["backend"]["version"].is_string(),
            "real DHU version recorded"
        );
    }
    let shot = s
        .scratch("art-video-focus.yaml")
        .join("screenshots/restored.png");
    assert!(
        std::fs::metadata(&shot).unwrap().len() > 1000,
        "runner screenshot looks empty"
    );

    let st = s.status_json();
    let dhu = st["dhuPid"].as_u64().unwrap();
    assert_eq!(code(&s.autolab(&["stop"])), 0);
    assert!(wait_until(std::time::Duration::from_secs(10), || !alive(
        dhu
    )));
}

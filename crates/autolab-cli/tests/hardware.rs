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
    let start = s.autolab(&["start", "--headless"]);
    assert_eq!(code(&start), 0, "{}", text(&start));

    // Includes repeated same-state calls: those exposed a stdout/stderr ordering bug once.
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

    let st = s.status_json();
    let dhu = st["dhuPid"].as_u64().unwrap();
    assert_eq!(code(&s.autolab(&["stop"])), 0);
    assert!(wait_until(std::time::Duration::from_secs(10), || !alive(
        dhu
    )));
}

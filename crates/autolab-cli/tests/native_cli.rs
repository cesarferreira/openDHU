//! `--backend native` through the CLI and the daemon, with a scripted fake head unit.

mod common;

use common::*;

fn fake_native() -> String {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../autolab-native/tests/fixtures/fake-native-hu.sh"
    )
    .to_string()
}

fn start_native(s: &Sandbox, extra: &[&str], env: &[(&str, &str)]) -> std::process::Output {
    let mut args = vec!["start", "--backend", "native", "--serial", "FAKE0001"];
    args.extend_from_slice(extra);
    let mut all = vec![("AUTOLAB_NATIVE_HU", fake_native())];
    all.extend(env.iter().map(|(k, v)| (*k, v.to_string())));
    let pairs: Vec<(&str, &str)> = all.iter().map(|(k, v)| (*k, v.as_str())).collect();
    s.autolab_env(&args, &pairs)
}

#[test]
fn native_lifecycle_reports_what_the_phone_said() {
    let s = Sandbox::new();
    let o = start_native(&s, &[], &[]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(
        text(&o).contains("Backend:      native (fake 0.0, build fake-build)"),
        "{}",
        text(&o)
    );

    let off = s.autolab(&["focus", "video", "off"]);
    assert_eq!(code(&off), 0, "{}", text(&off));
    let t = text(&off);
    assert!(t.contains("Video focus off confirmed by the phone"), "{t}");
    assert!(t.contains("Head unit sent:  off"), "{t}");
    assert!(t.contains("Phone confirmed: off"), "{t}");

    let st = s.status_json();
    assert_eq!(st["backend"], "native");
    assert_eq!(st["videoFocus"]["observed"], "off");

    let shot = s.scratch("native-shot.png");
    let refused = s.autolab(&["screenshot", &shot.display().to_string()]);
    assert_eq!(code(&refused), 1, "{}", text(&refused));
    assert!(
        text(&refused).contains("nothing to screenshot"),
        "{}",
        text(&refused)
    );

    let on = s.autolab(&["focus", "video", "on", "--json"]);
    assert_eq!(code(&on), 0);
    let v: serde_json::Value = serde_json::from_slice(&on.stdout).unwrap();
    assert_eq!(v["status"], "observed");
    assert_eq!(v["backend"], "native");
    assert_eq!(v["observed"], "on");

    let ok = s.autolab(&["screenshot", &shot.display().to_string()]);
    assert_eq!(code(&ok), 0, "{}", text(&ok));
    assert!(text(&ok).contains("(800x480)"), "{}", text(&ok));
    let _ = std::fs::remove_file(&shot);

    let (daemon, hu) = (
        st["daemonPid"].as_u64().unwrap(),
        st["dhuPid"].as_u64().unwrap(),
    );
    assert_eq!(code(&s.autolab(&["stop"])), 0);
    assert!(
        wait_until(std::time::Duration::from_secs(5), || !alive(hu)
            && !alive(daemon)),
        "head unit and daemon must be gone"
    );
}

#[test]
fn a_focus_change_the_phone_never_confirms_exits_6() {
    let s = Sandbox::new();
    assert_eq!(
        code(&start_native(
            &s,
            &[],
            &[("FAKE_NATIVE_MODE", "no-confirm")]
        )),
        0
    );
    let o = s.autolab(&["focus", "video", "off"]);
    assert_eq!(code(&o), 6, "{}", text(&o));
    assert!(
        text(&o).contains("not confirmed by the phone"),
        "{}",
        text(&o)
    );
}

#[test]
fn dhu_only_options_are_refused_for_the_native_backend() {
    let s = Sandbox::new();
    let o = start_native(&s, &["--dhu-path", &fake_dhu()], &[]);
    assert_eq!(code(&o), 2, "{}", text(&o));
    assert!(
        text(&o).contains("belong to the dhu backend"),
        "{}",
        text(&o)
    );
    let h = start_native(&s, &["--headless"], &[]);
    assert_eq!(code(&h), 2, "{}", text(&h));
}

#[test]
fn a_missing_native_head_unit_names_what_was_tried() {
    let s = Sandbox::new();
    let o = s.autolab_env(
        &["start", "--backend", "native", "--serial", "FAKE0001"],
        &[("AUTOLAB_NATIVE_HU", "/no/such/autolab-native-hu")],
    );
    assert_eq!(code(&o), 3, "{}", text(&o));
    assert!(
        text(&o).contains("/no/such/autolab-native-hu"),
        "{}",
        text(&o)
    );
}

#[test]
fn a_head_unit_that_dies_ends_the_session() {
    let s = Sandbox::new();
    assert_eq!(code(&start_native(&s, &[], &[])), 0);
    let st = s.status_json();
    let (daemon, hu) = (
        st["daemonPid"].as_u64().unwrap(),
        st["dhuPid"].as_u64().unwrap(),
    );
    std::process::Command::new("kill")
        .args(["-9", &hu.to_string()])
        .status()
        .unwrap();
    assert!(
        wait_until(std::time::Duration::from_secs(5), || !alive(daemon)),
        "the ephemeral daemon must exit when the head unit dies"
    );
    assert_eq!(code(&s.autolab(&["status"])), 3);
}

#[test]
fn the_config_file_can_choose_the_backend_and_its_dhu_settings_are_then_ignored() {
    let s = Sandbox::new();
    let dir = s.scratch("cfg");
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("config.toml");
    std::fs::write(
        &cfg,
        "[session]\nbackend = \"native\"\nserial = \"FAKE0001\"\n[dhu]\npath = \"/no/such/dhu\"\nheadless = true\n",
    )
    .unwrap();
    let o = s.autolab_env(
        &["start"],
        &[
            ("AUTOLAB_CONFIG", &cfg.display().to_string()),
            ("AUTOLAB_NATIVE_HU", &fake_native()),
        ],
    );
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert_eq!(s.status_json()["backend"], "native");
}

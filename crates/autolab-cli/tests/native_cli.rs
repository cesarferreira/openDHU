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

#[test]
fn tap_and_swipe_go_through_the_daemon_to_the_native_backend() {
    let s = Sandbox::new();
    assert_eq!(code(&start_native(&s, &[], &[])), 0);
    let tap = s.autolab(&["tap", "400", "240", "--json"]);
    assert_eq!(code(&tap), 0, "{}", text(&tap));
    let v: serde_json::Value = serde_json::from_slice(&tap.stdout).unwrap();
    assert_eq!(v["status"], "accepted");
    assert_eq!(v["backend"], "native");
    assert!(v["operationId"].as_str().unwrap().starts_with("op-"));

    let swipe = s.autolab(&["swipe", "10", "20", "700", "20", "--duration-ms", "500"]);
    assert_eq!(code(&swipe), 0, "{}", text(&swipe));
    assert!(
        text(&swipe).contains("does not acknowledge touch"),
        "{}",
        text(&swipe)
    );

    // The phone has stopped projecting: there is nothing to touch.
    assert_eq!(code(&s.autolab(&["focus", "video", "off"])), 0);
    let refused = s.autolab(&["tap", "1", "1"]);
    assert_eq!(code(&refused), 1, "{}", text(&refused));
    assert!(
        text(&refused).contains("nothing to touch"),
        "{}",
        text(&refused)
    );
}

#[test]
fn touch_without_a_session_is_backend_unavailable_and_bad_arguments_exit_2() {
    let s = Sandbox::new();
    assert_eq!(code(&s.autolab(&["tap", "1", "1"])), 3);
    assert_eq!(code(&s.autolab(&["tap", "-1", "1"])), 2);
    assert_eq!(code(&s.autolab(&["swipe", "1", "1"])), 2);
}

#[test]
fn the_dhu_backend_answers_touch_with_unsupported() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    let o = s.autolab(&["tap", "1", "1"]);
    assert_eq!(code(&o), 5, "{}", text(&o));
}

#[test]
fn key_presses_go_through_the_daemon_and_bad_names_list_the_valid_ones() {
    let s = Sandbox::new();
    assert_eq!(code(&s.autolab(&["key", "home"])), 3, "no session");
    let bad = s.autolab(&["key", "volume"]);
    assert_eq!(code(&bad), 2, "{}", text(&bad));
    assert!(
        text(&bad).contains("home") && text(&bad).contains("end_call"),
        "{}",
        text(&bad)
    );

    assert_eq!(code(&start_native(&s, &[], &[])), 0);
    let o = s.autolab(&["key", "home", "--json"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["status"], "accepted");
    assert_eq!(code(&s.autolab(&["key", "back", "--long"])), 0);
    assert_eq!(code(&s.autolab(&["focus", "video", "off"])), 0);
    let refused = s.autolab(&["key", "home"]);
    assert_eq!(code(&refused), 1, "{}", text(&refused));
}

#[test]
fn the_dhu_backend_answers_a_key_press_with_unsupported() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    assert_eq!(code(&s.autolab(&["key", "home"])), 5);
}

#[test]
fn audio_is_recorded_to_a_wav_with_a_summary_and_refused_without_a_native_session() {
    let s = Sandbox::new();
    let wav = s.scratch("rec.wav");
    let wav_arg = wav.display().to_string();
    assert_eq!(
        code(&s.autolab(&["audio", &wav_arg, "--seconds", "1"])),
        3,
        "no session"
    );
    assert_eq!(
        code(&s.autolab(&["audio", &wav_arg, "--channel", "music"])),
        2,
        "unknown channel"
    );

    assert_eq!(code(&s.start_fake()), 0);
    assert_eq!(
        code(&s.autolab(&["audio", &wav_arg, "--seconds", "1"])),
        5,
        "the DHU plays its own sound"
    );
    assert_eq!(code(&s.autolab(&["stop"])), 0);

    assert_eq!(code(&start_native(&s, &[], &[])), 0);
    let o = s.autolab(&["audio", &wav_arg, "--seconds", "2", "--json"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["channel"], "media");
    assert_eq!(v["format"], "s16le;rate=48000;channels=2");
    let bytes = v["bytes"].as_u64().unwrap();
    assert!(
        bytes >= 8 * 10,
        "the fake sends 8 bytes about 20 times a second: {v}"
    );
    let file = std::fs::read(&wav).unwrap();
    assert_eq!(&file[..4], b"RIFF");
    assert_eq!(
        file.len() as u64,
        44 + bytes,
        "header plus exactly the PCM that arrived"
    );
    let _ = std::fs::remove_file(&wav);
}

//! `--reverse` tunnels, tested against a fake adb that records every call.

mod common;

use common::*;

fn start_with(s: &Sandbox, env: &[(String, String)], extra: &[&str]) -> std::process::Output {
    let mut args = vec!["start", "--dhu", "--dhu-path", "", "--serial", "FAKE0001"];
    let dhu = fake_dhu();
    args[3] = &dhu;
    args.extend_from_slice(extra);
    s.opendhu_with(&args, env)
}

#[test]
fn reverse_tunnels_are_applied_confirmed_and_reported_live() {
    let s = Sandbox::new();
    let env = s.adb_env("normal");
    let o = start_with(&s, &env, &["--reverse", "10101", "--reverse", "8080"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(text(&o).contains("tcp:10101 (Active)"), "{}", text(&o));

    let st = s.opendhu_with(&["status", "--json"], &env);
    let v: serde_json::Value = serde_json::from_slice(&st.stdout).unwrap();
    let tunnels = v["tunnels"].as_array().unwrap();
    assert_eq!(tunnels.len(), 2);
    assert!(tunnels.iter().all(|t| t["state"] == "active"));

    let calls = s.adb_calls();
    assert!(
        calls.contains("-s FAKE0001 reverse tcp:10101 tcp:10101"),
        "{calls}"
    );
    assert!(
        calls.contains("-s FAKE0001 reverse tcp:8080 tcp:8080"),
        "{calls}"
    );
    assert!(
        !calls.contains("--remove"),
        "must never remove tunnels: {calls}"
    );
    assert!(!s.adb_state().join("removed.log").exists());
}

#[test]
fn status_reflects_the_device_not_the_request() {
    let s = Sandbox::new();
    let mut env = s.adb_env("normal");
    // Keep the keeper out of the way: this test is about what `status` reports, not about repairs.
    env.push(("OPENDHU_TUNNEL_CHECK_MS".into(), "600000".into()));
    assert_eq!(code(&start_with(&s, &env, &["--reverse", "10101"])), 0);
    // The phone re-enumerated and lost the tunnel behind openDHU's back.
    std::fs::remove_file(s.adb_state().join("reverse")).unwrap();
    let st = s.opendhu_with(&["status", "--json"], &env);
    let v: serde_json::Value = serde_json::from_slice(&st.stdout).unwrap();
    assert_eq!(v["tunnels"][0]["state"], "missing");
}

#[test]
fn rejected_tunnel_fails_start_and_leaves_no_session() {
    let s = Sandbox::new();
    let env = s.adb_env("reverse-fails");
    let o = start_with(&s, &env, &["--reverse", "10101"]);
    assert_eq!(code(&o), 4, "{}", text(&o));
    assert!(text(&o).contains("adb reverse tcp:10101"), "{}", text(&o));
    assert_eq!(
        code(&s.opendhu_with(&["status"], &env)),
        3,
        "no session may remain"
    );
}

#[test]
fn tunnel_adb_accepts_but_does_not_list_is_not_counted() {
    let s = Sandbox::new();
    let env = s.adb_env("lying-list");
    let o = start_with(&s, &env, &["--reverse", "10101"]);
    assert_eq!(code(&o), 4, "{}", text(&o));
    assert!(text(&o).contains("could not be confirmed"), "{}", text(&o));
    assert_eq!(code(&s.opendhu_with(&["status"], &env)), 3);
}

#[test]
fn phone_that_never_returns_to_adb_fails_within_the_bound() {
    let s = Sandbox::new();
    let mut env = s.adb_env("no-device");
    env.push(("OPENDHU_DEVICE_WAIT_SECS".into(), "1".into()));
    let started = std::time::Instant::now();
    let o = start_with(&s, &env, &["--reverse", "10101"]);
    assert_eq!(code(&o), 4, "{}", text(&o));
    assert!(
        text(&o).contains("did not see FAKE0001 again"),
        "{}",
        text(&o)
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
}

#[test]
fn invalid_ports_are_argument_errors() {
    let s = Sandbox::new();
    for bad in ["0", "70000", "abc", "-1"] {
        let o = start_with(&s, &[], &["--reverse", bad]);
        assert_eq!(code(&o), 2, "{bad}: {}", text(&o));
    }
}

#[test]
fn without_reverse_adb_is_never_called() {
    let s = Sandbox::new();
    let env = s.adb_env("normal");
    assert_eq!(code(&start_with(&s, &env, &[])), 0);
    assert_eq!(code(&s.opendhu_with(&["status"], &env)), 0);
    assert_eq!(s.adb_calls(), "", "no --reverse means no adb side effects");
}

// ---- keeping tunnels alive ---------------------------------------------------------------

use std::time::Duration;

fn fast_keeper(s: &Sandbox) -> Vec<(String, String)> {
    let mut env = s.adb_env("normal");
    env.push(("OPENDHU_TUNNEL_CHECK_MS".into(), "100".into()));
    env
}

fn reverse_calls(s: &Sandbox, port: u16) -> usize {
    s.adb_calls()
        .lines()
        .filter(|l| l.contains(&format!("reverse tcp:{port} tcp:{port}")))
        .count()
}

fn tunnel(s: &Sandbox, env: &[(String, String)]) -> serde_json::Value {
    let st = s.opendhu_with(&["status", "--json"], env);
    let v: serde_json::Value = serde_json::from_slice(&st.stdout).unwrap();
    v["tunnels"][0].clone()
}

#[test]
fn a_tunnel_the_phone_drops_is_put_back_and_counted() {
    let s = Sandbox::new();
    let env = fast_keeper(&s);
    assert_eq!(code(&start_with(&s, &env, &["--reverse", "10101"])), 0);
    assert_eq!(reverse_calls(&s, 10101), 1);

    // The phone re-enumerated and lost the tunnel.
    std::fs::remove_file(s.adb_state().join("reverse")).unwrap();
    assert!(
        wait_until(Duration::from_secs(10), || {
            let t = tunnel(&s, &env);
            t["state"] == "active" && t["reapplied"].as_u64() == Some(1)
        }),
        "tunnel was never restored: {}",
        tunnel(&s, &env)
    );
    assert_eq!(reverse_calls(&s, 10101), 2, "exactly one re-application");
}

#[test]
fn an_unreachable_phone_is_left_alone_until_it_comes_back() {
    let s = Sandbox::new();
    let env = fast_keeper(&s);
    assert_eq!(code(&start_with(&s, &env, &["--reverse", "10101"])), 0);

    std::fs::write(s.adb_state().join("offline"), "").unwrap();
    std::fs::remove_file(s.adb_state().join("reverse")).unwrap();
    std::thread::sleep(Duration::from_millis(800)); // ~8 checks while adb is failing
    assert_eq!(
        reverse_calls(&s, 10101),
        1,
        "must not hammer a phone that adb cannot reach"
    );

    std::fs::remove_file(s.adb_state().join("offline")).unwrap();
    assert!(
        wait_until(Duration::from_secs(10), || tunnel(&s, &env)["reapplied"]
            .as_u64()
            == Some(1)),
        "tunnel was not restored once the phone returned"
    );
}

#[test]
fn healthy_tunnels_are_never_touched() {
    let s = Sandbox::new();
    let env = fast_keeper(&s);
    assert_eq!(code(&start_with(&s, &env, &["--reverse", "10101"])), 0);
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(reverse_calls(&s, 10101), 1);
    assert_eq!(tunnel(&s, &env)["reapplied"], 0);
}

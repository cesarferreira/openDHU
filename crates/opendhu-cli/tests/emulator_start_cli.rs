//! Standalone startup must prepare the same emulator transport as a suite, and own its cleanup.
mod common;
use common::*;
use std::os::unix::fs::PermissionsExt;

fn fixture(s: &Sandbox, serial: &str) -> Vec<(String, String)> {
    let bin = s.scratch("tools");
    let state = s.scratch("state");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&state).unwrap();
    let adb = bin.join("adb");
    std::fs::write(&adb, r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$TEST_DEVICE_STATE/calls"
[ "$1" != '-s' ] || shift 2
case "$*" in
  'devices -l')
    printf 'List of devices attached\n%s device model:Test\n' "$TEST_DEVICE_SERIAL"
    if [ "${TEST_EXTRA_USB:-}" = yes ]; then printf 'PHONE1 device usb:1 model:Phone\nPHONE2 device usb:2 model:Phone\n'; fi ;;
  'shell am force-stop com.google.android.projection.gearhead') rm -f "$TEST_DEVICE_STATE/running" ;;
  'shell dumpsys activity services com.google.android.projection.gearhead')
    if [ -f "$TEST_DEVICE_STATE/running" ]; then echo 'ServiceRecord{test .companion.DeveloperHeadUnitNetworkService}'; fi ;;
  'shell am start -a com.google.android.projection.gearhead.SETTINGS') ;;
  'shell uiautomator dump '*) ;;
  'shell cat '*) echo '<hierarchy><node text="Start head unit server" bounds="[10,10][30,30]"/></hierarchy>' ;;
  'shell input tap 20 20') touch "$TEST_DEVICE_STATE/running" ;;
  'shell rm -f '*) ;;
  'forward tcp:0 tcp:5277') echo 43123 ;;
  'forward --remove tcp:43123') touch "$TEST_DEVICE_STATE/removed" ;;
  'reverse --list') ;;
  *) echo "unexpected adb invocation" >&2; exit 9 ;;
esac
"#).unwrap();
    std::fs::set_permissions(&adb, std::fs::Permissions::from_mode(0o755)).unwrap();
    let native = bin.join("native");
    std::fs::write(
        &native,
        format!(
            r#"#!/bin/sh
printf '%s' "${{OPENDHU_NATIVE_TCP:-USB}}" > "$TEST_DEVICE_STATE/transport"
exec '{}'
"#,
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../opendhu-native/tests/fixtures/fake-native-hu.sh"
            )
        ),
    )
    .unwrap();
    std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o755)).unwrap();
    vec![
        (
            "PATH".into(),
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        ),
        ("OPENDHU_NATIVE_HU".into(), native.display().to_string()),
        ("TEST_DEVICE_STATE".into(), state.display().to_string()),
        ("TEST_DEVICE_SERIAL".into(), serial.into()),
    ]
}

#[test]
fn standalone_emulator_start_bootstraps_tcp_and_stop_removes_its_forward() {
    let s = Sandbox::new();
    let env = fixture(&s, "emulator-5554");
    // No serial also covers the GUI's automatic single-device choice.
    let output = s.opendhu_with(&["session", "start", "--backend", "native"], &env);
    assert_eq!(code(&output), 0, "{}", text(&output));
    assert_eq!(s.status_json()["transport"], "tcp");
    let state = s.scratch("state");
    assert_eq!(
        std::fs::read_to_string(state.join("transport")).unwrap(),
        "127.0.0.1:43123"
    );
    let calls = std::fs::read_to_string(state.join("calls")).unwrap();
    assert!(
        calls.contains("-s emulator-5554 shell input tap 20 20"),
        "{calls}"
    );
    assert!(
        calls.contains("-s emulator-5554 forward tcp:0 tcp:5277"),
        "{calls}"
    );
    assert!(
        !state.join("removed").exists(),
        "forward must stay alive after the start CLI exits"
    );
    let stop = s.opendhu(&["session", "stop"]);
    assert_eq!(code(&stop), 0, "{}", text(&stop));
    assert!(wait_until(std::time::Duration::from_secs(3), || state
        .join("removed")
        .exists()));
}

#[test]
fn a_failed_emulator_backend_start_removes_the_allocated_forward() {
    let s = Sandbox::new();
    let mut env = fixture(&s, "emulator-5554");
    env.push(("FAKE_NATIVE_MODE".into(), "exit-early".into()));
    let output = s.opendhu_with(&["session", "start", "--serial", "emulator-5554"], &env);
    assert_ne!(code(&output), 0, "{}", text(&output));
    assert!(
        s.scratch("state").join("removed").exists(),
        "failed startup must release its forward: {}",
        text(&output)
    );
}

#[test]
fn explicit_tcp_and_physical_phones_keep_their_requested_transports() {
    for (serial, args, expected) in [
        (
            "emulator-5554",
            vec![
                "session",
                "start",
                "--serial",
                "emulator-5554",
                "--native-tcp",
                "127.0.0.1:15277",
            ],
            "127.0.0.1:15277",
        ),
        (
            "PHONE123",
            vec!["session", "start", "--serial", "PHONE123"],
            "USB",
        ),
    ] {
        let s = Sandbox::new();
        let env = fixture(&s, serial);
        let output = s.opendhu_with(&args, &env);
        assert_eq!(code(&output), 0, "{}", text(&output));
        let state = s.scratch("state");
        assert_eq!(
            std::fs::read_to_string(state.join("transport")).unwrap(),
            expected
        );
        assert!(
            !std::fs::read_to_string(state.join("calls"))
                .unwrap_or_default()
                .contains("forward tcp:0")
        );
    }
}

#[test]
fn an_explicit_emulator_does_not_require_unplugging_unrelated_usb_phones() {
    let s = Sandbox::new();
    let mut env = fixture(&s, "emulator-5554");
    env.push(("TEST_EXTRA_USB".into(), "yes".into()));
    let output = s.opendhu_with(&["session", "start", "--serial", "emulator-5554"], &env);
    assert_eq!(code(&output), 0, "{}", text(&output));
    assert_eq!(s.status_json()["transport"], "tcp");
}

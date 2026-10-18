//! Suite validation must be useful in CI before an APK or Android device exists.
mod common;
use common::*;

static USB_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn validates_suite_relative_globs_and_ui_actions_without_a_device_or_apk() {
    let s = Sandbox::new();
    let dir = s.scratch("suite");
    std::fs::create_dir_all(dir.join("tests")).unwrap();
    std::fs::write(dir.join("suite.yaml"), "version: 1\nenvironment:\n  emulator: medium_phone\n  apk: missing.apk\n  package: ai.wayve.robotui.storybook\n  appLabel: RobotUI Storybook\ntests: [tests/*.yaml]\n").unwrap();
    std::fs::write(dir.join("tests/catalog.yaml"), "name: catalog\nbackend: native\nsteps:\n  - action: app.open\n  - action: ui.waitFor\n    text: RobotUI Storybook\n    timeout: 15s\n").unwrap();
    let output = s.opendhu_env(
        &[
            "test",
            dir.join("suite.yaml").to_str().unwrap(),
            "--validate",
            "--show-head-unit",
            "--json",
        ],
        &[("PATH", "/no/tools")],
    );
    assert_eq!(code(&output), 0, "{}", text(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["valid"], true);
    assert_eq!(value["tests"], 1);
    assert!(!s.dir.exists(), "validation must not start a daemon");
}

#[test]
fn invalid_suite_test_is_rejected_before_device_setup() {
    let s = Sandbox::new();
    let dir = s.scratch("suite");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("suite.yaml"), "version: 1\nenvironment: {emulator: medium_phone, apk: missing.apk, package: test.app, appLabel: Test}\ntests: [bad.yaml]\n").unwrap();
    std::fs::write(
        dir.join("bad.yaml"),
        "name: bad\nsteps:\n  - action: ui.expect\n    text: ''\n    timeout: 0s\n",
    )
    .unwrap();
    let output = s.opendhu_env(
        &["test", dir.join("suite.yaml").to_str().unwrap()],
        &[("PATH", "/no/tools")],
    );
    assert_eq!(code(&output), 2, "{}", text(&output));
    assert!(text(&output).contains("timeout"));
    assert!(!s.dir.exists(), "invalid tests must not start a daemon");
}

#[test]
fn validates_managed_services_and_device_setup_without_running_commands() {
    let s = Sandbox::new();
    let dir = s.scratch("services-suite");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("test.yaml"),
        "name: managed\nsteps: [{action: app.open}]\n",
    )
    .unwrap();
    let setup = "version: 1\nenvironment:\n  emulator: medium_phone\n  apk: missing.apk\n  package: test.app\n  appLabel: Test\n  permissions: [android.permission.ACCESS_FINE_LOCATION]\n  launchActivity: test.app.MainActivity\n  reversePorts: [{device: 10101, host: 19101}]\nservices:\n  - name: backend\n    command: [does-not-exist]\n    cwd: backend\n    readyPort: 56551\n    timeout: 10s\ntests: [test.yaml]\n";
    std::fs::write(dir.join("suite.yaml"), setup).unwrap();
    let output = s.opendhu_env(
        &[
            "test",
            dir.join("suite.yaml").to_str().unwrap(),
            "--validate",
        ],
        &[("PATH", "/no/tools")],
    );
    assert_eq!(code(&output), 0, "{}", text(&output));
    std::fs::write(
        dir.join("suite.yaml"),
        setup.replace("host: 19101", "host: 0"),
    )
    .unwrap();
    let output = s.opendhu_env(
        &[
            "test",
            dir.join("suite.yaml").to_str().unwrap(),
            "--validate",
        ],
        &[("PATH", "/no/tools")],
    );
    assert_eq!(code(&output), 2);
    assert!(text(&output).contains("port"));
}

struct UsbSuite {
    sandbox: Sandbox,
    suite: std::path::PathBuf,
    artifacts: std::path::PathBuf,
    env: Vec<(String, String)>,
}

impl UsbSuite {
    fn new(devices: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let sandbox = Sandbox::new();
        let dir = sandbox.scratch("usb-suite");
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let suite = dir.join("suite.yaml");
        std::fs::write(&suite, "version: 1\nenvironment:\n  emulator: unavailable_avd\n  apk: app.apk\n  package: test.app\n  appLabel: Test\n  launchActivity: test.app.MainActivity\ntests: [test.yaml]\n").unwrap();
        std::fs::write(dir.join("app.apk"), "fake APK").unwrap();
        std::fs::write(
            dir.join("test.yaml"),
            "name: USB journey\nbackend: native\nsteps: [{action: wait, duration: 1ms}]\n",
        )
        .unwrap();
        std::fs::write(dir.join("devices"), devices).unwrap();
        for (name, script) in [
            (
                "adb",
                r#"#!/bin/sh
printf '%s\n' "$*" >> "$USB_FIXTURE/calls"
if [ "$1" = devices ]; then
  if [ "$EMULATOR_FIXTURE" = 1 ]; then
    echo 'List of devices attached'
    [ ! -f "$USB_FIXTURE/emulator-running" ] || echo 'emulator-5554 device'
    [ ! -f "$USB_FIXTURE/other-running" ] || echo 'emulator-5556 device'
  else cat "$USB_FIXTURE/devices"; fi
  exit 0
fi
[ "$1" = -s ] && { [ "$2" = PHONE ] || { [ "$EMULATOR_FIXTURE" = 1 ] && { [ "$2" = emulator-5554 ] || [ "$2" = emulator-5556 ]; }; }; } || exit 9
fixture_serial=$2
shift 2
if [ "$1" = reverse ] && [ "$EMULATOR_FIXTURE" != 1 ]; then
  [ -f "$USB_FIXTURE/native-pid" ] && kill -0 "$(cat "$USB_FIXTURE/native-pid")" 2>/dev/null || {
    echo 'ADB disconnected after USB projection stopped' >&2; exit 9
  }
fi
case "$*" in
  'emu avd name')
    if [ "$fixture_serial" = emulator-5556 ]; then cat "$USB_FIXTURE/other-avd"; else cat "$USB_FIXTURE/avd-name"; fi;;
  'emu kill')
    if [ "$fixture_serial" = emulator-5556 ]; then prefix=other; else prefix=emulator; fi
    rm -f "$USB_FIXTURE/$prefix-running"; kill "$(cat "$USB_FIXTURE/$prefix-pid")";;
  'shell getprop sys.boot_completed') echo 1;;
  'shell am force-stop com.google.android.projection.gearhead') rm -f "$USB_FIXTURE/server-running";;
  'shell dumpsys activity services com.google.android.projection.gearhead')
    [ ! -f "$USB_FIXTURE/server-running" ] || echo 'ServiceRecord{.companion.DeveloperHeadUnitNetworkService}' ;;
  'shell am start -a com.google.android.projection.gearhead.SETTINGS') touch "$USB_FIXTURE/server-running";;
  'forward tcp:0 tcp:5277') echo 37123;;
  'forward --remove tcp:37123')
    if [ "$EMULATOR_REPLACE" = 1 ] && [ ! -f "$USB_FIXTURE/emulator-replaced" ]; then
      kill -KILL "$(cat "$USB_FIXTURE/emulator-pid")"
      touch "$USB_FIXTURE/emulator-replaced"
      echo external > "$USB_FIXTURE/emulator-running"
    fi;;
  'install -r -t '*) rm -f "$USB_FIXTURE/log-pipe" "$USB_FIXTURE/monitor-ready";;
  'shell am start -W -n test.app/test.app.MainActivity')
    if [ -n "$USB_LAUNCH_CRASH" ] && [ -p "$USB_FIXTURE/log-pipe" ]; then
      printf '%s\n' "$USB_LAUNCH_CRASH" > "$USB_FIXTURE/log-pipe"
    fi;;
  'shell am force-stop test.app') ;;
  'get-state')
    if [ "$USB_DELAY_ADB" = 1 ] && [ ! -f "$USB_FIXTURE/adb-returned" ]; then
      touch "$USB_FIXTURE/adb-returned"
      echo 'error: device offline' >&2; exit 1
    fi
    echo device;;
  'reverse --list') [ ! -f "$USB_FIXTURE/reverse" ] || cat "$USB_FIXTURE/reverse";;
  'reverse --no-rebind tcp:10101 tcp:19101')
    [ -f "$USB_FIXTURE/native" ] || { echo 'reverse created before USB projection' >&2; exit 9; }
    echo 'UsbFfs tcp:10101 tcp:19101' > "$USB_FIXTURE/reverse";;
  'reverse --remove tcp:10101') rm "$USB_FIXTURE/reverse";;
  'logcat -T 1'|'logcat -b main -b system -b crash -b events -v threadtime -T 1 *:V')
    [ -f "$USB_FIXTURE/native" ] || exit 9
    echo $$ > "$USB_FIXTURE/log-pid"
    rm -f "$USB_FIXTURE/log-pipe" "$USB_FIXTURE/monitor-ready"
    mkfifo "$USB_FIXTURE/log-pipe"
    exec 3<>"$USB_FIXTURE/log-pipe"
    echo 'capturing PHONE'
    exec cat <&3;;
  'shell log -t OpenDHUHealth '*)
    n=0; while [ ! -p "$USB_FIXTURE/log-pipe" ] && [ "$n" -lt 100 ]; do sleep 0.01; n=$((n+1)); done
    if [ "$USB_DROP_FIRST_MARKER" = 1 ] && [ ! -f "$USB_FIXTURE/marker-dropped" ]; then
      touch "$USB_FIXTURE/marker-dropped"; exit 0
    fi
    if [ -f "$USB_FIXTURE/monitor-ready" ]; then
      [ "$USB_MONITOR_STALL" != 1 ] || exit 0
      [ -z "$USB_CRASH_LOG" ] || printf '%s\n' "$USB_CRASH_LOG" > "$USB_FIXTURE/log-pipe"
      [ "$USB_MONITOR_EXIT" != 1 ] || { kill "$(cat "$USB_FIXTURE/log-pid")"; exit 0; }
    fi
    echo $$ > "$USB_FIXTURE/marker-pid"
    printf '10-17 23:08:00.000  100  100 I OpenDHUHealth: %s\n' "$5" > "$USB_FIXTURE/log-pipe"
    touch "$USB_FIXTURE/monitor-ready";;
  *) echo "unexpected adb call: $*" >&2; exit 9;;
esac
"#,
            ),
            ("tesseract", "#!/bin/sh\nexit 0\n"),
            (
                "emulator",
                "#!/bin/sh\necho emulator >> \"$USB_FIXTURE/calls\"\nexit 9\n",
            ),
            (
                "native-hu",
                r#"#!/bin/sh
printf 'tcp=%s\n' "${OPENDHU_NATIVE_TCP-unset}" >> "$USB_FIXTURE/native"
printf '%s\n' "$OPENDHU_RUNTIME_DIR" > "$USB_FIXTURE/runtime"
echo $$ > "$USB_FIXTURE/native-pid"
exec "$USB_FAKE_NATIVE"
"#,
            ),
        ] {
            let path = bin.join(name);
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let env = vec![
            ("USB_FIXTURE".into(), dir.display().to_string()),
            (
                "USB_FAKE_NATIVE".into(),
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../opendhu-native/tests/fixtures/fake-native-hu.sh"
                )
                .into(),
            ),
            (
                "OPENDHU_NATIVE_HU".into(),
                bin.join("native-hu").display().to_string(),
            ),
            ("PATH".into(), format!("{}:/usr/bin:/bin", bin.display())),
            (
                "ANDROID_HOME".into(),
                dir.join("missing-sdk").display().to_string(),
            ),
        ];
        Self {
            artifacts: dir.join("artifacts"),
            sandbox,
            suite,
            env,
        }
    }

    fn emulator_fixture() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let mut fixture = Self::new("List of devices attached\n");
        let root = fixture.suite.parent().unwrap();
        let name = format!("test_{}", root.file_name().unwrap().to_string_lossy());
        std::fs::write(root.join("avd-name"), &name).unwrap();
        let suite = std::fs::read_to_string(&fixture.suite)
            .unwrap()
            .replace("unavailable_avd", &name);
        std::fs::write(&fixture.suite, suite).unwrap();
        let emulator = root.join("bin/emulator");
        std::fs::write(
            &emulator,
            r#"#!/bin/sh
echo "boot:$2" >> "$USB_FIXTURE/emulator-events"
echo "$*" >> "$USB_FIXTURE/emulator-args"
if [ "$2" = "$(cat "$USB_FIXTURE/avd-name")" ]; then prefix=emulator; else
  prefix=other; echo "$2" > "$USB_FIXTURE/other-avd"
fi
echo $$ > "$USB_FIXTURE/$prefix-pid"
touch "$USB_FIXTURE/$prefix-running"
exec sleep 120
"#,
        )
        .unwrap();
        std::fs::set_permissions(emulator, std::fs::Permissions::from_mode(0o755)).unwrap();
        fixture.env.push(("EMULATOR_FIXTURE".into(), "1".into()));
        fixture
    }

    fn run(&self, extra: &[&str]) -> std::process::Output {
        let mut args = vec![
            "test",
            self.suite.to_str().unwrap(),
            "--usb",
            "--headless",
            "--no-interactive",
            "--artifacts-dir",
            self.artifacts.to_str().unwrap(),
        ];
        args.extend_from_slice(extra);
        // Successful USB fixtures share the native transport's global lease.
        let _guard = USB_TEST.lock().unwrap();
        self.sandbox.opendhu_with(&args, &self.env)
    }
    fn register_all(&self) {
        let root = self.suite.parent().unwrap();
        std::fs::copy(&self.suite, root.join("second.yaml")).unwrap();
        std::fs::write(
            root.join("opendhu.yaml"),
            "suites: {first: suite.yaml, second: second.yaml}\n",
        )
        .unwrap();
    }

    fn all_command(&self, extra: &[&str]) -> std::process::Command {
        self.registered_command(&["all"], extra)
    }

    fn registered_command(&self, target: &[&str], extra: &[&str]) -> std::process::Command {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_aa"));
        cmd.current_dir(self.suite.parent().unwrap())
            .arg("test")
            .args(target)
            .args(
                if self.env.iter().any(|(key, _)| key == "EMULATOR_FIXTURE") {
                    &[][..]
                } else {
                    &["--usb"][..]
                },
            )
            .args([
                "--headless",
                "--no-interactive",
                "--artifacts-dir",
                self.artifacts.to_str().unwrap(),
            ])
            .args(extra)
            .env("OPENDHU_RUNTIME_DIR", &self.sandbox.dir)
            .env("XDG_CONFIG_HOME", self.sandbox.dir.join("no-xdg"))
            .env_remove("OPENDHU_CONFIG");
        for (key, value) in &self.env {
            cmd.env(key, value);
        }
        cmd
    }
}

#[test]
fn usb_overrides_emulator_runs_suite_and_cleans_up_only_its_session() {
    let mut s = UsbSuite::new(
        "List of devices attached\nemulator-5554 device model:AVD\nPHONE device usb:1 model:Pixel\n192.168.1.2:5555 device model:Wireless\n",
    );
    s.env.push(("USB_DELAY_ADB".into(), "1".into()));
    let output = s.run(&[]);
    assert_eq!(code(&output), 0, "{}", text(&output));
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(s.artifacts.join("suite.json")).unwrap()).unwrap();
    assert_eq!(metadata["serial"], "PHONE");
    assert_eq!(metadata["transport"], "usb");
    assert!(metadata["emulator"].is_null());
    assert!(s.artifacts.join("results.json").is_file());
    assert!(
        std::fs::read_to_string(s.artifacts.join("logcat.txt"))
            .unwrap()
            .contains("capturing PHONE")
    );
    let dir = s.suite.parent().unwrap();
    let calls = std::fs::read_to_string(dir.join("calls")).unwrap();
    assert!(calls.contains("-s PHONE install -r -t"), "{calls}");
    assert!(calls.matches("-s PHONE get-state").count() >= 2, "{calls}");
    assert!(
        calls.contains("-s PHONE shell am force-stop test.app"),
        "{calls}"
    );
    let runtime = std::fs::read_to_string(dir.join("runtime")).unwrap();
    assert!(!std::path::Path::new(runtime.trim()).exists());
    let pid = std::fs::read_to_string(dir.join("native-pid")).unwrap();
    assert!(!alive(pid.trim().parse().unwrap()));
    for forbidden in ["emu ", "emulator", "forward", "gearhead"] {
        assert!(!calls.contains(forbidden), "{calls}");
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("native")).unwrap(),
        "tcp=unset\n"
    );
}

#[test]
fn usb_reports_missing_unauthorized_offline_and_multiple_phones_before_installing() {
    for (devices, expected) in [
        ("emulator-5554 device\n192.168.1.2:5555 device\n", "no USB"),
        ("PHONE unauthorized usb:1\n", "USB debugging"),
        ("PHONE offline usb:1\n", "offline"),
        ("PHONE device usb:1\nOTHER device usb:2\n", "multiple USB"),
        (
            "PHONE device usb:1\nOTHER unauthorized usb:2\n",
            "multiple USB",
        ),
    ] {
        let s = UsbSuite::new(&format!("List of devices attached\n{devices}"));
        let output = s.run(&[]);
        assert_ne!(code(&output), 0, "{}", text(&output));
        assert!(text(&output).contains(expected), "{}", text(&output));
        let calls = std::fs::read_to_string(s.suite.parent().unwrap().join("calls")).unwrap();
        assert_eq!(calls, "devices -l\n");
    }
}

#[test]
fn usb_validation_does_not_require_a_phone_or_tools() {
    let s = UsbSuite::new("List of devices attached\n");
    let output = s.run(&["--validate"]);
    assert_eq!(code(&output), 0, "{}", text(&output));
    assert!(!s.suite.parent().unwrap().join("calls").exists());
}

#[test]
fn usb_acquires_reverse_ports_after_projection_and_removes_them_at_cleanup() {
    let s = UsbSuite::new("List of devices attached\nPHONE device usb:1\n");
    let suite = std::fs::read_to_string(&s.suite).unwrap().replace(
        "  appLabel: Test\n",
        "  appLabel: Test\n  reversePorts: [{device: 10101, host: 19101}]\n",
    );
    std::fs::write(&s.suite, suite).unwrap();
    let output = s.run(&[]);
    assert_eq!(code(&output), 0, "{}", text(&output));
    let dir = s.suite.parent().unwrap();
    assert!(!dir.join("reverse").exists());
    let calls = std::fs::read_to_string(dir.join("calls")).unwrap();
    assert!(
        calls.contains("-s PHONE reverse --no-rebind tcp:10101 tcp:19101"),
        "{calls}"
    );
    assert!(
        calls.contains("-s PHONE reverse --remove tcp:10101"),
        "{calls}"
    );
}

#[test]
fn usb_rejects_ordinary_test_files_instead_of_silently_using_an_existing_session() {
    let s = UsbSuite::new("List of devices attached\n");
    let output = s.sandbox.opendhu(&[
        "test",
        s.suite
            .parent()
            .unwrap()
            .join("test.yaml")
            .to_str()
            .unwrap(),
        "--usb",
    ]);
    assert_eq!(code(&output), 2, "{}", text(&output));
    assert!(
        text(&output).contains("--usb requires a managed suite"),
        "{}",
        text(&output)
    );
}

#[test]
fn usb_start_failure_cleans_up_the_private_runtime() {
    let mut s = UsbSuite::new("List of devices attached\nPHONE device usb:1\n");
    s.env.push(("FAKE_NATIVE_MODE".into(), "exit-early".into()));
    let output = s.run(&[]);
    assert_ne!(code(&output), 0, "{}", text(&output));
    let dir = s.suite.parent().unwrap();
    let runtime = std::fs::read_to_string(dir.join("runtime")).unwrap();
    assert!(!std::path::Path::new(runtime.trim()).exists());
    let pid = std::fs::read_to_string(dir.join("native-pid")).unwrap();
    assert!(!alive(pid.trim().parse().unwrap()));
}

#[test]
fn all_usb_runs_suites_sequentially_with_separate_reports_and_one_json_summary() {
    let _guard = USB_TEST.lock().unwrap();
    for target in [&[][..], &["all"][..]] {
        let s = UsbSuite::new("List of devices attached\nPHONE device usb:1\n");
        s.register_all();
        let output = s.registered_command(target, &["--json"]).output().unwrap();
        assert_eq!(code(&output), 0, "{}", text(&output));
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["exitCode"], 0);
        assert_eq!(value["suites"][0]["name"], "first");
        assert_eq!(value["suites"][1]["name"], "second");
        for name in ["first", "second"] {
            let dir = s.artifacts.join(name);
            assert!(dir.join("results.json").exists());
            assert!(dir.join("results.html").exists());
            assert!(dir.join("suite.json").exists());
        }
        let root = s.suite.parent().unwrap();
        let calls = std::fs::read_to_string(root.join("calls")).unwrap();
        assert_eq!(calls.matches("install -r -t").count(), 2, "{calls}");
        let runtime = std::fs::read_to_string(root.join("runtime")).unwrap();
        assert!(!std::path::Path::new(runtime.trim()).exists());
        let pid = std::fs::read_to_string(root.join("native-pid")).unwrap();
        assert!(!alive(pid.trim().parse().unwrap()));
    }
}

#[test]
fn all_usb_continues_after_a_failed_suite_and_returns_failure() {
    let _guard = USB_TEST.lock().unwrap();
    for target in [&[][..], &["all"][..]] {
        let s = UsbSuite::new("List of devices attached\nPHONE device usb:1\n");
        s.register_all();
        let root = s.suite.parent().unwrap();
        std::fs::write(root.join("failed.yaml"), "name: Wrong focus\nbackend: native\nsteps: [{action: assert.status, expect: {videoFocus: {observed: off}}}]\n").unwrap();
        let suite = std::fs::read_to_string(&s.suite)
            .unwrap()
            .replace("tests: [test.yaml]", "tests: [failed.yaml]");
        std::fs::write(&s.suite, suite).unwrap();
        let output = s.registered_command(target, &["--json"]).output().unwrap();
        assert_eq!(code(&output), 7, "{}", text(&output));
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["exitCode"], 7);
        assert_eq!(value["suites"][0]["exitCode"], 7);
        assert_eq!(value["suites"][1]["exitCode"], 0);
        assert!(s.artifacts.join("second/results.json").is_file());
    }
}

#[test]
fn all_usb_validates_later_suites_before_starting_any_device() {
    let s = UsbSuite::new("List of devices attached\nPHONE device usb:1\n");
    s.register_all();
    let root = s.suite.parent().unwrap();
    std::fs::write(
        root.join("second.yaml"),
        "version: 2\nenvironment: {}\ntests: []\n",
    )
    .unwrap();
    let output = s.all_command(&[]).output().unwrap();
    assert_eq!(code(&output), 2, "{}", text(&output));
    assert!(!root.join("calls").exists());
    assert!(!s.artifacts.exists());
}

#[test]
fn interrupting_all_usb_cleans_up_and_skips_remaining_suites() {
    use std::process::Stdio;
    use std::time::Duration;
    let _guard = USB_TEST.lock().unwrap();
    let s = UsbSuite::new("List of devices attached\nPHONE device usb:1\n");
    s.register_all();
    let root = s.suite.parent().unwrap();
    std::fs::write(
        root.join("test.yaml"),
        "name: Long wait\nbackend: native\nsteps: [{action: wait, duration: 60s}]\n",
    )
    .unwrap();
    let child = s
        .all_command(&["--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let ready = wait_until(Duration::from_secs(15), || {
        s.artifacts.join("first/suite.json").exists()
    });
    let signal = if ready { "-INT" } else { "-KILL" };
    std::process::Command::new("kill")
        .args([signal, &child.id().to_string()])
        .status()
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(ready, "suite must start: {}", text(&output));
    assert_eq!(code(&output), 130, "{}", text(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["interrupted"], true);
    assert_eq!(value["suites"].as_array().unwrap().len(), 1);
    assert!(!s.artifacts.join("second").exists());
    let runtime = std::fs::read_to_string(root.join("runtime")).unwrap();
    assert!(!std::path::Path::new(runtime.trim()).exists());
}

#[test]
fn recovered_app_crashes_and_anrs_fail_the_suite_and_every_report() {
    for log in [
        "10-17 23:08:01.000  321  321 E AndroidRuntime: FATAL EXCEPTION: main\n10-17 23:08:01.001  321  321 E AndroidRuntime: Process: test.app, PID: 321\n10-17 23:08:01.002  321  321 E AndroidRuntime: java.lang.IllegalStateException: injected crash",
        "10-17 23:08:01.000  321  321 F libc: Fatal signal 11 (SIGSEGV), code 1, fault addr 0x0 in tid 321 (test.app), pid 321 (test.app)",
        "10-17 23:08:01.000  665  700 E ActivityManager: ANR in test.app (test.app.MainActivity)",
    ] {
        let mut s = UsbSuite::new("List of devices attached\nPHONE device usb:1\n");
        s.env.push(("USB_CRASH_LOG".into(), log.into()));
        let output = s.run(&[]);
        assert_eq!(code(&output), 7, "{}", text(&output));
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(s.artifacts.join("results.json")).unwrap())
                .unwrap();
        assert_eq!(report["tests"][0]["outcome"], "failed");
        assert_eq!(report["metadata"]["summary"]["passed"], 0);
        for name in ["results.json", "results.xml", "results.html"] {
            let report = std::fs::read_to_string(s.artifacts.join(name)).unwrap();
            assert!(report.contains("test.app"), "{name}");
            assert!(report.contains("23:08:01"), "{name}");
        }
        assert!(text(&output).contains("app.health"), "{}", text(&output));
    }
}

#[test]
fn unrelated_crashes_and_intentional_force_stops_do_not_fail_the_suite() {
    let mut s = UsbSuite::new("List of devices attached\nPHONE device usb:1\n");
    s.env.push(("USB_CRASH_LOG".into(), "10-17 23:08:01.000 321 321 E AndroidRuntime: FATAL EXCEPTION: main\n10-17 23:08:01.001 321 321 E AndroidRuntime: Process: other.app, PID: 321\n10-17 23:08:01.002 665 665 I ActivityManager: Force stopping test.app appid=123 user=0\n10-17 23:08:01.003 321 321 E App: ordinary error".into()));
    let output = s.run(&[]);
    assert_eq!(code(&output), 0, "{}", text(&output));
}

#[test]
fn losing_the_crash_collector_is_an_error_in_the_report() {
    let mut s = UsbSuite::new("List of devices attached\nPHONE device usb:1\n");
    s.env.push(("USB_MONITOR_EXIT".into(), "1".into()));
    let output = s.run(&[]);
    assert_ne!(code(&output), 0, "{}", text(&output));
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(s.artifacts.join("results.json")).unwrap()).unwrap();
    assert_eq!(report["tests"][0]["outcome"], "error");
    assert!(
        report["tests"][0]["message"]
            .as_str()
            .unwrap()
            .contains("monitor")
    );
}

#[test]
fn startup_checkpoint_retries_if_logcat_attaches_after_the_first_marker() {
    let mut s = UsbSuite::new("List of devices attached\nPHONE device usb:1\n");
    s.env.push(("USB_DROP_FIRST_MARKER".into(), "1".into()));
    let output = s.run(&[]);
    assert_eq!(code(&output), 0, "{}", text(&output));
}

#[test]
fn a_stalled_collector_cannot_finalize_a_passing_test() {
    let mut s = UsbSuite::new("List of devices attached\nPHONE device usb:1\n");
    s.env.push(("USB_MONITOR_STALL".into(), "1".into()));
    let output = s.run(&[]);
    assert_ne!(code(&output), 0, "{}", text(&output));
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(s.artifacts.join("results.json")).unwrap()).unwrap();
    assert_eq!(report["tests"][0]["outcome"], "error");
    assert!(
        report["tests"][0]["message"]
            .as_str()
            .unwrap()
            .contains("barrier")
    );
}

#[test]
fn a_crash_during_the_configured_launch_is_not_discarded_by_test_reset() {
    let mut s = UsbSuite::new("List of devices attached\nPHONE device usb:1\n");
    s.env.push(("USB_LAUNCH_CRASH".into(), "10-17 23:08:01.000 321 321 E AndroidRuntime: FATAL EXCEPTION: main\n10-17 23:08:01.001 321 321 E AndroidRuntime: Process: test.app, PID: 321".into()));
    let output = s.run(&[]);
    assert_eq!(code(&output), 7, "{}", text(&output));
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(s.artifacts.join("results.json")).unwrap()).unwrap();
    assert_eq!(report["tests"][0]["outcome"], "failed");
}

#[test]
fn all_emulator_boots_once_across_suites_and_stops_only_at_run_end() {
    let _guard = USB_TEST.lock().unwrap();
    for failing in [false, true] {
        let s = UsbSuite::emulator_fixture();
        s.register_all();
        let root = s.suite.parent().unwrap();
        if failing {
            std::fs::write(root.join("failed.yaml"), "name: Wrong focus\nbackend: native\nsteps: [{action: assert.status, expect: {videoFocus: {observed: off}}}]\n").unwrap();
            let suite = std::fs::read_to_string(&s.suite)
                .unwrap()
                .replace("tests: [test.yaml]", "tests: [failed.yaml]");
            std::fs::write(&s.suite, suite).unwrap();
        }
        let output = s.registered_command(&[], &["--json"]).output().unwrap();
        assert_eq!(
            code(&output),
            if failing { 7 } else { 0 },
            "{}",
            text(&output)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["suites"][1]["exitCode"], 0);
        let events = std::fs::read_to_string(root.join("emulator-events")).unwrap();
        assert_eq!(events.lines().count(), 1, "boot only once: {events}");
        let calls = std::fs::read_to_string(root.join("calls")).unwrap();
        assert_eq!(
            calls.matches("emu kill").count(),
            1,
            "stop once after all suites"
        );
        assert!(calls.rfind("emu kill").unwrap() > calls.rfind("install -r -t").unwrap());
        let pid = std::fs::read_to_string(root.join("emulator-pid")).unwrap();
        assert!(!alive(pid.trim().parse().unwrap()));
        assert!(s.artifacts.join("first/results.json").is_file());
        assert!(s.artifacts.join("second/results.json").is_file());
    }
}

#[test]
fn all_emulator_preserves_a_preexisting_emulator() {
    let _guard = USB_TEST.lock().unwrap();
    let s = UsbSuite::emulator_fixture();
    s.register_all();
    let root = s.suite.parent().unwrap();
    std::fs::write(root.join("emulator-running"), "external").unwrap();
    let output = s.all_command(&["--json"]).output().unwrap();
    assert_eq!(code(&output), 0, "{}", text(&output));
    assert!(root.join("emulator-running").exists());
    assert!(!root.join("emulator-events").exists());
    let calls = std::fs::read_to_string(root.join("calls")).unwrap();
    assert!(!calls.contains("emu kill"));
}

#[test]
fn all_emulator_reuses_each_required_avd_and_cleans_up_both() {
    let _guard = USB_TEST.lock().unwrap();
    let s = UsbSuite::emulator_fixture();
    s.register_all();
    let root = s.suite.parent().unwrap();
    let first = std::fs::read_to_string(&s.suite).unwrap();
    let name = std::fs::read_to_string(root.join("avd-name")).unwrap();
    std::fs::write(
        root.join("second.yaml"),
        first.replace(&name, "another_avd"),
    )
    .unwrap();
    std::fs::copy(&s.suite, root.join("third.yaml")).unwrap();
    std::fs::write(
        root.join("opendhu.yaml"),
        "suites: {first: suite.yaml, second: second.yaml, third: third.yaml}\n",
    )
    .unwrap();
    let output = s.all_command(&["--json"]).output().unwrap();
    assert_eq!(code(&output), 0, "{}", text(&output));
    let events = std::fs::read_to_string(root.join("emulator-events")).unwrap();
    assert_eq!(events.lines().count(), 2, "one boot per AVD: {events}");
    let calls = std::fs::read_to_string(root.join("calls")).unwrap();
    assert_eq!(calls.matches("emu kill").count(), 2);
    assert!(calls.find("emu kill").unwrap() > calls.rfind("install -r -t").unwrap());
    for prefix in ["emulator", "other"] {
        let pid = std::fs::read_to_string(root.join(format!("{prefix}-pid"))).unwrap();
        assert!(!alive(pid.trim().parse().unwrap()));
    }
}

#[test]
fn standalone_emulator_still_stops_at_suite_end() {
    let _guard = USB_TEST.lock().unwrap();
    let s = UsbSuite::emulator_fixture();
    let output = s
        .registered_command(&["suite.yaml"], &["--json"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", text(&output));
    let root = s.suite.parent().unwrap();
    assert!(!root.join("emulator-running").exists());
    let pid = std::fs::read_to_string(root.join("emulator-pid")).unwrap();
    assert!(!alive(pid.trim().parse().unwrap()));
}

#[test]
fn interrupting_all_emulator_stops_owned_emulator_and_skips_remaining_suites() {
    use std::process::Stdio;
    use std::time::Duration;
    let _guard = USB_TEST.lock().unwrap();
    let s = UsbSuite::emulator_fixture();
    s.register_all();
    let root = s.suite.parent().unwrap();
    std::fs::write(
        root.join("test.yaml"),
        "name: Long wait\nbackend: native\nsteps: [{action: wait, duration: 60s}]\n",
    )
    .unwrap();
    let child = s
        .all_command(&["--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let ready = wait_until(Duration::from_secs(15), || {
        s.artifacts.join("first/suite.json").exists()
    });
    std::process::Command::new("kill")
        .args([
            if ready { "-INT" } else { "-KILL" },
            &child.id().to_string(),
        ])
        .status()
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(ready, "suite must start: {}", text(&output));
    assert_eq!(code(&output), 130, "{}", text(&output));
    assert!(!s.artifacts.join("second").exists());
    let pid = std::fs::read_to_string(root.join("emulator-pid")).unwrap();
    assert!(!alive(pid.trim().parse().unwrap()));
    assert!(!root.join("emulator-running").exists());
}

#[test]
fn all_emulator_does_not_stop_a_replacement_after_its_owned_child_exits() {
    let _guard = USB_TEST.lock().unwrap();
    let mut s = UsbSuite::emulator_fixture();
    s.env.push(("EMULATOR_REPLACE".into(), "1".into()));
    s.register_all();
    let output = s.all_command(&["--json"]).output().unwrap();
    assert_eq!(code(&output), 0, "{}", text(&output));
    let root = s.suite.parent().unwrap();
    assert!(root.join("emulator-replaced").exists());
    assert!(
        root.join("emulator-running").exists(),
        "external replacement must survive cleanup"
    );
    let calls = std::fs::read_to_string(root.join("calls")).unwrap();
    assert!(
        !calls.contains("emu kill"),
        "the original child exited; its serial is no longer owned"
    );
    let pid = std::fs::read_to_string(root.join("emulator-pid")).unwrap();
    assert!(!alive(pid.trim().parse().unwrap()));
}

#[test]
fn emulator_uses_configured_dns_servers_on_boot() {
    let _guard = USB_TEST.lock().unwrap();
    let s = UsbSuite::emulator_fixture();
    let source = std::fs::read_to_string(&s.suite).unwrap().replace(
        "  apk:",
        "  dnsServers: [8.8.8.8, '2001:4860:4860::8888']\n  apk:",
    );
    std::fs::write(&s.suite, source).unwrap();
    let output = s
        .registered_command(&["suite.yaml"], &["--json"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", text(&output));
    let args = std::fs::read_to_string(s.suite.parent().unwrap().join("emulator-args")).unwrap();
    assert!(
        args.contains("-dns-server 8.8.8.8,2001:4860:4860::8888"),
        "{args}"
    );
}

#[test]
fn invalid_emulator_dns_is_rejected_before_device_setup() {
    let s = Sandbox::new();
    let root = s.scratch("invalid-dns");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("test.yaml"),
        "name: test\nsteps: [{action: wait, duration: 1ms}]\n",
    )
    .unwrap();
    for dns in [
        "[not-an-ip]",
        "['']",
        "[8.8.8.8,8.8.4.4,1.1.1.1,1.0.0.1,9.9.9.9]",
    ] {
        std::fs::write(root.join("suite.yaml"), format!("version: 1\nenvironment: {{emulator: phone, apk: missing.apk, package: test.app, appLabel: Test, dnsServers: {dns}}}\ntests: [test.yaml]\n")).unwrap();
        let output = s.opendhu_env(
            &[
                "test",
                root.join("suite.yaml").to_str().unwrap(),
                "--validate",
            ],
            &[("PATH", "/no/tools")],
        );
        assert_eq!(code(&output), 2, "{}", text(&output));
        assert!(!s.dir.exists());
    }
}

//! The daemon as a process: foreground `autolab daemon`, signals, and what survives what.

mod common;

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use common::*;

fn spawn_daemon(s: &Sandbox, extra: &[&str]) -> Child {
    let mut args = vec!["daemon", "--port", "0"];
    args.extend_from_slice(extra);
    let child = Command::new(env!("CARGO_BIN_EXE_autolab"))
        .args(&args)
        .env("AUTOLAB_RUNTIME_DIR", &s.dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    assert!(
        wait_until(Duration::from_secs(10), || s
            .dir
            .join("daemon.json")
            .is_file()),
        "daemon never wrote daemon.json"
    );
    child
}

fn kill(sig: &str, pid: u64) {
    Command::new("kill")
        .args([sig, &pid.to_string()])
        .status()
        .unwrap();
}

#[test]
fn a_foreground_daemon_serves_the_cli_and_outlives_the_session() {
    let s = Sandbox::new();
    let mut d = spawn_daemon(&s, &[]);
    let pid = u64::from(d.id());

    assert_eq!(code(&s.start_fake()), 0);
    let st = s.status_json();
    assert_eq!(
        st["daemonPid"], pid,
        "the CLI must have used this daemon, not started another"
    );
    assert_eq!(st["phase"], "running");
    assert_eq!(st["sessionId"].as_str().unwrap().len(), 6);

    assert_eq!(code(&s.autolab(&["stop"])), 0);
    assert!(
        alive(pid),
        "a daemon you started yourself stays up after the session ends"
    );
    assert_eq!(
        code(&s.autolab(&["status"])),
        3,
        "no session, even though the daemon is up"
    );
    assert_eq!(code(&s.start_fake()), 0, "and it can host another session");

    kill("-TERM", pid);
    assert!(d.wait().unwrap().success());
}

fn signal_stops_session_dhu_and_file(signal: &str) {
    let s = Sandbox::new();
    let mut d = spawn_daemon(&s, &[]);
    assert_eq!(code(&s.start_fake()), 0);
    let dhu = s.status_json()["dhuPid"].as_u64().unwrap();

    kill(signal, u64::from(d.id()));
    let status = loop_wait(&mut d);
    assert!(
        status.success(),
        "a signalled daemon should exit cleanly: {status:?}"
    );
    assert!(
        wait_until(Duration::from_secs(5), || !alive(dhu)),
        "the DHU must not outlive the daemon"
    );
    assert!(!s.dir.join("daemon.json").exists());
}

#[test]
fn sigterm_stops_the_session_the_dhu_and_removes_the_file() {
    signal_stops_session_dhu_and_file("-TERM");
}

#[test]
fn ctrl_c_stops_the_session_the_dhu_and_removes_the_file() {
    signal_stops_session_dhu_and_file("-INT");
}

fn loop_wait(child: &mut Child) -> std::process::ExitStatus {
    let end = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < end {
        if let Some(st) = child.try_wait().unwrap() {
            return st;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("daemon did not exit after SIGTERM");
}

#[test]
fn a_killed_daemon_leaves_no_dhu_and_a_new_session_can_start() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    let st = s.status_json();
    let (daemon, dhu) = (
        st["daemonPid"].as_u64().unwrap(),
        st["dhuPid"].as_u64().unwrap(),
    );

    kill("-KILL", daemon);
    assert!(
        wait_until(Duration::from_secs(5), || !alive(dhu)),
        "the DHU exits when the daemon that owns its stdin dies, so nothing is orphaned"
    );
    assert!(
        s.dir.join("daemon.json").exists(),
        "a killed daemon cannot clean up after itself"
    );
    assert_eq!(
        code(&s.autolab(&["status"])),
        3,
        "the stale file must not look like a live session"
    );
    assert_eq!(code(&s.start_fake()), 0, "and it must not block a new one");
}

#[test]
fn a_second_daemon_in_the_same_runtime_dir_is_refused_with_exit_8() {
    let s = Sandbox::new();
    let mut first = spawn_daemon(&s, &[]);
    let o = s.autolab(&["daemon", "--port", "0"]);
    assert_eq!(code(&o), 8, "{}", text(&o));
    assert!(text(&o).contains("already running"), "{}", text(&o));
    assert!(
        first.try_wait().unwrap().is_none(),
        "the first daemon must be unharmed"
    );
    kill("-TERM", u64::from(first.id()));
    let _ = first.wait();
}

#[test]
fn relative_paths_are_sent_to_the_daemon_as_absolute() {
    let s = Sandbox::new();
    let work = s.scratch("work");
    std::fs::create_dir_all(work.join("cfg")).unwrap();
    let args = s.scratch("args.txt");
    let o = Command::new(env!("CARGO_BIN_EXE_autolab"))
        .args([
            "start",
            "--dhu-path",
            &fake_dhu(),
            "--serial",
            "FAKE0001",
            "--config",
            "cfg/my.ini",
        ])
        .current_dir(&work)
        .env("AUTOLAB_RUNTIME_DIR", &s.dir)
        .env("FAKE_DHU_ARGS_FILE", &args)
        .output()
        .unwrap();
    assert_eq!(code(&o), 0, "{}", text(&o));
    let seen = std::fs::read_to_string(&args).unwrap();
    // A working directory reports its resolved form (/tmp is /private/tmp on macOS).
    let want = format!(
        "--config={}",
        work.canonicalize().unwrap().join("cfg/my.ini").display()
    );
    assert!(
        seen.lines().any(|l| l == want),
        "the daemon's cwd is not the caller's: {seen}"
    );
    assert!(seen.lines().any(|l| l == "--usb=FAKE0001"), "{seen}");
}

#[test]
fn status_json_is_one_flat_object_with_stable_keys() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    let v = s.status_json();
    for k in [
        "sessionId",
        "phase",
        "backend",
        "state",
        "serial",
        "backendVersion",
        "videoFocus",
        "transport",
        "daemonPid",
        "dhuPid",
        "tunnels",
    ] {
        assert!(v.get(k).is_some(), "missing key {k}: {v}");
    }
    assert_eq!(v["videoFocus"]["observed"], "unknown");
}

#[test]
fn start_does_not_leave_a_daemon_running_after_stop() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    let daemon = s.status_json()["daemonPid"].as_u64().unwrap();
    assert_eq!(code(&s.autolab(&["stop"])), 0);
    assert!(
        wait_until(Duration::from_secs(10), || !alive(daemon)),
        "a daemon that `start` launched exits when its session ends"
    );
    assert!(!Path::new(&s.dir.join("daemon.json")).exists());
}

//! Observe the real CLI while a step is still running, rather than checking its final report.
mod common;
use common::*;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[test]
fn progress_is_flushed_before_a_long_step_finishes() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    let file = s.scratch("progress.yaml");
    std::fs::write(
        &file,
        "name: Live progress\nsteps:\n  - action: wait\n    duration: 10ms\n  - action: wait\n    duration: 10s\n",
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_aa"))
        .args([
            "test",
            file.to_str().unwrap(),
            "--artifacts-dir",
            s.scratch("art").to_str().unwrap(),
        ])
        .env("AUTOLAB_RUNTIME_DIR", &s.dir)
        .env("XDG_CONFIG_HOME", s.dir.join("no-xdg"))
        .env_remove("AUTOLAB_CONFIG")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut observed = Vec::new();
    while let Ok(line) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        let ready = line.contains("RUN 2. wait");
        observed.push(line);
        if ready {
            break;
        }
    }
    let running = child.try_wait().unwrap().is_none();
    let _ = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status();
    let _ = child.wait();
    reader.join().unwrap();
    assert!(
        observed.iter().any(|s| s.contains("RUN 2. wait")),
        "no live step progress: {observed:?}"
    );
    assert!(
        observed.iter().any(|s| s.contains("PASS 1. wait")),
        "completed step was not reported before the next wait: {observed:?}"
    );
    assert!(running, "progress arrived only after the test exited");
}

// A real PTY exercises terminal detection and animated output; no emulator is involved.
fn terminal_run(s: &Sandbox, args: &[&str], no_color: bool) -> std::process::Output {
    terminal_run_mode(s, args, no_color, false)
}
fn terminal_run_mode(
    s: &Sandbox,
    args: &[&str],
    no_color: bool,
    interrupt: bool,
) -> std::process::Output {
    const PTY: &str = r#"
import os, pty, signal, subprocess, sys
master, slave = pty.openpty()
p = subprocess.Popen(sys.argv[1:], stdin=subprocess.DEVNULL, stdout=slave, stderr=slave)
os.close(slave)
sent = False
while True:
    try:
        data = os.read(master, 65536)
        if not data: break
        sys.stdout.buffer.write(data)
        sys.stdout.buffer.flush()
        if os.environ.get("AUTOLAB_TEST_INTERRUPT_PTY") == "1" and b"wait (10s)" in data and not sent:
            p.send_signal(signal.SIGTERM)
            sent = True
    except OSError:
        break
os.close(master)
sys.exit(p.wait())
"#;
    let mut command = Command::new("python3");
    command
        .args(["-c", PTY, env!("CARGO_BIN_EXE_aa")])
        .args(args)
        .env("AUTOLAB_RUNTIME_DIR", &s.dir)
        .env("XDG_CONFIG_HOME", s.dir.join("no-xdg"))
        .env("TERM", "xterm-256color")
        .env_remove("AUTOLAB_CONFIG")
        .env_remove("NO_COLOR");
    if no_color {
        command.env("NO_COLOR", "1");
    }
    command.env(
        "AUTOLAB_TEST_INTERRUPT_PTY",
        if interrupt { "1" } else { "0" },
    );
    command.output().unwrap()
}

#[test]
fn terminal_results_are_colored_but_no_color_and_redirected_logs_stay_plain() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    let file = s.scratch("color.yaml");
    std::fs::write(
        &file,
        "name: Colored result\nsteps: [{action: wait, duration: 10ms}]\n",
    )
    .unwrap();
    let art = s.scratch("art");
    let args = [
        "test",
        file.to_str().unwrap(),
        "--artifacts-dir",
        art.to_str().unwrap(),
    ];
    let terminal = terminal_run(&s, &args, false);
    assert_eq!(code(&terminal), 0, "{}", text(&terminal));
    assert!(
        terminal.stdout.windows(5).any(|s| s == b"\x1b[32m"),
        "passes need green terminal styling: {}",
        text(&terminal)
    );
    let monochrome = terminal_run(&s, &args, true);
    assert_eq!(code(&monochrome), 0, "{}", text(&monochrome));
    for color in ["\x1b[32m", "\x1b[36m", "\x1b[0m"] {
        assert!(
            !text(&monochrome).contains(color),
            "NO_COLOR leaked styling"
        );
    }
    let redirected = s.autolab(&args);
    assert_eq!(code(&redirected), 0, "{}", text(&redirected));
    assert!(
        !redirected.stdout.contains(&0x1b),
        "redirected logs leaked ANSI escapes"
    );
}

#[test]
fn interactive_defaults_animate_a_wait_then_leave_its_result_and_preserve_json() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    let file = s.scratch("spinner.yaml");
    std::fs::write(
        &file,
        "name: Animated wait\nsteps: [{action: wait, duration: 500ms}]\n",
    )
    .unwrap();
    let art = s.scratch("art");
    let args = [
        "test",
        file.to_str().unwrap(),
        "--artifacts-dir",
        art.to_str().unwrap(),
    ];
    let terminal = terminal_run(&s, &args, false);
    assert_eq!(code(&terminal), 0, "{}", text(&terminal));
    let shown = text(&terminal);
    assert!(
        shown.matches("\x1b[2K").count() >= 3,
        "wait did not animate: {shown}"
    );
    assert!(
        shown.contains("wait (500ms)") && shown.contains("PASS") && shown.contains("1 passed"),
        "{shown}"
    );
    let mut static_args = args.to_vec();
    static_args.push("--no-interactive");
    let static_output = terminal_run(&s, &static_args, true);
    assert_eq!(code(&static_output), 0, "{}", text(&static_output));
    assert!(
        !text(&static_output).contains("\x1b[2K"),
        "spinner must be disabled"
    );
    assert!(text(&static_output).contains("RUN 1. wait"));
    let plain = s.autolab(&args);
    assert_eq!(code(&plain), 0, "{}", text(&plain));
    assert!(
        !plain.stdout.contains(&0x1b),
        "redirected interactive output must degrade to plain logs"
    );
    let mut json_args = args.to_vec();
    json_args.push("--json");
    let json = s.autolab(&json_args);
    assert_eq!(code(&json), 0, "{}", text(&json));
    let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(report["exitCode"], 0);
    assert!(
        !text(&json).contains('\x1b'),
        "JSON mode must disable styling and animation"
    );
}

#[test]
fn interactive_stops_its_spinner_and_writes_the_report_when_interrupted() {
    let s = Sandbox::new();
    assert_eq!(code(&s.start_fake()), 0);
    let file = s.scratch("interrupted.yaml");
    std::fs::write(
        &file,
        "name: Interrupted animation\nsteps: [{action: wait, duration: 10s}]\n",
    )
    .unwrap();
    let art = s.scratch("art");
    let output = terminal_run_mode(
        &s,
        &[
            "test",
            file.to_str().unwrap(),
            "--interactive",
            "--artifacts-dir",
            art.to_str().unwrap(),
        ],
        false,
        true,
    );
    assert_eq!(code(&output), 130, "{}", text(&output));
    let shown = text(&output);
    let final_result = shown.rfind("Result:").unwrap();
    assert!(
        !shown[final_result..].contains("\x1b[2K"),
        "spinner continued after interruption: {shown}"
    );
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(art.join("results.json")).unwrap()).unwrap();
    assert_eq!(report["interrupted"], true);
}

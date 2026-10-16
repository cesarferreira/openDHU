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

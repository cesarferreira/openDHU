use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use autolab_core::{Error, Result};

use crate::console;

enum Chunk {
    Data(String),
    Eof,
}

/// A running DHU with piped stdin and a single merged output pipe. Verified: the DHU accepts
/// console commands over plain pipes (no pty) and exits cleanly when its stdin closes.
///
/// The DHU prints its prompt on stdout but its replies and log lines on stderr. Reading them
/// from two pipes loses their relative order, so both share one pipe and arrive in write order.
pub struct DhuProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<Chunk>,
    startup: Vec<String>,
}

fn pump(mut r: impl Read + Send + 'static, tx: mpsc::Sender<Chunk>) {
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match r.read(&mut buf) {
                Ok(0) | Err(_) => {
                    let _ = tx.send(Chunk::Eof);
                    return;
                }
                Ok(n) => {
                    if tx
                        .send(Chunk::Data(String::from_utf8_lossy(&buf[..n]).into_owned()))
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
    });
}

impl DhuProcess {
    /// Spawn with a structured argv. Never goes through a shell.
    pub fn spawn<S: AsRef<OsStr>>(
        program: &Path,
        args: &[S],
        env: &[(&str, &str)],
    ) -> Result<Self> {
        let (reader, writer) = io::pipe()
            .map_err(|e| Error::BackendUnavailable(format!("cannot create output pipe: {e}")))?;
        let dup = |w: &io::PipeWriter| {
            w.try_clone().map_err(|e| {
                Error::BackendUnavailable(format!("cannot duplicate output pipe: {e}"))
            })
        };
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(dup(&writer)?)
            .stderr(writer);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let spawned = cmd.spawn();
        drop(cmd); // releases our copies of the write end so the reader sees EOF on exit
        let mut child = spawned.map_err(|e| {
            Error::BackendUnavailable(format!("cannot launch {}: {e}", program.display()))
        })?;
        let (tx, rx) = mpsc::channel();
        pump(reader, tx);
        let stdin = child.stdin.take();
        Ok(Self {
            child,
            stdin,
            rx,
            startup: Vec::new(),
        })
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn startup_lines(&self) -> &[String] {
        &self.startup
    }

    /// `Some(description)` once the process has exited.
    pub fn exit_status(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(s)) => Some(s.to_string()),
            Ok(None) => None,
            Err(e) => Some(format!("unknown ({e})")),
        }
    }

    /// Block until the phone link is up (TLS handshake done) or `timeout` passes.
    pub fn wait_ready(&mut self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        let mut raw = String::new();
        loop {
            if console::clean_lines(&raw)
                .iter()
                .any(|l| l.contains(console::READY_MARKER))
            {
                break;
            }
            match self.recv_until(deadline) {
                Some(Chunk::Data(d)) => raw.push_str(&d),
                Some(Chunk::Eof) => {
                    self.reap();
                    let lines = console::clean_lines(&raw);
                    return Err(Error::BackendUnavailable(format!(
                        "DHU exited during startup ({}). Last output:\n{}",
                        self.exit_status().unwrap_or_else(|| "still running".into()),
                        tail(&lines, 8)
                    )));
                }
                None => {
                    let lines = console::clean_lines(&raw);
                    return Err(Error::Timeout(format!(
                        "DHU did not finish connecting within {}s. Last output:\n{}",
                        timeout.as_secs(),
                        tail(&lines, 8)
                    )));
                }
            }
        }
        // Let trailing banner/prompt output arrive so it is not mistaken for a command reply.
        let quiet = Instant::now() + Duration::from_millis(400);
        while let Some(Chunk::Data(d)) = self.recv_until(quiet) {
            raw.push_str(&d);
        }
        self.startup = console::clean_lines(&raw);
        Ok(())
    }

    /// Send one console command and return the reply lines. Bounded by `timeout`.
    pub fn round_trip(&mut self, command: &str, timeout: Duration) -> Result<Vec<String>> {
        if command.contains(['\n', '\r']) {
            return Err(Error::InvalidArgs(
                "console command must be a single line".into(),
            ));
        }
        if let Some(st) = self.exit_status() {
            return Err(Error::BackendUnavailable(format!("DHU has exited ({st})")));
        }
        while self.rx.try_recv().is_ok() {} // discard stale async output
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| Error::BackendUnavailable("DHU stdin is closed".into()))?;
        writeln!(stdin, "{command}")
            .and_then(|_| stdin.flush())
            .map_err(|e| Error::BackendUnavailable(format!("cannot write to DHU: {e}")))?;

        let deadline = Instant::now() + timeout;
        let mut raw = String::new();
        loop {
            match self.recv_until(deadline) {
                Some(Chunk::Data(d)) => {
                    raw.push_str(&d);
                    if console::ends_with_prompt(&raw) {
                        return Ok(console::clean_lines(&raw));
                    }
                }
                Some(Chunk::Eof) => {
                    self.reap();
                    return Err(Error::BackendUnavailable(format!(
                        "DHU exited while handling '{command}' ({})",
                        self.exit_status()
                            .unwrap_or_else(|| "status unknown".into())
                    )));
                }
                None => {
                    return Err(Error::Timeout(format!(
                        "no console prompt within {}s after '{command}'",
                        timeout.as_secs_f32()
                    )));
                }
            }
        }
    }

    /// Ask the DHU to quit, then force it if it does not exit in time. Returns whether the
    /// shutdown was graceful.
    pub fn shutdown(&mut self, grace: Duration) -> bool {
        if self.exit_status().is_some() {
            return true;
        }
        if let Some(mut stdin) = self.stdin.take() {
            let _ = writeln!(stdin, "quit");
            let _ = stdin.flush();
            // dropping stdin closes the pipe, which also makes the DHU exit
        }
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if self.exit_status().is_some() {
                return true;
            }
            thread::sleep(Duration::from_millis(25));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        false
    }

    fn reap(&mut self) {
        let end = Instant::now() + Duration::from_secs(2);
        while Instant::now() < end && self.exit_status().is_none() {
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Next chunk of output, `Eof` once the DHU closed its output, or `None` on timeout.
    fn recv_until(&self, deadline: Instant) -> Option<Chunk> {
        let left = deadline.saturating_duration_since(Instant::now());
        match self.rx.recv_timeout(left) {
            Ok(c) => Some(c),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => Some(Chunk::Eof),
        }
    }
}

impl Drop for DhuProcess {
    fn drop(&mut self) {
        self.shutdown(Duration::from_secs(2));
    }
}

fn tail(lines: &[String], n: usize) -> String {
    let start = lines.len().saturating_sub(n);
    lines[start..]
        .iter()
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

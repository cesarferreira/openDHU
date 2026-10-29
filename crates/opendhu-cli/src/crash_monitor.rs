//! Device-scoped logcat capture. A marker round trip orders each test's final health check
//! after all records already written to logd, rather than guessing a delay for the reader.
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use opendhu_core::{Error, Result};

const TAG: &str = "OpenDHUHealth";
const BARRIER_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Parser {
    package: String,
    // AndroidRuntime announces the exception before its Process line identifies the app.
    java: HashMap<String, Vec<String>>,
    incident: Option<(String, Vec<String>)>,
}

fn process_matches(process: &str, package: &str) -> bool {
    process == package
        || process
            .strip_prefix(package)
            .is_some_and(|s| s.starts_with(':'))
}

fn record(line: &str) -> Option<(&str, &str, &str)> {
    let mut rest = line;
    let mut pid = "";
    for field in 0..5 {
        let (value, tail) = rest.trim_start().split_once(char::is_whitespace)?;
        if field == 2 {
            pid = value;
        }
        rest = tail;
    }
    let (tag, message) = rest.trim_start().split_once(':')?;
    Some((pid, tag.trim(), message.trim()))
}

impl Parser {
    fn push(&mut self, line: &str) {
        let Some((pid, tag, message)) = record(line) else {
            return;
        };
        if tag == "AndroidRuntime" {
            if message.starts_with("FATAL EXCEPTION:") {
                if self.java.len() >= 32 {
                    self.java.clear();
                }
                self.java.insert(pid.into(), vec![line.into()]);
            } else if let Some(lines) = self.java.get_mut(pid) {
                if lines.len() < 24 {
                    lines.push(line.into());
                }
            }
            if let Some(process) = message
                .strip_prefix("Process: ")
                .and_then(|s| s.split(',').next())
                && process_matches(process, &self.package)
                && let Some(lines) = self.java.remove(pid)
                && self.incident.is_none()
            {
                self.incident = Some((pid.into(), lines));
            } else if let Some((owner, lines)) = &mut self.incident
                && owner == pid
                && lines.len() < 24
            {
                lines.push(line.into());
            }
        }
        let anr = tag == "ActivityManager"
            && message
                .strip_prefix("ANR in ")
                .and_then(|s| s.split_whitespace().next())
                .is_some_and(|p| process_matches(p, &self.package));
        let native = (tag == "libc"
            && message.starts_with("Fatal signal ")
            && message
                .rsplit_once('(')
                .and_then(|(_, s)| s.strip_suffix(')'))
                .is_some_and(|p| process_matches(p, &self.package)))
            || (tag == "DEBUG"
                && message
                    .split_once(">>> ")
                    .and_then(|(_, s)| s.split_once(" <<<"))
                    .is_some_and(|(p, _)| process_matches(p, &self.package)));
        let event = matches!(tag, "am_crash" | "am_anr")
            && message
                .trim_start_matches('[')
                .split(',')
                .nth(2)
                .is_some_and(|p| process_matches(p.trim(), &self.package));
        if (anr || native || event) && self.incident.is_none() {
            self.incident = Some((pid.into(), vec![line.into()]));
        }
    }
    fn detail(&self) -> Option<String> {
        self.incident.as_ref().map(|(_, lines)| {
            format!(
                "unexpected app crash or ANR in {}:\n{}",
                self.package,
                lines.join("\n")
            )
        })
    }
}

struct State {
    parser: Parser,
    active: bool,
    expected: String,
    received: bool,
    closed: Option<String>,
    /// Which logcat stream is current; a replaced stream's reader must not mark the monitor closed.
    generation: u64,
}

type Shared = Arc<(Mutex<State>, Condvar)>;

pub struct CrashMonitor {
    adb: PathBuf,
    serial: String,
    log: PathBuf,
    child: Mutex<Child>,
    shared: Shared,
    // Only the test-running thread issues barriers; the reader never takes this lock.
    barrier: Mutex<u64>,
    nonce: u128,
}

impl CrashMonitor {
    pub fn start(adb: &Path, serial: &str, package: &str, log: &Path) -> Result<Self> {
        let file = std::fs::File::create(log).map_err(|e| Error::Other(e.to_string()))?;
        let mut child = spawn_logcat(adb, serial, &file)?;
        let stdout = child.stdout.take().expect("logcat stdout");
        let shared = Arc::new((
            Mutex::new(State {
                parser: Parser {
                    package: package.into(),
                    ..Parser::default()
                },
                active: false,
                expected: String::new(),
                received: false,
                closed: None,
                generation: 0,
            }),
            Condvar::new(),
        ));
        let reader = Arc::clone(&shared);
        std::thread::spawn(move || capture(stdout, file, reader, 0));
        let monitor = Self {
            adb: adb.into(),
            serial: serial.into(),
            log: log.into(),
            child: Mutex::new(child),
            shared,
            barrier: Mutex::new(0),
            nonce: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        };
        // Ignore historical records, without clearing the device's shared log buffers.
        monitor.checkpoint()?;
        Ok(monitor)
    }

    /// Replace the logcat stream after the phone re-enumerated (a new USB head-unit session drops
    /// every ADB connection). Records logged while no stream was attached are not inspected.
    pub fn reconnect(&self) -> Result<()> {
        let file = std::fs::OpenOptions::new()
            .append(true)
            .open(&self.log)
            .map_err(|e| Error::Other(e.to_string()))?;
        let mut child = spawn_logcat(&self.adb, &self.serial, &file)?;
        let stdout = child.stdout.take().expect("logcat stdout");
        let generation = {
            let mut state = self.shared.0.lock().unwrap();
            state.generation += 1;
            state.closed = None;
            state.active = false;
            state.generation
        };
        {
            let mut old = self.child.lock().unwrap();
            let _ = old.kill();
            let _ = old.wait();
            *old = child;
        }
        let reader = Arc::clone(&self.shared);
        std::thread::spawn(move || capture(stdout, file, reader, generation));
        self.checkpoint()
    }

    pub fn check(&self) -> Result<Option<String>> {
        let state = self.shared.0.lock().unwrap();
        if let Some(detail) = state.parser.detail() {
            return Ok(Some(format!("{detail}\nlogcat: {}", self.log.display())));
        }
        if let Some(error) = &state.closed {
            return Err(Error::Other(format!(
                "crash monitor unavailable: {error}; see {}",
                self.log.display()
            )));
        }
        Ok(None)
    }

    pub fn checkpoint(&self) -> Result<()> {
        let mut sequence = self.barrier.lock().unwrap();
        *sequence += 1;
        let marker = format!("opendhu-{}-{}-{}", std::process::id(), self.nonce, sequence);
        {
            let mut state = self.shared.0.lock().unwrap();
            if let Some(error) = &state.closed {
                return Err(Error::Other(format!("crash monitor unavailable: {error}")));
            }
            state.expected = marker.clone();
            state.received = false;
        }
        let deadline = Instant::now() + BARRIER_TIMEOUT;
        loop {
            // -T 1 may skip the first marker if logcat attaches after newer records.
            // Retry the same token until the live collector acknowledges it.
            crate::device::command(
                &self.adb,
                &["-s", &self.serial, "shell", "log", "-t", TAG, &marker],
                deadline.saturating_duration_since(Instant::now()),
            )
            .map_err(|e| {
                Error::Other(format!(
                    "crash monitor barrier failed: {e}; see {}",
                    self.log.display()
                ))
            })?;
            let state = self.shared.0.lock().unwrap();
            let (state, _) = self
                .shared
                .1
                .wait_timeout_while(
                    state,
                    Duration::from_millis(100)
                        .min(deadline.saturating_duration_since(Instant::now())),
                    |s| !s.received && s.closed.is_none(),
                )
                .unwrap();
            if state.received {
                break;
            }
            if state.closed.is_some() || Instant::now() >= deadline {
                return Err(Error::Other(format!(
                    "crash monitor did not receive logcat barrier: {}; see {}",
                    state
                        .closed
                        .as_deref()
                        .unwrap_or("collector stalled or log access denied"),
                    self.log.display()
                )));
            }
        }
        Ok(())
    }

    /// Flush and consume the incident atomically. Records arriving after this snapshot
    /// remain pending for the next test instead of being erased by a later acknowledgement.
    pub fn finish(&self) -> Result<Option<String>> {
        self.checkpoint()?;
        let mut state = self.shared.0.lock().unwrap();
        if let Some(detail) = state.parser.detail() {
            state.parser.incident = None;
            return Ok(Some(format!("{detail}\nlogcat: {}", self.log.display())));
        }
        if let Some(error) = &state.closed {
            return Err(Error::Other(format!(
                "crash monitor unavailable: {error}; see {}",
                self.log.display()
            )));
        }
        Ok(None)
    }

    pub fn stop(&self) {
        let mut child = self.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Drop for CrashMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}

fn spawn_logcat(adb: &Path, serial: &str, file: &std::fs::File) -> Result<Child> {
    Command::new(adb)
        .args([
            "-s",
            serial,
            "logcat",
            "-b",
            "main",
            "-b",
            "system",
            "-b",
            "crash",
            "-b",
            "events",
            "-v",
            "threadtime",
            "-T",
            "1",
            "*:V",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(
            file.try_clone().map_err(|e| Error::Other(e.to_string()))?,
        ))
        .spawn()
        .map_err(|e| Error::Other(format!("cannot start crash monitor: {e}")))
}

fn capture(stdout: impl std::io::Read, mut file: std::fs::File, shared: Shared, generation: u64) {
    let result = (|| -> std::io::Result<()> {
        let mut reader = BufReader::new(stdout);
        let mut bytes = Vec::new();
        loop {
            bytes.clear();
            if reader.read_until(b'\n', &mut bytes)? == 0 {
                break;
            }
            file.write_all(&bytes)?;
            file.flush()?;
            let line = String::from_utf8_lossy(&bytes);
            let mut state = shared.0.lock().unwrap();
            if state.generation != generation {
                return Ok(());
            }
            if record(&line)
                .is_some_and(|(_, tag, message)| tag == TAG && message == state.expected)
            {
                state.received = true;
                state.active = true;
                shared.1.notify_all();
            } else if state.active {
                state.parser.push(line.trim_end());
            }
        }
        Ok(())
    })();
    let mut state = shared.0.lock().unwrap();
    if state.generation != generation {
        return;
    }
    state.closed = Some(
        result
            .err()
            .map_or_else(|| "logcat exited or disconnected".into(), |e| e.to_string()),
    );
    shared.1.notify_all();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(package: &str, log: &str) -> Option<String> {
        let mut parser = Parser {
            package: package.into(),
            ..Parser::default()
        };
        for line in log.lines() {
            parser.push(line);
        }
        parser.detail()
    }

    #[test]
    fn a_reconnected_stream_restores_health_checks_after_the_phone_re_enumerates() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("aa-crash-monitor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let device_log = dir.join("device.log");
        std::fs::write(&device_log, "").unwrap();
        // `logcat` streams the device log; `shell log -t TAG MSG` appends to it like logd.
        let adb = dir.join("adb");
        std::fs::write(
            &adb,
            format!(
                "#!/bin/sh\nshift 2\ncase \"$1\" in\n  logcat) echo $$ > '{dir}/logcat.pid'; exec tail -n 0 -F '{log}' ;;\n  shell) echo \"10-29 11:00:00.000 1 1 I $4: $5\" >> '{log}' ;;\nesac\n",
                dir = dir.display(),
                log = device_log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&adb, std::fs::Permissions::from_mode(0o755)).unwrap();
        let monitor =
            CrashMonitor::start(&adb, "SERIAL", "test.app", &dir.join("logcat.txt")).unwrap();
        assert!(monitor.check().unwrap().is_none());

        // A new USB session drops every ADB connection, ending the stream.
        let pid = std::fs::read_to_string(dir.join("logcat.pid")).unwrap();
        Command::new("kill").arg(pid.trim()).status().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while monitor.check().is_ok() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            monitor.check().is_err(),
            "a dropped stream must be reported"
        );

        monitor.reconnect().unwrap();
        assert!(monitor.check().unwrap().is_none());
        let mut log = std::fs::OpenOptions::new()
            .append(true)
            .open(&device_log)
            .unwrap();
        writeln!(
            log,
            "10-29 11:00:01.000 665 665 I am_crash: [0,321,test.app,123,reason]"
        )
        .unwrap();
        let detail = monitor.finish().unwrap().expect("crash after reconnect");
        assert!(detail.contains("am_crash"), "{detail}");
        monitor.stop();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn interleaved_java_crashes_keep_the_target_stack_and_latch_after_recovery() {
        let log = "10-17 23:08:01.000 321 321 E AndroidRuntime: FATAL EXCEPTION: main\n\
                   10-17 23:08:01.001 999 999 E AndroidRuntime: FATAL EXCEPTION: main\n\
                   10-17 23:08:01.002 999 999 E AndroidRuntime: Process: other.app, PID: 999\n\
                   10-17 23:08:01.003 321 321 E AndroidRuntime: Process: test.app:worker, PID: 321\n\
                   10-17 23:08:01.004 321 321 E AndroidRuntime: java.lang.IllegalStateException: broken\n\
                   10-17 23:08:01.005 321 321 E AndroidRuntime:     at test.app.Work.run(Work.kt:4)\n\
                   10-17 23:08:02.000 665 665 I ActivityManager: Start proc 500:test.app/u0a123";
        let detail = parse("test.app", log).unwrap();
        assert!(detail.contains("FATAL EXCEPTION"));
        assert!(detail.contains("Work.kt:4"));
        assert!(!detail.contains("other.app"));
    }

    #[test]
    fn native_tombstone_identifies_long_process_names_after_truncated_libc_record() {
        let log = "10-17 23:08:01.000 321 321 F libc: Fatal signal 11 (SIGSEGV), code 1 in tid 321 (ui.dev.debug), pid 321 (ui.dev.debug)\n\
                   10-17 23:08:01.100 400 400 F DEBUG   : pid: 321, tid: 321, name: ui.dev.debug  >>> ai.wayve.robotui.dev.debug <<<";
        assert!(
            parse("ai.wayve.robotui.dev.debug", log)
                .unwrap()
                .contains("pid: 321")
        );
    }

    #[test]
    fn system_crash_and_anr_events_are_scoped_to_exact_package_or_subprocess() {
        for tag in ["am_crash", "am_anr"] {
            let log =
                format!("10-17 23:08:01.000 665 665 I {tag}: [0,321,test.app:worker,123,reason]");
            assert!(parse("test.app", &log).is_some());
        }
        for log in [
            "10-17 23:08:01.000 665 665 E ActivityManager: ANR in test.application",
            "10-17 23:08:01.000 321 321 E AndroidRuntime: Process: test.app, PID: 321",
            "10-17 23:08:01.000 321 321 E App: Exception mentioning test.app",
            "10-17 23:08:01.000 665 665 I am_crash: [0,321,test.application,123,reason]",
            "10-17 23:08:01.000 321 321 F libc: Fatal signal 11 in tid 321 (test.app), pid 321 (other.app)",
            "10-17 23:08:01.000 665 665 I ActivityManager: Force stopping test.app appid=123 user=0",
        ] {
            assert!(parse("test.app", log).is_none(), "{log}");
        }
    }
}

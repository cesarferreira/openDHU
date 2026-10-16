//! Finding the daemon, and starting one when `autolab start` needs it.

use std::fs::File;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use autolab_core::{Error, Result};
use autolab_daemon::{Client, runtime};

const STARTUP: Duration = Duration::from_secs(15);

/// Connect to a running daemon. Fails with exit code 3 ("no session") if there is none.
pub fn connect() -> Result<Client> {
    Client::connect()
}

/// Connect to the daemon, starting one if none answers. A daemon started here is detached and
/// exits when its session ends; run `autolab daemon` yourself for one that stays.
pub fn ensure() -> Result<Client> {
    let dir = runtime::runtime_dir()?;
    if let Ok(client) = Client::connect_in(&dir) {
        return Ok(client);
    }
    let log = dir.join(runtime::LOG_FILE);
    let file = File::create(&log)
        .map_err(|e| Error::Other(format!("cannot create {}: {e}", log.display())))?;
    let exe = std::env::current_exe()
        .map_err(|e| Error::Other(format!("cannot locate own executable: {e}")))?;
    let mut child = Command::new(exe)
        .args(["daemon", "--exit-when-idle"])
        .stdin(Stdio::null())
        .stdout(file.try_clone().map_err(|e| Error::Other(e.to_string()))?)
        .stderr(file)
        .process_group(0)
        .spawn()
        .map_err(|e| Error::Other(format!("cannot start the AutoLab daemon: {e}")))?;

    let deadline = Instant::now() + STARTUP;
    loop {
        // Connecting wins over an early exit: a second `autolab start` racing this one may have
        // lost the bind to the first daemon, which is exactly the one we want to talk to.
        if let Ok(client) = Client::connect_in(&dir) {
            return Ok(client);
        }
        if let Ok(Some(_)) = child.try_wait() {
            return Err(Error::BackendUnavailable(format!(
                "the AutoLab daemon exited during startup: {}",
                failure_reason(&log)
            )));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Timeout(format!(
                "the AutoLab daemon did not become reachable within {}s (see {})",
                STARTUP.as_secs(),
                log.display()
            )));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// The daemon prints structured JSON logs and, on failure, one `error: ...` line.
fn failure_reason(log: &std::path::Path) -> String {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    text.lines()
        .rev()
        .find_map(|l| l.strip_prefix("error: "))
        .map_or_else(|| format!("see {}", log.display()), str::to_string)
}

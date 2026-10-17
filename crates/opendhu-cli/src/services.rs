//! Processes owned by a suite. Never attach to or stop an unrelated development service.
use fs2::FileExt;
use opendhu_core::{Error, Result};
use opendhu_test::schema::Dur;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Service {
    pub name: String,
    pub command: Vec<String>,
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub ready_port: u16,
    #[serde(default = "default_timeout")]
    pub timeout: Dur,
}
fn default_timeout() -> Dur {
    Dur(Duration::from_secs(60))
}
impl Service {
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty()
            || !self
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            || self.command.is_empty()
            || self.command[0].is_empty()
            || self.ready_port == 0
            || self.timeout.0.is_zero()
            || self.timeout.0 > Duration::from_secs(600)
        {
            return Err(Error::InvalidArgs(
                "service requires a safe name, command, readyPort and bounded timeout".into(),
            ));
        }
        if self
            .env
            .keys()
            .any(|key| key.is_empty() || key.contains('=') || key.contains('\0'))
        {
            return Err(Error::InvalidArgs(
                "invalid service environment name".into(),
            ));
        }
        Ok(())
    }
}
pub struct OwnedService {
    child: Child,
    name: String,
    _port_lock: std::fs::File,
}
impl OwnedService {
    pub fn spawn(spec: &Service, base: &Path, artifacts: &Path) -> Result<Self> {
        let lock_dir = std::env::temp_dir().join("opendhu-service-locks");
        std::fs::create_dir_all(&lock_dir).map_err(|e| Error::Other(e.to_string()))?;
        let port_lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_dir.join(format!("{}.lock", spec.ready_port)))
            .map_err(|e| Error::Other(e.to_string()))?;
        port_lock.try_lock_exclusive().map_err(|_| {
            Error::SessionConflict(format!(
                "service ready port {} is leased by another suite",
                spec.ready_port
            ))
        })?;
        // A readiness check must never accidentally accept an already-running user's service.
        let probe = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, spec.ready_port))
            .map_err(|_| {
            Error::SessionConflict(format!(
                "service {} ready port {} is already in use",
                spec.name, spec.ready_port
            ))
        })?;
        drop(probe);
        let file = std::fs::File::create(artifacts.join(format!("{}.log", spec.name)))
            .map_err(|e| Error::Other(e.to_string()))?;
        let cwd = spec
            .cwd
            .as_ref()
            .map(|path| base.join(path))
            .unwrap_or_else(|| base.into());
        let child = Command::new(&spec.command[0])
            .args(&spec.command[1..])
            .current_dir(cwd)
            .envs(&spec.env)
            .env("OPENDHU_ARTIFACTS_DIR", artifacts)
            .stdin(Stdio::null())
            .stdout(file.try_clone().map_err(|e| Error::Other(e.to_string()))?)
            .stderr(file)
            .process_group(0)
            .spawn()
            .map_err(|e| {
                Error::BackendUnavailable(format!("cannot start service {}: {e}", spec.name))
            })?;
        Ok(Self {
            child,
            name: spec.name.clone(),
            _port_lock: port_lock,
        })
    }
    pub fn wait_ready(&mut self, spec: &Service, cancel: &AtomicBool) -> Result<()> {
        let deadline = Instant::now() + spec.timeout.0;
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(Error::Other("service startup interrupted".into()));
            }
            if let Some(status) = self
                .child
                .try_wait()
                .map_err(|e| Error::Other(e.to_string()))?
            {
                return Err(Error::BackendUnavailable(format!(
                    "service {} exited {status}; see {}.log",
                    self.name, self.name
                )));
            }
            let address =
                std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, spec.ready_port));
            if std::net::TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout(format!(
                    "service {} was not ready; see {}.log",
                    self.name, self.name
                )));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
impl Drop for OwnedService {
    fn drop(&mut self) {
        let group = -(self.child.id() as i32);
        // Every descendant belongs to this freshly-created process group.
        unsafe {
            libc::kill(group, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if unsafe { libc::kill(group, 0) } != 0 {
                break;
            }
            let _ = self.child.try_wait();
            std::thread::sleep(Duration::from_millis(25));
        }
        unsafe {
            libc::kill(group, libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_service_cleans_up_its_descendants_and_preserves_logs() {
        let dir = std::env::temp_dir().join(format!("aa-service-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("child.pid");
        let script = format!(
            "sleep 120 & echo $! > '{}'; echo service-started; wait",
            marker.display()
        );
        let spec = Service {
            name: "fixture".into(),
            command: vec!["sh".into(), "-c".into(), script],
            cwd: None,
            env: Default::default(),
            ready_port: 0,
            timeout: opendhu_test::schema::Dur(std::time::Duration::from_millis(100)),
        };
        let mut service = OwnedService::spawn(&spec, &dir, &dir).unwrap();
        let started = Instant::now();
        while !marker.exists() && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            marker.exists(),
            "child process must start before testing readiness failure"
        );
        assert!(
            service
                .wait_ready(&spec, &std::sync::atomic::AtomicBool::new(false))
                .is_err()
        );
        drop(service);
        let pid: i32 = std::fs::read_to_string(marker)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "descendant must be gone");
        assert!(
            std::fs::read_to_string(dir.join("fixture.log"))
                .unwrap()
                .contains("service-started")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn readiness_port_is_leased_even_before_the_child_listens() {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
        let spec = Service {
            name: "port-lease".into(),
            command: vec!["/bin/sleep".into(), "120".into()],
            cwd: None,
            env: Default::default(),
            ready_port: port,
            timeout: default_timeout(),
        };
        let dir = std::env::temp_dir();
        let first = OwnedService::spawn(&spec, &dir, &dir).unwrap();
        let second = OwnedService::spawn(&spec, &dir, &dir);
        assert!(
            second.is_err(),
            "another suite must not claim the starting service's ready port"
        );
        drop(first);
        assert!(
            OwnedService::spawn(&spec, &dir, &dir).is_ok(),
            "dropping service releases its lease"
        );
    }
    #[test]
    fn a_busy_ready_port_is_refused_without_touching_its_owner() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let spec = Service {
            name: "occupied".into(),
            command: vec!["false".into()],
            cwd: None,
            env: Default::default(),
            ready_port: listener.local_addr().unwrap().port(),
            timeout: default_timeout(),
        };
        let dir = std::env::temp_dir();
        assert!(OwnedService::spawn(&spec, &dir, &dir).is_err());
        assert!(std::net::TcpStream::connect(listener.local_addr().unwrap()).is_ok());
    }
}

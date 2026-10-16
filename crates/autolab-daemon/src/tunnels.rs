//! `adb reverse` tunnels for the app under test. Starting a DHU session switches the phone's USB
//! mode, which drops them, so they are applied after the DHU attaches and kept alive afterwards.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use autolab_core::{Error, Result};
use autolab_dhu::adb;

use crate::api::{TunnelState, TunnelStatus};
use crate::events::EventBus;

const DEVICE_RETURN_WAIT: Duration = Duration::from_secs(30);
/// How often the keeper checks that requested tunnels still exist.
const CHECK_EVERY: Duration = Duration::from_secs(2);

/// Tunnels requested for a session, and the adb used to check them.
pub struct Tunnels {
    pub adb: Option<PathBuf>,
    pub ports: Vec<u16>,
    /// How many times the keeper had to put each tunnel back, parallel to `ports`.
    pub reapplied: Vec<AtomicU32>,
}

impl Tunnels {
    pub fn none() -> Self {
        Self {
            adb: None,
            ports: Vec::new(),
            reapplied: Vec::new(),
        }
    }
}

/// Live view from `adb reverse --list`, never from what was requested.
pub fn status(serial: &str, tunnels: &Tunnels) -> Vec<TunnelStatus> {
    let live = tunnels
        .adb
        .as_deref()
        .and_then(|adb| adb::reversed_ports(adb, serial));
    tunnels
        .ports
        .iter()
        .zip(&tunnels.reapplied)
        .map(|(&port, reapplied)| TunnelStatus {
            port,
            state: match &live {
                None => TunnelState::Unknown,
                Some(l) if l.contains(&port) => TunnelState::Active,
                Some(_) => TunnelState::Missing,
            },
            reapplied: reapplied.load(Ordering::SeqCst),
        })
        .collect()
}

/// Apply the tunnels and confirm each against `adb reverse --list`. A tunnel counts only once the
/// device lists it. Any failure aborts the start, so a session never looks healthy while the app
/// under test cannot reach its backend.
pub fn setup(serial: &str, ports: &[u16]) -> Result<Tunnels> {
    if ports.is_empty() {
        return Ok(Tunnels::none());
    }
    let adb = adb::find_adb(
        std::env::var("PATH").ok().as_deref(),
        &adb::sdk_roots_from_env(),
    )
    .ok_or_else(|| {
        Error::BackendUnavailable("adb not found; it is needed for reverse tunnels".into())
    })?;
    let wait = std::env::var("AUTOLAB_DEVICE_WAIT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(DEVICE_RETURN_WAIT, Duration::from_secs);
    if !adb::wait_for_device(&adb, serial, wait) {
        return Err(Error::DeviceDisconnected(format!(
            "adb did not see {serial} again within {}s after the DHU attached, so tunnels cannot be set up",
            wait.as_secs()
        )));
    }
    for &port in ports {
        adb::reverse(&adb, serial, port).map_err(|e| {
            Error::DeviceDisconnected(format!("`adb reverse tcp:{port}` failed: {e}"))
        })?;
    }
    let live = adb::reversed_ports(&adb, serial).ok_or_else(|| {
        Error::DeviceDisconnected(
            "could not read `adb reverse --list` to confirm the tunnels".into(),
        )
    })?;
    if let Some(port) = ports.iter().find(|p| !live.contains(p)) {
        return Err(Error::DeviceDisconnected(format!(
            "adb accepted `reverse tcp:{port}` but `adb reverse --list` does not show it, so it could not be confirmed"
        )));
    }
    Ok(Tunnels {
        adb: Some(adb),
        ports: ports.to_vec(),
        reapplied: ports.iter().map(|_| AtomicU32::new(0)).collect(),
    })
}

/// Background upkeep until `stop` is set: put back any requested tunnel the phone dropped. It acts
/// only when adb answers and the list lacks the port, so a phone that is mid-re-enumeration (adb
/// failing) is left alone instead of being hammered.
pub fn keep(
    serial: String,
    tunnels: Arc<Tunnels>,
    stop: Arc<AtomicBool>,
    events: Arc<EventBus>,
    session: String,
) {
    let Some(adb) = tunnels.adb.clone() else {
        return;
    };
    let every = std::env::var("AUTOLAB_TUNNEL_CHECK_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(CHECK_EVERY, Duration::from_millis);
    while !sleep_unless_stopped(every, &stop) {
        let Some(live) = adb::reversed_ports(&adb, &serial) else {
            continue;
        };
        for (&port, count) in tunnels.ports.iter().zip(&tunnels.reapplied) {
            if live.contains(&port) {
                continue;
            }
            match adb::reverse(&adb, &serial, port) {
                Ok(()) => {
                    count.fetch_add(1, Ordering::SeqCst);
                    crate::log::info("tunnel.reapplied", serde_json::json!({ "port": port }));
                    events.emit(
                        "tunnel.reapplied",
                        Some(&session),
                        serde_json::json!({ "port": port }),
                    );
                }
                Err(e) => crate::log::warn(
                    "tunnel.reapply_failed",
                    serde_json::json!({ "port": port, "message": e }),
                ),
            }
        }
    }
}

/// Sleep up to `total` in short slices. Returns true as soon as `stop` is set.
fn sleep_unless_stopped(total: Duration, stop: &AtomicBool) -> bool {
    let end = Instant::now() + total;
    while Instant::now() < end {
        if stop.load(Ordering::SeqCst) {
            return true;
        }
        thread::sleep(Duration::from_millis(50).min(end.saturating_duration_since(Instant::now())));
    }
    stop.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_ports_means_no_adb_is_needed() {
        let t = setup("S", &[]).unwrap();
        assert!(t.adb.is_none() && t.ports.is_empty());
    }

    #[test]
    fn sleeping_ends_promptly_when_stopped() {
        let stop = AtomicBool::new(true);
        let started = Instant::now();
        assert!(sleep_unless_stopped(Duration::from_secs(30), &stop));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn sleeping_runs_to_completion_when_not_stopped() {
        let stop = AtomicBool::new(false);
        let started = Instant::now();
        assert!(!sleep_unless_stopped(Duration::from_millis(120), &stop));
        assert!(started.elapsed() >= Duration::from_millis(120));
    }
}

//! The daemon's one session and every operation on it.
//!
//! Concurrency model: `slot` is locked only for brief state transitions, never across slow work.
//! The blocking backend lives behind its own mutex, taken only inside `spawn_blocking`, so
//! commands are serialized (clients cannot interleave on the DHU console) while `status` still
//! answers immediately during a slow start. A session is `Idle`, `Starting`, `Running` or
//! `Stopping`, and anything but `Idle` rejects a second `start`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use autolab_core::{Capabilities, Error, HeadUnitBackend, HeadUnitStatus, Result};
use autolab_dhu::adb;
use serde_json::json;
use tokio::sync::watch;

use crate::api::{
    BackendKind, DaemonInfo, DeviceInfo, DevicesResponse, InputResponse, OperationResponse, Phase,
    RunningDetails, ScreenshotResponse, SessionStatus, StartRequest, StatusResponse,
};
use crate::backend::AnyBackend;
use crate::events::EventBus;
use crate::tunnels::{self, Tunnels};
use crate::{launch, log, runtime};

#[derive(Clone)]
struct RunningSession {
    id: String,
    backend: Arc<Mutex<AnyBackend>>,
    kind: BackendKind,
    serial: String,
    tunnels: Arc<Tunnels>,
    stop_keeper: Arc<AtomicBool>,
}

enum Slot {
    Idle,
    Starting { id: String },
    Running(RunningSession),
    Stopping { id: String },
}

struct Inner {
    slot: Mutex<Slot>,
    events: Arc<EventBus>,
    ops: AtomicU64,
    info: DaemonInfo,
    /// An `exit_when_idle` daemon with no session for this long exits, so one whose client died
    /// before sending `start` does not linger.
    idle_grace: Duration,
    shutdown: watch::Sender<bool>,
}

#[derive(Clone)]
pub struct Daemon {
    inner: Arc<Inner>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn no_session() -> Error {
    Error::BackendUnavailable(
        "no AutoLab session is running; start one with `autolab start`".into(),
    )
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::Other(format!("backend task failed: {e}")))
}

async fn with_backend<T: Send + 'static>(
    backend: Arc<Mutex<AnyBackend>>,
    f: impl FnOnce(&mut AnyBackend) -> T + Send + 'static,
) -> Result<T> {
    blocking(move || f(&mut lock(&backend))).await
}

impl Daemon {
    pub fn new(info: DaemonInfo, idle_grace: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                slot: Mutex::new(Slot::Idle),
                events: Arc::new(EventBus::new()),
                ops: AtomicU64::new(0),
                info,
                idle_grace,
                shutdown: watch::channel(false).0,
            }),
        }
    }

    pub fn info(&self) -> &DaemonInfo {
        &self.inner.info
    }

    pub fn events(&self) -> &Arc<EventBus> {
        &self.inner.events
    }

    fn next_op(&self) -> String {
        format!("op-{}", self.inner.ops.fetch_add(1, Ordering::SeqCst) + 1)
    }

    fn running(&self) -> Result<RunningSession> {
        match &*lock(&self.inner.slot) {
            Slot::Running(r) => Ok(r.clone()),
            Slot::Idle => Err(no_session()),
            Slot::Starting { .. } => Err(Error::BackendUnavailable(
                "the session is still starting".into(),
            )),
            Slot::Stopping { .. } => {
                Err(Error::BackendUnavailable("the session is stopping".into()))
            }
        }
    }

    // ---- reads ---------------------------------------------------------------------------

    pub async fn status(&self) -> Result<StatusResponse> {
        let (id, phase, running) = match &*lock(&self.inner.slot) {
            Slot::Idle => {
                return Ok(StatusResponse {
                    daemon: self.inner.info.clone(),
                    session: None,
                });
            }
            Slot::Starting { id } => (id.clone(), Phase::Starting, None),
            Slot::Stopping { id } => (id.clone(), Phase::Stopping, None),
            Slot::Running(r) => (r.id.clone(), Phase::Running, Some(r.clone())),
        };
        let details = match running {
            None => None,
            Some(r) => {
                let (head, dhu_pid): (Result<HeadUnitStatus>, u32) =
                    with_backend(Arc::clone(&r.backend), |b| (b.status(), b.pid())).await?;
                let (serial, tunnels) = (r.serial.clone(), Arc::clone(&r.tunnels));
                // adb can be slow, so tunnel state is read after the backend lock is released.
                let tunnels = blocking(move || tunnels::status(&serial, &tunnels)).await?;
                Some(RunningDetails {
                    head_unit: head?,
                    transport: "usb".into(),
                    daemon_pid: std::process::id(),
                    dhu_pid,
                    tunnels,
                })
            }
        };
        Ok(StatusResponse {
            daemon: self.inner.info.clone(),
            session: Some(SessionStatus {
                session_id: id,
                phase,
                details,
            }),
        })
    }

    pub async fn capabilities(&self) -> Result<Capabilities> {
        let r = self.running()?;
        with_backend(r.backend, |b| b.capabilities()).await?
    }

    pub async fn devices(&self) -> Result<DevicesResponse> {
        blocking(|| {
            let adb_path = launch::adb_path()?;
            let list = adb::list_devices(&adb_path)
                .map_err(|e| Error::BackendUnavailable(format!("adb failed: {e}")))?;
            Ok(DevicesResponse {
                devices: list
                    .into_iter()
                    .map(|d| DeviceInfo {
                        serial: d.serial,
                        state: d.state,
                        model: d.model,
                    })
                    .collect(),
            })
        })
        .await?
    }

    // ---- commands ------------------------------------------------------------------------

    pub async fn focus_video(&self, enabled: bool) -> Result<OperationResponse> {
        let r = self.running()?;
        let op = self.next_op();
        let events = &self.inner.events;
        events.emit(
            "focus.video.requested",
            Some(&r.id),
            json!({ "operationId": op, "enabled": enabled }),
        );
        match with_backend(r.backend, move |b| b.set_video_focus(enabled)).await? {
            Ok(result) => {
                let kind = if result.status.is_success() {
                    "focus.video.accepted"
                } else {
                    "focus.video.failed"
                };
                events.emit(
                    kind,
                    Some(&r.id),
                    json!({
                        "operationId": op,
                        "requested": result.requested,
                        "backendReported": result.backend_reported,
                        "observed": result.observed,
                        "evidence": result.evidence,
                    }),
                );
                Ok(OperationResponse {
                    operation_id: op,
                    backend: r.kind,
                    result,
                })
            }
            Err(e) => {
                events.emit(
                    "focus.video.failed",
                    Some(&r.id),
                    json!({ "operationId": op, "message": e.message() }),
                );
                Err(e)
            }
        }
    }

    /// Both touch kinds share one path so they log and emit the same way.
    async fn input(
        &self,
        kind: &'static str,
        detail: serde_json::Value,
        f: impl FnOnce(&mut AnyBackend) -> Result<autolab_core::InputResult> + Send + 'static,
    ) -> Result<InputResponse> {
        let r = self.running()?;
        let op = self.next_op();
        let events = &self.inner.events;
        events.emit(
            "input.requested",
            Some(&r.id),
            json!({ "operationId": op, "kind": kind, "at": detail }),
        );
        match with_backend(r.backend, f).await? {
            Ok(result) => {
                events.emit(
                    "input.accepted",
                    Some(&r.id),
                    json!({ "operationId": op, "kind": kind, "evidence": result.evidence }),
                );
                Ok(InputResponse {
                    operation_id: op,
                    backend: r.kind,
                    result,
                })
            }
            Err(e) => {
                events.emit(
                    "input.failed",
                    Some(&r.id),
                    json!({ "operationId": op, "kind": kind, "message": e.message() }),
                );
                Err(e)
            }
        }
    }

    pub async fn key(&self, key: autolab_core::Key, long_press: bool) -> Result<InputResponse> {
        self.input(
            "key",
            json!({ "key": key.name(), "longPress": long_press }),
            move |b| b.press_key(key, long_press),
        )
        .await
    }

    pub async fn tap(&self, x: u32, y: u32) -> Result<InputResponse> {
        self.input("tap", json!({ "x": x, "y": y }), move |b| b.tap(x, y))
            .await
    }

    pub async fn swipe(
        &self,
        from: (u32, u32),
        to: (u32, u32),
        duration: std::time::Duration,
    ) -> Result<InputResponse> {
        self.input(
            "swipe",
            json!({ "from": [from.0, from.1], "to": [to.0, to.1], "durationMs": duration.as_millis() as u64 }),
            move |b| b.swipe(from, to, duration),
        )
        .await
    }

    /// A live feed of the phone's video, for `GET /v1/video`.
    pub async fn video(&self) -> Result<std::sync::mpsc::Receiver<autolab_native::VideoChunk>> {
        let r = self.running()?;
        with_backend(r.backend, |b| b.subscribe_video()).await?
    }

    /// A live feed of one channel of the phone's audio, for `GET /v1/audio/{channel}`.
    pub async fn audio(
        &self,
        channel: autolab_core::AudioChannel,
    ) -> Result<std::sync::mpsc::Receiver<autolab_native::VideoChunk>> {
        let r = self.running()?;
        with_backend(r.backend, move |b| b.subscribe_audio(channel)).await?
    }

    pub async fn screenshot(&self, path: PathBuf) -> Result<ScreenshotResponse> {
        validate_screenshot_path(&path)?;
        let r = self.running()?;
        let op = self.next_op();
        let events = &self.inner.events;
        events.emit(
            "screenshot.requested",
            Some(&r.id),
            json!({ "operationId": op, "path": path }),
        );
        match with_backend(r.backend, move |b| b.screenshot(&path)).await? {
            Ok(result) => {
                events.emit(
                    "screenshot.saved",
                    Some(&r.id),
                    json!({ "operationId": op, "path": result.path, "width": result.width, "height": result.height }),
                );
                Ok(ScreenshotResponse {
                    operation_id: op,
                    result,
                })
            }
            Err(e) => {
                events.emit(
                    "screenshot.failed",
                    Some(&r.id),
                    json!({ "operationId": op, "message": e.message() }),
                );
                Err(e)
            }
        }
    }

    // ---- lifecycle -----------------------------------------------------------------------

    pub async fn start(&self, req: StartRequest) -> Result<StatusResponse> {
        if req.reverse.contains(&0) {
            return Err(Error::InvalidArgs("`reverse` ports must be 1-65535".into()));
        }
        let id = runtime::new_session_id()?;
        {
            let mut slot = lock(&self.inner.slot);
            match &*slot {
                Slot::Idle => *slot = Slot::Starting { id: id.clone() },
                Slot::Starting { .. } => {
                    return Err(Error::SessionConflict(
                        "a session is already starting".into(),
                    ));
                }
                Slot::Running(_) => {
                    return Err(Error::SessionConflict(
                        "an AutoLab session is already running; run `autolab stop` first".into(),
                    ));
                }
                Slot::Stopping { .. } => {
                    return Err(Error::SessionConflict(
                        "the previous session is still stopping".into(),
                    ));
                }
            }
        }
        let events = &self.inner.events;
        events.emit("session.starting", Some(&id), json!({}));
        log::info("session.starting", json!({ "sessionId": id }));

        let (launch_id, bus) = (id.clone(), Arc::clone(&self.inner.events));
        match blocking(move || launch_session(&req, launch_id, bus)).await? {
            Ok(running) => {
                *lock(&self.inner.slot) = Slot::Running(running.clone());
                events.emit(
                    "session.started",
                    Some(&id),
                    json!({ "serial": running.serial }),
                );
                log::info(
                    "session.started",
                    json!({ "sessionId": id, "serial": running.serial }),
                );
                self.status().await
            }
            Err(e) => {
                *lock(&self.inner.slot) = Slot::Idle;
                events.emit(
                    "session.start_failed",
                    Some(&id),
                    json!({ "message": e.message() }),
                );
                log::warn(
                    "session.start_failed",
                    json!({ "sessionId": id, "message": e.message() }),
                );
                self.signal_if_ephemeral();
                Err(e)
            }
        }
    }

    pub async fn stop(&self) -> Result<()> {
        let running = {
            let mut slot = lock(&self.inner.slot);
            match &*slot {
                Slot::Running(r) => {
                    let r = r.clone();
                    *slot = Slot::Stopping { id: r.id.clone() };
                    r
                }
                Slot::Idle => return Err(no_session()),
                Slot::Starting { .. } => {
                    return Err(Error::SessionConflict(
                        "the session is still starting; try again shortly".into(),
                    ));
                }
                Slot::Stopping { .. } => {
                    return Err(Error::SessionConflict(
                        "the session is already stopping".into(),
                    ));
                }
            }
        };
        let events = &self.inner.events;
        events.emit("session.stopping", Some(&running.id), json!({}));
        running.stop_keeper.store(true, Ordering::SeqCst);
        let result = with_backend(Arc::clone(&running.backend), |b| b.stop()).await;
        *lock(&self.inner.slot) = Slot::Idle;
        events.emit("session.stopped", Some(&running.id), json!({}));
        log::info("session.stopped", json!({ "sessionId": running.id }));
        self.signal_if_ephemeral();
        result?
    }

    /// Stop the session (if any) and ask the server to exit.
    pub async fn shutdown(&self) {
        for _ in 0..1200 {
            if !matches!(*lock(&self.inner.slot), Slot::Starting { .. }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let _ = self.stop().await;
        self.inner.shutdown.send_replace(true);
    }

    pub async fn wait_shutdown(&self) {
        let mut rx = self.inner.shutdown.subscribe();
        let _ = rx.wait_for(|stop| *stop).await;
    }

    /// Receiver that becomes `true` when shutdown starts; long-lived streams end on it.
    pub fn shutdown_receiver(&self) -> watch::Receiver<bool> {
        self.inner.shutdown.subscribe()
    }

    fn signal_if_ephemeral(&self) {
        if self.inner.info.exit_when_idle {
            self.inner.shutdown.send_replace(true);
        }
    }

    /// Watch for the DHU dying on its own and end the session when it does.
    pub fn spawn_monitor(&self) {
        let d = self.clone();
        tokio::spawn(async move {
            let mut idle_since: Option<Instant> = None;
            loop {
                tokio::time::sleep(Duration::from_millis(200)).await;
                let (running, idle) = match &*lock(&d.inner.slot) {
                    Slot::Running(r) => (Some(r.clone()), false),
                    Slot::Idle => (None, true),
                    _ => (None, false),
                };
                if idle && d.inner.info.exit_when_idle {
                    let since = *idle_since.get_or_insert_with(Instant::now);
                    if since.elapsed() >= d.inner.idle_grace {
                        log::info(
                            "daemon.idle_timeout",
                            json!({ "afterMs": since.elapsed().as_millis() as u64 }),
                        );
                        d.inner.shutdown.send_replace(true);
                        return;
                    }
                } else {
                    idle_since = None;
                }
                let Some(r) = running else { continue };
                // try_lock: never wait behind a command that is in flight.
                let exited = r.backend.try_lock().ok().and_then(|mut b| b.exit_status());
                if let Some(status) = exited {
                    d.end_session(&r, &format!("the DHU exited ({status})"));
                }
            }
        });
    }

    fn end_session(&self, r: &RunningSession, reason: &str) {
        {
            let mut slot = lock(&self.inner.slot);
            if !matches!(&*slot, Slot::Running(cur) if cur.id == r.id) {
                return;
            }
            *slot = Slot::Idle;
        }
        r.stop_keeper.store(true, Ordering::SeqCst);
        self.inner
            .events
            .emit("session.ended", Some(&r.id), json!({ "reason": reason }));
        log::warn(
            "session.ended",
            json!({ "sessionId": r.id, "reason": reason }),
        );
        self.signal_if_ephemeral();
    }
}

/// Runs on a blocking thread. If anything after the DHU launch fails, dropping the backend stops
/// the DHU, so a failed start leaves nothing running.
fn launch_session(req: &StartRequest, id: String, events: Arc<EventBus>) -> Result<RunningSession> {
    let (backend, serial) = launch::launch_backend(req)?;
    let tunnels = Arc::new(tunnels::setup(&serial, &req.reverse)?);
    let stop_keeper = Arc::new(AtomicBool::new(false));
    if !tunnels.ports.is_empty() {
        let (serial, shared, stop) = (
            serial.clone(),
            Arc::clone(&tunnels),
            Arc::clone(&stop_keeper),
        );
        let session = id.clone();
        thread::spawn(move || tunnels::keep(serial, shared, stop, events, session));
    }
    Ok(RunningSession {
        id,
        backend: Arc::new(Mutex::new(backend)),
        kind: req.backend,
        serial,
        tunnels,
        stop_keeper,
    })
}

/// Absolute, `.png`, no whitespace (the DHU console cannot take it), existing directory.
pub fn validate_screenshot_path(path: &Path) -> Result<()> {
    let text = path.to_string_lossy();
    if !path.is_absolute() {
        return Err(Error::InvalidArgs("`path` must be absolute".into()));
    }
    if text.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(Error::InvalidArgs(
            "`path` must not contain whitespace or control characters (the DHU console cannot take them safely)".into(),
        ));
    }
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
    {
        return Err(Error::InvalidArgs("`path` must end in .png".into()));
    }
    if !path.parent().is_some_and(Path::is_dir) {
        return Err(Error::InvalidArgs(format!(
            "directory of {text} does not exist"
        )));
    }
    if path.is_dir() {
        return Err(Error::InvalidArgs(format!(
            "{text} is a directory, not a file"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_paths_are_validated() {
        let ok = std::env::temp_dir().join("shot.png");
        assert!(validate_screenshot_path(&ok).is_ok());
        for bad in [
            "rel.png",
            "/tmp/a b.png",
            "/tmp/x.jpg",
            "/no/such/dir/x.png",
            "/tmp/new\nline.png",
        ] {
            assert_eq!(
                validate_screenshot_path(Path::new(bad))
                    .unwrap_err()
                    .exit_code(),
                2,
                "{bad}"
            );
        }
        assert!(
            validate_screenshot_path(&std::env::temp_dir()).is_err(),
            "a directory is not a file"
        );
    }
}

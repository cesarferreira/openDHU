//! Session ownership for the DHU backend.
//!
//! The DHU takes console commands only on its own stdin, so a DHU that some other process
//! started cannot be controlled. `autolab start` therefore launches a small detached
//! supervisor that owns the DHU's pipes and serves requests over a user-only Unix socket.
//! This is the seed of the Milestone 2 daemon; the wire format is deliberately tiny.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use autolab_core::{
    Capabilities, Error, HeadUnitBackend, HeadUnitStatus, OperationResult, Result, ScreenshotResult,
};
use autolab_dhu::{DhuBackend, DhuLaunch, adb};
use serde::{Deserialize, Serialize};

/// DHU handshake (up to 45s) plus waiting for the phone to return to adb (up to 30s).
const START_TIMEOUT: Duration = Duration::from_secs(100);
const DEVICE_RETURN_WAIT: Duration = Duration::from_secs(30);
/// How often the supervisor checks that requested tunnels still exist.
const TUNNEL_CHECK: Duration = Duration::from_secs(2);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Status,
    Capabilities,
    FocusVideo { enabled: bool },
    Screenshot { path: PathBuf },
    Stop,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    Status(StatusInfo),
    Capabilities(Capabilities),
    Operation(OperationResult),
    Screenshot(ScreenshotResult),
    Stopped,
    Error { code: u8, message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusInfo {
    #[serde(flatten)]
    pub head_unit: HeadUnitStatus,
    pub transport: String,
    pub supervisor_pid: u32,
    pub dhu_pid: u32,
    /// Tunnels requested with `--reverse`, each checked live against `adb reverse --list`.
    #[serde(default)]
    pub tunnels: Vec<TunnelStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TunnelState {
    Active,
    Missing,
    /// adb could not be queried, so nothing is known.
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelStatus {
    pub port: u16,
    pub state: TunnelState,
    /// Times AutoLab re-applied this tunnel after the phone dropped it.
    #[serde(default)]
    pub reapplied: u32,
}

/// Tunnels the supervisor was asked to maintain, and the adb it uses to check them.
struct Tunnels {
    adb: Option<PathBuf>,
    ports: Vec<u16>,
    /// How many times the keeper had to put each tunnel back, parallel to `ports`.
    reapplied: Vec<AtomicU32>,
}

pub fn runtime_dir() -> Result<PathBuf> {
    let dir = match std::env::var_os("AUTOLAB_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => {
            let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
            std::env::temp_dir().join(format!("autolab-{user}"))
        }
    };
    let io = |e: std::io::Error| Error::Other(format!("runtime dir {}: {e}", dir.display()));
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .map_err(io)?;
    let meta = fs::symlink_metadata(&dir).map_err(io)?;
    if !meta.is_dir() || meta.mode() & 0o077 != 0 {
        return Err(Error::Other(format!(
            "runtime dir {} must be a real directory with mode 0700",
            dir.display()
        )));
    }
    if let Some(home) = std::env::var_os("HOME")
        && let Ok(h) = fs::metadata(home)
        && h.uid() != meta.uid()
    {
        return Err(Error::Other(format!(
            "runtime dir {} is not owned by the current user",
            dir.display()
        )));
    }
    Ok(dir)
}

fn socket_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("session.sock"))
}

fn log_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("supervisor.log"))
}

fn no_session() -> Error {
    Error::BackendUnavailable(
        "no AutoLab session is running; start one with `autolab start`".into(),
    )
}

/// Send one request to the running supervisor.
pub fn request(req: &Request) -> Result<Response> {
    let path = socket_path()?;
    let mut stream = UnixStream::connect(&path).map_err(|e| match e.kind() {
        ErrorKind::NotFound | ErrorKind::ConnectionRefused => no_session(),
        _ => Error::Other(format!("cannot reach session at {}: {e}", path.display())),
    })?;
    let io = |e: std::io::Error| match e.kind() {
        ErrorKind::WouldBlock | ErrorKind::TimedOut => {
            Error::Timeout("session did not answer in time".into())
        }
        _ => Error::Other(format!("session connection failed: {e}")),
    };
    stream.set_read_timeout(Some(CLIENT_TIMEOUT)).map_err(io)?;
    stream.set_write_timeout(Some(CLIENT_TIMEOUT)).map_err(io)?;
    let mut line = serde_json::to_string(req).expect("request serializes");
    line.push('\n');
    stream.write_all(line.as_bytes()).map_err(io)?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).map_err(io)?;
    if reply.is_empty() {
        return Err(Error::Other(
            "session closed the connection without a reply".into(),
        ));
    }
    match serde_json::from_str(&reply) {
        Ok(Response::Error { code, message }) => Err(Error::from_code(code, message)),
        Ok(r) => Ok(r),
        Err(e) => Err(Error::Other(format!("unreadable session reply: {e}"))),
    }
}

pub fn status() -> Result<StatusInfo> {
    match request(&Request::Status)? {
        Response::Status(s) => Ok(s),
        other => Err(Error::Other(format!(
            "unexpected reply to status: {other:?}"
        ))),
    }
}

pub fn set_video_focus(enabled: bool) -> Result<OperationResult> {
    match request(&Request::FocusVideo { enabled })? {
        Response::Operation(r) => Ok(r),
        other => Err(Error::Other(format!(
            "unexpected reply to focus: {other:?}"
        ))),
    }
}

pub fn screenshot(path: &Path) -> Result<ScreenshotResult> {
    match request(&Request::Screenshot {
        path: path.to_path_buf(),
    })? {
        Response::Screenshot(r) => Ok(r),
        other => Err(Error::Other(format!(
            "unexpected reply to screenshot: {other:?}"
        ))),
    }
}

pub fn stop() -> Result<()> {
    match request(&Request::Stop)? {
        Response::Stopped => Ok(()),
        other => Err(Error::Other(format!("unexpected reply to stop: {other:?}"))),
    }
}

/// True if a live supervisor answers on the socket. Removes a stale socket file.
fn live_session_exists(sock: &Path) -> bool {
    match UnixStream::connect(sock) {
        Ok(_) => true,
        Err(e) => {
            if sock.exists()
                && matches!(e.kind(), ErrorKind::ConnectionRefused | ErrorKind::NotFound)
            {
                let _ = fs::remove_file(sock);
            }
            false
        }
    }
}

fn conflict() -> Error {
    Error::SessionConflict("an AutoLab session is already running; run `autolab stop` first".into())
}

/// Spawn the detached supervisor and wait until it answers, fails, or times out.
pub fn start(launch: &DhuLaunch, reverse: &[u16]) -> Result<StatusInfo> {
    let sock = socket_path()?;
    if live_session_exists(&sock) {
        return Err(conflict());
    }
    let log = log_path()?;
    let log_file = File::create(&log)
        .map_err(|e| Error::Other(format!("cannot create {}: {e}", log.display())))?;
    let exe = std::env::current_exe()
        .map_err(|e| Error::Other(format!("cannot locate own executable: {e}")))?;
    let mut cmd = Command::new(exe);
    cmd.arg("__supervise")
        .arg("--dhu-path")
        .arg(&launch.program)
        .arg("--serial")
        .arg(&launch.serial);
    if let Some(c) = &launch.config {
        cmd.arg("--config").arg(c);
    }
    if launch.headless {
        cmd.arg("--headless");
    }
    for port in reverse {
        cmd.arg("--reverse").arg(port.to_string());
    }
    cmd.stdin(Stdio::null())
        .stdout(
            log_file
                .try_clone()
                .map_err(|e| Error::Other(e.to_string()))?,
        )
        .stderr(log_file)
        .process_group(0);
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Other(format!("cannot spawn supervisor: {e}")))?;

    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if let Ok(s) = status() {
            return Ok(s);
        }
        if let Ok(Some(st)) = child.try_wait() {
            let text = fs::read_to_string(&log).unwrap_or_default();
            let msg = text.trim().to_string();
            let exit = st.code().and_then(|c| u8::try_from(c).ok()).unwrap_or(3);
            return Err(Error::from_code(if exit == 0 { 3 } else { exit }, msg));
        }
        if Instant::now() >= deadline {
            let _ = child.kill(); // supervisor dies, its stdin pipe to the DHU closes, DHU exits
            let _ = child.wait();
            return Err(Error::Timeout(format!(
                "session did not become ready within {}s (see {})",
                START_TIMEOUT.as_secs(),
                log.display()
            )));
        }
        thread::sleep(Duration::from_millis(200));
    }
}

/// Body of the detached supervisor process. Owns the DHU until stopped or until the DHU exits.
pub fn supervise(launch: DhuLaunch, reverse: Vec<u16>) -> Result<()> {
    let sock = socket_path()?;
    if live_session_exists(&sock) {
        return Err(conflict());
    }
    let mut backend = DhuBackend::launch(&launch)?;
    // The USB accessory switch drops adb tunnels, so they are only added once the DHU is attached.
    // A failure here ends the session (the DHU stops with it) rather than leaving one that looks
    // healthy while the app under test cannot reach its backend.
    let tunnels = Arc::new(setup_tunnels(&launch.serial, &reverse)?);
    if !tunnels.ports.is_empty() {
        let (serial, shared) = (launch.serial.clone(), Arc::clone(&tunnels));
        thread::spawn(move || keep_tunnels(serial, shared));
    }
    let listener = UnixListener::bind(&sock).map_err(|e| match e.kind() {
        ErrorKind::AddrInUse => conflict(),
        _ => Error::Other(format!("cannot bind {}: {e}", sock.display())),
    })?;
    let _ = fs::set_permissions(&sock, fs::Permissions::from_mode(0o600));
    listener
        .set_nonblocking(true)
        .map_err(|e| Error::Other(e.to_string()))?;
    println!("supervisor ready: dhu pid {}", backend.pid());

    let mut exit_reason = None;
    loop {
        if let Some(st) = backend.exit_status() {
            exit_reason = Some(format!("DHU exited ({st}); ending session"));
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if handle(&mut backend, &tunnels, stream) {
                    break;
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100))
            }
            Err(e) => {
                exit_reason = Some(format!("accept failed: {e}"));
                break;
            }
        }
    }
    let _ = fs::remove_file(&sock);
    let _ = backend.stop();
    match exit_reason {
        Some(r) => {
            println!("{r}");
            Ok(())
        }
        None => Ok(()),
    }
}

/// Handle one client. Returns true when the session should end.
fn handle(backend: &mut DhuBackend, tunnels: &Tunnels, stream: UnixStream) -> bool {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() || line.trim().is_empty() {
        return false;
    }
    let (response, stop) = match serde_json::from_str::<Request>(&line) {
        Err(e) => (
            Response::Error {
                code: 2,
                message: format!("invalid request: {e}"),
            },
            false,
        ),
        Ok(Request::Status) => (
            into_response(build_status(backend, tunnels).map(Response::Status)),
            false,
        ),
        Ok(Request::Capabilities) => (
            into_response(backend.capabilities().map(Response::Capabilities)),
            false,
        ),
        Ok(Request::FocusVideo { enabled }) => (
            into_response(backend.set_video_focus(enabled).map(Response::Operation)),
            false,
        ),
        Ok(Request::Screenshot { path }) => (
            into_response(backend.screenshot(&path).map(Response::Screenshot)),
            false,
        ),
        Ok(Request::Stop) => (
            into_response(backend.stop().map(|_| Response::Stopped)),
            true,
        ),
    };
    let mut out = serde_json::to_string(&response).expect("response serializes");
    out.push('\n');
    let _ = (&stream).write_all(out.as_bytes());
    stop
}

fn into_response(r: Result<Response>) -> Response {
    r.unwrap_or_else(|e| Response::Error {
        code: e.exit_code(),
        message: e.message().to_string(),
    })
}

fn build_status(backend: &mut DhuBackend, tunnels: &Tunnels) -> Result<StatusInfo> {
    let head_unit = backend.status()?;
    let serial = head_unit.serial.clone().unwrap_or_default();
    Ok(StatusInfo {
        head_unit,
        transport: "usb".into(),
        supervisor_pid: std::process::id(),
        dhu_pid: backend.pid(),
        tunnels: tunnel_status(&serial, tunnels),
    })
}

/// Live view from `adb reverse --list`, never from what was requested.
fn tunnel_status(serial: &str, tunnels: &Tunnels) -> Vec<TunnelStatus> {
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

/// Background upkeep: the phone drops `adb reverse` tunnels whenever it re-enumerates over USB, so
/// put back any that are missing. It acts only when adb answers and the list lacks the port, so
/// a phone that is mid-re-enumeration (adb not answering) is left alone instead of being hammered.
fn keep_tunnels(serial: String, tunnels: Arc<Tunnels>) {
    let Some(adb) = tunnels.adb.clone() else {
        return;
    };
    let every = std::env::var("AUTOLAB_TUNNEL_CHECK_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(TUNNEL_CHECK, Duration::from_millis);
    loop {
        thread::sleep(every);
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
                    println!("tunnel tcp:{port} was missing; re-applied");
                }
                Err(e) => println!("tunnel tcp:{port} is missing and re-applying failed: {e}"),
            }
        }
    }
}

fn setup_tunnels(serial: &str, ports: &[u16]) -> Result<Tunnels> {
    if ports.is_empty() {
        return Ok(Tunnels {
            adb: None,
            ports: Vec::new(),
            reapplied: Vec::new(),
        });
    }
    let adb = adb::find_adb(
        std::env::var("PATH").ok().as_deref(),
        &adb::sdk_roots_from_env(),
    )
    .ok_or_else(|| Error::BackendUnavailable("adb not found; it is needed for --reverse".into()))?;
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
    // A tunnel counts only once the device lists it.
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
    println!("tunnels active: {ports:?}");
    Ok(Tunnels {
        adb: Some(adb),
        ports: ports.to_vec(),
        reapplied: ports.iter().map(|_| AtomicU32::new(0)).collect(),
    })
}

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
use std::thread;
use std::time::{Duration, Instant};

use autolab_core::{Error, FocusState, HeadUnitBackend, OperationResult, Result};
use autolab_dhu::{DhuBackend, DhuLaunch};
use serde::{Deserialize, Serialize};

const START_TIMEOUT: Duration = Duration::from_secs(60);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Status,
    FocusVideo { enabled: bool },
    Stop,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    Status(StatusInfo),
    Operation(OperationResult),
    Stopped,
    Error { code: u8, message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusInfo {
    pub backend: String,
    pub state: String,
    pub transport: String,
    pub serial: String,
    pub supervisor_pid: u32,
    pub dhu_pid: u32,
    pub dhu_version: Option<String>,
    pub dhu_build: Option<String>,
    pub video_focus: VideoFocusStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoFocusStatus {
    pub requested: FocusState,
    pub backend_reported: FocusState,
    pub observed: FocusState,
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
pub fn start(launch: &DhuLaunch) -> Result<StatusInfo> {
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
pub fn supervise(launch: DhuLaunch) -> Result<()> {
    let sock = socket_path()?;
    if live_session_exists(&sock) {
        return Err(conflict());
    }
    let mut backend = DhuBackend::launch(&launch)?;
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
                if handle(&mut backend, &launch, stream) {
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
fn handle(backend: &mut DhuBackend, launch: &DhuLaunch, stream: UnixStream) -> bool {
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
            into_response(build_status(backend, launch).map(Response::Status)),
            false,
        ),
        Ok(Request::FocusVideo { enabled }) => (
            into_response(backend.set_video_focus(enabled).map(Response::Operation)),
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
        message: e.to_string(),
    })
}

fn build_status(backend: &mut DhuBackend, launch: &DhuLaunch) -> Result<StatusInfo> {
    let reported = backend.read_video_focus()?;
    Ok(StatusInfo {
        backend: "dhu".into(),
        state: "running".into(),
        transport: "usb".into(),
        serial: launch.serial.clone(),
        supervisor_pid: std::process::id(),
        dhu_pid: backend.pid(),
        dhu_version: backend.info().version.clone(),
        dhu_build: backend.info().build.clone(),
        video_focus: VideoFocusStatus {
            requested: backend.requested_video(),
            backend_reported: reported,
            observed: FocusState::Unknown,
        },
    })
}

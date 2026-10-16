//! The HTTP server: router, handlers, SSE stream, and the process-level `run`.

use std::convert::Infallible;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use autolab_core::{Capabilities, Error, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router, middleware};
use serde_json::json;

use crate::api::{
    DaemonInfo, DevicesResponse, FocusRequest, InputResponse, OperationResponse, ScreenshotRequest,
    ScreenshotResponse, StartRequest, StatusResponse, StoppedResponse, SwipeRequest, TapRequest,
};
use crate::auth::{self, Guard};
use crate::error::{ApiError, AppJson, method_not_allowed, not_found};
use crate::state::Daemon;
use crate::{client, log, runtime};

/// PRD section 16 default.
pub const DEFAULT_PORT: u16 = 8765;

/// Long enough for `autolab start` to spawn the daemon and send its first request.
const IDLE_GRACE: Duration = Duration::from_secs(30);

pub struct ServeOptions {
    pub runtime_dir: PathBuf,
    /// `None`: try 8765 and fall back to an ephemeral port if it is taken. `Some(p)`: exactly `p`
    /// (`Some(0)` asks the OS for any free port).
    pub port: Option<u16>,
    /// Exit when the session ends (set for a daemon started by `autolab start`).
    pub exit_when_idle: bool,
    /// How long an `exit_when_idle` daemon waits with no session before exiting. `None`: 30 s.
    pub idle_grace: Option<Duration>,
}

type ApiResult<T> = std::result::Result<Json<T>, ApiError>;

async fn status(State(d): State<Daemon>) -> ApiResult<StatusResponse> {
    Ok(Json(d.status().await?))
}

async fn capabilities(State(d): State<Daemon>) -> ApiResult<Capabilities> {
    Ok(Json(d.capabilities().await?))
}

async fn devices(State(d): State<Daemon>) -> ApiResult<DevicesResponse> {
    Ok(Json(d.devices().await?))
}

async fn session_start(
    State(d): State<Daemon>,
    AppJson(req): AppJson<StartRequest>,
) -> std::result::Result<(StatusCode, Json<StatusResponse>), ApiError> {
    Ok((StatusCode::CREATED, Json(d.start(req).await?)))
}

async fn session_stop(State(d): State<Daemon>) -> ApiResult<StoppedResponse> {
    d.stop().await?;
    Ok(Json(StoppedResponse { stopped: true }))
}

async fn focus_video(
    State(d): State<Daemon>,
    AppJson(req): AppJson<FocusRequest>,
) -> ApiResult<OperationResponse> {
    Ok(Json(d.focus_video(req.enabled).await?))
}

async fn screenshot(
    State(d): State<Daemon>,
    AppJson(req): AppJson<ScreenshotRequest>,
) -> ApiResult<ScreenshotResponse> {
    Ok(Json(d.screenshot(req.path).await?))
}

async fn input_tap(
    State(d): State<Daemon>,
    AppJson(req): AppJson<TapRequest>,
) -> ApiResult<InputResponse> {
    Ok(Json(d.tap(req.x, req.y).await?))
}

async fn input_swipe(
    State(d): State<Daemon>,
    AppJson(req): AppJson<SwipeRequest>,
) -> ApiResult<InputResponse> {
    Ok(Json(
        d.swipe(
            (req.x1, req.y1),
            (req.x2, req.y2),
            std::time::Duration::from_millis(req.duration_ms),
        )
        .await?,
    ))
}

async fn daemon_shutdown(State(d): State<Daemon>) -> Json<serde_json::Value> {
    d.shutdown().await;
    Json(json!({ "shuttingDown": true }))
}

/// Endpoints from the PRD that AutoLab does not implement answer with a structured error rather
/// than 404, so a client can tell "not implemented" from "wrong URL".
fn unimplemented_feature(what: &str) -> ApiError {
    ApiError(Error::Unsupported(format!("{what} is not implemented yet")))
}

async fn focus_audio() -> ApiError {
    unimplemented_feature("audio focus")
}
async fn focus_navigation() -> ApiError {
    unimplemented_feature("navigation focus")
}
async fn display_configure() -> ApiError {
    unimplemented_feature("display configuration")
}

fn sse_event(e: &crate::api::Event) -> SseEvent {
    SseEvent::default()
        .id(e.id.to_string())
        .event(e.kind.clone())
        .json_data(e)
        .unwrap_or_else(|_| SseEvent::default().event(e.kind.clone()).data("{}"))
}

async fn events(
    State(d): State<Daemon>,
) -> Sse<impl tokio_stream::Stream<Item = std::result::Result<SseEvent, Infallible>>> {
    // Subscribe before responding, so nothing emitted after the client sees `stream.opened` is missed.
    let mut rx = d.events().subscribe();
    let mut shutdown = d.shutdown_receiver();
    let (tx, out) = tokio::sync::mpsc::channel::<std::result::Result<SseEvent, Infallible>>(64);
    tokio::spawn(async move {
        if tx
            .send(Ok(SseEvent::default().event("stream.opened").data("{}")))
            .await
            .is_err()
        {
            return;
        }
        loop {
            tokio::select! {
                got = rx.recv() => match got {
                    Ok(e) => {
                        if tx.send(Ok(sse_event(&e))).await.is_err() {
                            return; // the client went away
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        let lag = SseEvent::default().event("stream.lagged").data(json!({ "missed": n }).to_string());
                        if tx.send(Ok(lag)).await.is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                },
                _ = async { let _ = shutdown.wait_for(|stop| *stop).await; } => {
                    // Everything emitted before shutdown is already buffered: deliver it, then end,
                    // or graceful shutdown would wait on this stream forever.
                    while let Ok(e) = rx.try_recv() {
                        if tx.send(Ok(sse_event(&e))).await.is_err() {
                            return;
                        }
                    }
                    return;
                }
            }
        }
    });
    Sse::new(tokio_stream::wrappers::ReceiverStream::new(out)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    )
}

pub fn router(daemon: Daemon) -> Router {
    Router::new()
        .route("/v1/status", get(status))
        .route("/v1/capabilities", get(capabilities))
        .route("/v1/devices", get(devices))
        .route("/v1/session/start", post(session_start))
        .route("/v1/session/stop", post(session_stop))
        .route("/v1/focus/video", post(focus_video))
        .route("/v1/focus/audio", post(focus_audio))
        .route("/v1/focus/navigation", post(focus_navigation))
        .route("/v1/display/configure", post(display_configure))
        .route("/v1/input/tap", post(input_tap))
        .route("/v1/input/swipe", post(input_swipe))
        .route("/v1/screenshot", post(screenshot))
        .route("/v1/events", get(events))
        .route("/v1/daemon/shutdown", post(daemon_shutdown))
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(daemon)
}

fn bind(port: Option<u16>) -> Result<TcpListener> {
    let try_bind = |p: u16| TcpListener::bind(("127.0.0.1", p));
    let listener = match port {
        Some(p) => try_bind(p).map_err(|e| {
            Error::BackendUnavailable(format!("cannot listen on 127.0.0.1:{p}: {e}"))
        })?,
        None => match try_bind(DEFAULT_PORT) {
            Ok(l) => l,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => try_bind(0).map_err(|e| {
                Error::BackendUnavailable(format!("cannot listen on a free port: {e}"))
            })?,
            Err(e) => {
                return Err(Error::BackendUnavailable(format!(
                    "cannot listen on 127.0.0.1:{DEFAULT_PORT}: {e}"
                )));
            }
        },
    };
    listener
        .set_nonblocking(true)
        .map_err(|e| Error::Other(e.to_string()))?;
    Ok(listener)
}

/// Run the daemon until it is told to stop (signal, `POST /v1/daemon/shutdown`, or the session
/// ending for an `exit_when_idle` daemon). Blocks the calling thread.
pub fn run(opts: ServeOptions) -> Result<()> {
    runtime::check_dir(&opts.runtime_dir)?;
    if let Some(existing) = runtime::read_daemon_file(&opts.runtime_dir)
        && client::Client::connect_in(&opts.runtime_dir).is_ok()
    {
        return Err(Error::SessionConflict(format!(
            "an AutoLab daemon is already running (pid {}); stop it first",
            existing.pid
        )));
    }
    let listener = bind(opts.port)?;
    let port = listener
        .local_addr()
        .map_err(|e| Error::Other(e.to_string()))?
        .port();
    let token = runtime::new_token()?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| Error::Other(format!("cannot start the async runtime: {e}")))?;
    rt.block_on(serve(opts, listener, port, token))
}

async fn serve(opts: ServeOptions, listener: TcpListener, port: u16, token: String) -> Result<()> {
    let info = DaemonInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        pid: std::process::id(),
        port,
        exit_when_idle: opts.exit_when_idle,
    };
    let daemon = Daemon::new(info.clone(), opts.idle_grace.unwrap_or(IDLE_GRACE));
    daemon.spawn_monitor();
    let guard = Arc::new(Guard {
        token: token.clone(),
        hosts: vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")],
    });
    let app = router(daemon.clone()).layer(middleware::from_fn_with_state(guard, auth::guard));
    let listener =
        tokio::net::TcpListener::from_std(listener).map_err(|e| Error::Other(e.to_string()))?;

    // Written only once the socket is bound, so a client that finds the file can always connect.
    runtime::write_daemon_file(
        &opts.runtime_dir,
        &runtime::DaemonFile {
            port,
            pid: info.pid,
            token: token.clone(),
            version: info.version.clone(),
        },
    )?;
    log::info(
        "daemon.started",
        json!({ "port": port, "pid": info.pid, "exitWhenIdle": info.exit_when_idle }),
    );

    tokio::spawn({
        let d = daemon.clone();
        async move {
            wait_for_signal().await;
            log::info("daemon.signal", json!({}));
            d.shutdown().await;
        }
    });

    let d = daemon.clone();
    let served = axum::serve(listener, app)
        .with_graceful_shutdown(async move { d.wait_shutdown().await })
        .await;
    runtime::remove_daemon_file_if_ours(&opts.runtime_dir, &token);
    log::info("daemon.stopped", json!({}));
    served.map_err(|e| Error::Other(format!("server error: {e}")))
}

async fn wait_for_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).ok();
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = async {
            match term.as_mut() {
                Some(t) => { t.recv().await; }
                None => std::future::pending::<()>().await,
            }
        } => {}
    }
}

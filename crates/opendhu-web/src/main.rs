//! `opendhu-web`: the head unit in a browser. It serves one page on loopback with the live picture,
//! touch, car keys and, under `aa test --web`, the run's progress. Like the desktop window and the
//! terminal viewer it is a client of the daemon's HTTP API; the browser never sees the daemon's token.

mod log;
mod session;
mod video;

use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use opendhu_core::{FocusState, Key, OpStatus};
use opendhu_daemon::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

const PAGE: &str = include_str!("index.html");
/// Tried first, so the address can be bookmarked; any free port if it is taken.
const DEFAULT_PORT: u16 = 8766;
/// After the run ends, how long pages have to receive that before the server exits.
const LINGER: Duration = Duration::from_secs(2);

#[derive(Parser, Debug)]
#[command(
    name = "opendhu-web",
    version,
    about = "Show the head unit in a browser"
)]
struct Args {
    /// Port on 127.0.0.1 [default: 8766, or any free port if that is taken]
    #[arg(long)]
    port: Option<u16>,
    /// Open the page in the default browser
    #[arg(long)]
    open: bool,
}

#[derive(Clone)]
struct App {
    video: Arc<video::Video>,
    log: Arc<log::Log>,
    /// `Host` values the page may be reached by: anything else is DNS rebinding.
    hosts: Arc<Vec<String>>,
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn bind(port: Option<u16>) -> std::io::Result<TcpListener> {
    let at = |p| TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, p)));
    match port {
        Some(p) => at(p),
        None => at(DEFAULT_PORT).or_else(|_| at(0)),
    }
}

fn run(args: Args) -> std::io::Result<()> {
    let listener = bind(args.port)?;
    let port = listener.local_addr()?.port();
    let feed = log_feed();
    let log = Arc::new(log::Log::new(feed.is_some()));
    let app = App {
        video: video::Video::start(),
        log: Arc::clone(&log),
        hosts: Arc::new(vec![
            format!("127.0.0.1:{port}"),
            format!("localhost:{port}"),
        ]),
    };
    if let Some(feed) = feed {
        let log = Arc::clone(&log);
        std::thread::spawn(move || {
            log.read(std::io::BufReader::new(feed), |dir| {
                session::set(dir);
            });
            std::thread::sleep(LINGER);
            std::process::exit(0);
        });
    }

    let url = format!("http://127.0.0.1:{port}/");
    eprintln!("Head unit in the browser: {url}");
    if args.open {
        open_browser(&url);
    }

    listener.set_nonblocking(true)?;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener)?;
            axum::serve(listener, router(app)).await
        })
}

fn router(app: App) -> Router {
    Router::new()
        .route("/", get(|| async { Html(PAGE) }))
        .route("/video.mjpg", get(mjpeg))
        .route("/api/status", get(status))
        .route("/api/log", get(log_events))
        .route("/api/screenshot", get(screenshot))
        .route("/api/tap", post(tap))
        .route("/api/swipe", post(swipe))
        .route("/api/key", post(key))
        .route("/api/focus", post(focus))
        .layer(middleware::from_fn_with_state(app.clone(), guard))
        .with_state(app)
}

/// Loopback is reachable by every web page the user has open, and this page can touch the phone.
/// A foreign `Host` is DNS rebinding and a foreign `Origin` is another site; both are refused. Posts
/// must also be JSON, which a plain cross-site form cannot send.
async fn guard(State(app): State<App>, req: Request, next: Next) -> Response {
    if let Err(why) = allowed(req.headers(), &app.hosts) {
        return (StatusCode::FORBIDDEN, Json(json!({ "error": why }))).into_response();
    }
    next.run(req).await
}

fn allowed(headers: &HeaderMap, hosts: &[String]) -> Result<(), &'static str> {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .filter(|h| hosts.iter().any(|ok| ok == h))
        .ok_or("unexpected Host header")?;
    match headers.get(header::ORIGIN).map(|o| o.to_str()) {
        None => Ok(()),
        Some(Ok(origin)) if origin == format!("http://{host}") => Ok(()),
        Some(_) => Err("cross-origin requests are not allowed"),
    }
}

/// `aa test --web` hands over the run's progress on the descriptor named by `OPENDHU_WEB_LOG_FD`.
/// Without it there is no log pane.
fn log_feed() -> Option<std::fs::File> {
    use std::os::fd::FromRawFd;

    let fd: i32 = std::env::var("OPENDHU_WEB_LOG_FD").ok()?.parse().ok()?;
    // SAFETY: the parent passed this descriptor to this process alone, for this purpose.
    Some(unsafe { std::fs::File::from_raw_fd(fd) })
}

fn open_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let opened = std::process::Command::new(opener)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if opened.is_err() {
        eprintln!("Could not open a browser; go to {url}");
    }
}

/// An error for the page: the daemon's message, or why no daemon could be reached.
struct Failure(StatusCode, String);

impl Failure {
    fn daemon(e: impl std::fmt::Display) -> Self {
        Self(StatusCode::BAD_GATEWAY, e.to_string())
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

/// Run a blocking daemon call off the async workers.
async fn call<T: Send + 'static>(
    f: impl FnOnce(Client) -> opendhu_core::Result<T> + Send + 'static,
) -> Result<T, Failure> {
    tokio::task::spawn_blocking(move || session::client().and_then(f))
        .await
        .map_err(Failure::daemon)?
        .map_err(Failure::daemon)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusBody {
    /// `None`: no session to show.
    session: Option<SessionBody>,
    /// Why there is no picture, when there is a session but the video failed.
    video_problem: Option<String>,
    /// Whether a test run feeds this page, and whether it has finished.
    run: Option<RunBody>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionBody {
    summary: String,
    width: u32,
    height: u32,
    /// What the phone last reported: `on`, `off` or `unknown`.
    video_focus: FocusState,
}

#[derive(Serialize)]
struct RunBody {
    ended: bool,
}

async fn status(State(app): State<App>) -> Json<StatusBody> {
    let session = call(|c| c.status()).await.ok().and_then(|s| {
        let head = s.session?.details?.head_unit;
        let (width, height) = match (head.screen_width, head.screen_height) {
            (Some(w), Some(h)) => (w.into(), h.into()),
            _ => (
                opendhu_native::protocol::SCREEN_WIDTH.into(),
                opendhu_native::protocol::SCREEN_HEIGHT.into(),
            ),
        };
        Some(SessionBody {
            summary: format!(
                "{} {:?} {width}x{height} focus {:?}",
                head.backend, head.state, head.video_focus.observed
            ),
            width,
            height,
            video_focus: head.video_focus.observed,
        })
    });
    // Without a session the video cannot connect; its error would only repeat that.
    let video_problem = session.as_ref().and(app.video.problem());
    Json(StatusBody {
        session,
        video_problem,
        run: app.log.attached().then(|| RunBody {
            ended: app.log.ended(),
        }),
    })
}

/// The picture as `multipart/x-mixed-replace`, which an `<img>` plays as it arrives. A still
/// screen sends nothing new, so the newest frame is sent again every few seconds to keep the
/// connection from looking dead to proxies.
async fn mjpeg(State(app): State<App>) -> Response {
    const BOUNDARY: &str = "opendhu-frame";
    let mut frames = app.video.subscribe();
    let (tx, out) = tokio::sync::mpsc::channel::<Result<Vec<u8>, Infallible>>(2);
    tokio::spawn(async move {
        loop {
            let jpeg = frames.borrow_and_update().clone();
            if let Some(jpeg) = jpeg {
                let mut part = format!(
                    "--{BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                    jpeg.len()
                )
                .into_bytes();
                part.extend_from_slice(&jpeg);
                part.extend_from_slice(b"\r\n");
                if tx.send(Ok(part)).await.is_err() {
                    return;
                }
            }
            match tokio::time::timeout(Duration::from_secs(5), frames.changed()).await {
                Ok(Ok(())) | Err(_) => {}
                Ok(Err(_)) => return,
            }
            if tx.is_closed() {
                return;
            }
        }
    });
    Response::builder()
        .header(
            header::CONTENT_TYPE,
            format!("multipart/x-mixed-replace; boundary={BOUNDARY}"),
        )
        .header(header::CACHE_CONTROL, "no-store")
        .body(axum::body::Body::from_stream(
            tokio_stream::wrappers::ReceiverStream::new(out),
        ))
        .expect("static headers are valid")
}

/// The run so far, then each new line, as server-sent events of `log::Event` JSON.
async fn log_events(
    State(app): State<App>,
) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, Infallible>>> {
    let (past, mut next) = app.log.follow();
    let (tx, out) = tokio::sync::mpsc::channel::<Result<SseEvent, Infallible>>(256);
    // Without a test run there is nothing to follow: the stream ends at once.
    let attached = app.log.attached();
    let event = |e: &log::Event| {
        SseEvent::default().data(serde_json::to_string(e).expect("events serialize"))
    };
    tokio::spawn(async move {
        for e in &past {
            if tx.send(Ok(event(e))).await.is_err() {
                return;
            }
        }
        if !attached || past.last() == Some(&log::Event::End) {
            return;
        }
        loop {
            let e = match next.recv().await {
                Ok(e) => e,
                // Too slow to keep up: carry on with what is newest rather than stop.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            };
            let end = e == log::Event::End;
            if tx.send(Ok(event(&e))).await.is_err() || end {
                return;
            }
        }
    });
    Sse::new(tokio_stream::wrappers::ReceiverStream::new(out)).keep_alive(KeepAlive::default())
}

/// A PNG of the screen, straight to the browser as a download.
async fn screenshot() -> Result<Response, Failure> {
    let png = call(|c| {
        let path = std::env::temp_dir().join(format!(
            "opendhu-web-{}-{}.png",
            std::process::id(),
            unix_nanos()
        ));
        let taken = c.screenshot(&path);
        let bytes = std::fs::read(&path);
        let _ = std::fs::remove_file(&path);
        taken?;
        bytes.map_err(|e| opendhu_core::Error::Other(format!("cannot read the screenshot: {e}")))
    })
    .await?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/png".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"aa-screenshot-{}.png\"",
                    unix_nanos() / 1_000_000_000
                ),
            ),
        ],
        png,
    )
        .into_response())
}

fn unix_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos())
}

#[derive(Deserialize)]
struct Tap {
    x: u32,
    y: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Swipe {
    from: (u32, u32),
    to: (u32, u32),
    duration_ms: u64,
}

#[derive(Deserialize)]
struct KeyPress {
    key: String,
    #[serde(default)]
    long: bool,
}

#[derive(Deserialize)]
struct Focus {
    on: bool,
}

type Done = Result<Json<serde_json::Value>, Failure>;

/// What the daemon says happened. It answers 200 even when the phone refused or never confirmed,
/// so the status decides: anything short of success is an error for the page, with the evidence
/// that explains it. A success carries the evidence too, since `accepted` only means "sent".
fn outcome(status: OpStatus, evidence: &[String]) -> Done {
    let detail = evidence.last().cloned().unwrap_or_default();
    if status.is_success() {
        return Ok(Json(json!({ "status": status, "detail": detail })));
    }
    let word = serde_json::to_value(status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
        .replace('_', " ");
    Err(Failure(
        StatusCode::UNPROCESSABLE_ENTITY,
        if detail.is_empty() {
            word
        } else {
            format!("{word}: {detail}")
        },
    ))
}

async fn tap(Json(t): Json<Tap>) -> Done {
    let r = call(move |c| c.tap(t.x, t.y)).await?;
    outcome(r.result.status, &r.result.evidence)
}

async fn swipe(Json(s): Json<Swipe>) -> Done {
    let held = Duration::from_millis(s.duration_ms.clamp(50, 10_000));
    let r = call(move |c| c.swipe(s.from, s.to, held)).await?;
    outcome(r.result.status, &r.result.evidence)
}

async fn key(Json(k): Json<KeyPress>) -> Done {
    let key: Key = k
        .key
        .parse()
        .map_err(|m| Failure(StatusCode::BAD_REQUEST, m))?;
    let r = call(move |c| c.key(key, k.long)).await?;
    outcome(r.result.status, &r.result.evidence)
}

async fn focus(Json(f): Json<Focus>) -> Done {
    let r = call(move |c| c.focus_video(f.on)).await?;
    outcome(r.result.status, &r.result.evidence)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(header::HeaderName, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(k, v.parse().unwrap());
        }
        h
    }

    fn hosts() -> Vec<String> {
        vec!["127.0.0.1:8766".into(), "localhost:8766".into()]
    }

    #[test]
    fn only_this_servers_own_address_is_served() {
        assert!(allowed(&headers(&[(header::HOST, "127.0.0.1:8766")]), &hosts()).is_ok());
        assert!(allowed(&headers(&[(header::HOST, "localhost:8766")]), &hosts()).is_ok());
        // DNS rebinding: a name that resolves to loopback.
        assert!(allowed(&headers(&[(header::HOST, "evil.example:8766")]), &hosts()).is_err());
        assert!(allowed(&HeaderMap::new(), &hosts()).is_err());
    }

    #[test]
    fn the_page_itself_may_post_but_other_sites_may_not() {
        let own = headers(&[
            (header::HOST, "127.0.0.1:8766"),
            (header::ORIGIN, "http://127.0.0.1:8766"),
        ]);
        assert!(allowed(&own, &hosts()).is_ok());
        let foreign = headers(&[
            (header::HOST, "127.0.0.1:8766"),
            (header::ORIGIN, "https://evil.example"),
        ]);
        assert!(allowed(&foreign, &hosts()).is_err());
        let null = headers(&[(header::HOST, "127.0.0.1:8766"), (header::ORIGIN, "null")]);
        assert!(allowed(&null, &hosts()).is_err());
    }

    #[test]
    fn an_unconfirmed_or_refused_input_is_an_error_not_a_success() {
        let evidence = vec![
            "sent VideoFocusIndication UNFOCUSED (unrequested)".to_string(),
            "the phone did not confirm within 3s; it still reports video on".to_string(),
        ];
        let Err(Failure(code, message)) = outcome(OpStatus::TimedOut, &evidence) else {
            panic!("a timeout must not look like success");
        };
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            message,
            "timed out: the phone did not confirm within 3s; it still reports video on"
        );
        assert!(outcome(OpStatus::Failed, &[]).is_err());
        assert!(outcome(OpStatus::Unsupported, &[]).is_err());
        let Ok(Json(ok)) = outcome(OpStatus::Accepted, &["sent home".to_string()]) else {
            panic!("accepted is a success");
        };
        assert_eq!(ok, json!({ "status": "accepted", "detail": "sent home" }));
    }

    #[test]
    fn page_talks_only_to_routes_that_exist() {
        for route in [
            "/video.mjpg",
            "/api/status",
            "/api/log",
            "/api/screenshot",
            "/api/tap",
            "/api/swipe",
            "/api/key",
            "/api/focus",
        ] {
            assert!(PAGE.contains(route), "the page never uses {route}");
        }
    }

    #[test]
    fn page_key_names_are_car_buttons() {
        // Every `data-key` / key binding in the page must parse, or the button does nothing.
        let names: Vec<&str> = PAGE
            .split("data-key=\"")
            .skip(1)
            .filter_map(|s| s.split('"').next())
            .collect();
        assert!(!names.is_empty());
        for name in names {
            assert!(name.parse::<Key>().is_ok(), "{name} is not a car button");
        }
    }
}

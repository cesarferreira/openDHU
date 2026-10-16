//! The real daemon, in-process on an ephemeral port, driving the fake DHU over real HTTP.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use autolab_core::{Error, FocusState};
use autolab_daemon::api::{Phase, StartRequest};
use autolab_daemon::{Client, ServeOptions, runtime};
use serde_json::Value;

static N: AtomicUsize = AtomicUsize::new(0);

fn fake_dhu() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../autolab-dhu/tests/fixtures/fake-dhu.sh"
    ))
}

fn alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .unwrap()
        .status
        .success()
}

fn wait_until(limit: Duration, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        if f() {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    f()
}

struct TestDaemon {
    dir: PathBuf,
    handle: Option<JoinHandle<autolab_core::Result<()>>>,
    client: Client,
}

impl TestDaemon {
    fn start(exit_when_idle: bool) -> Self {
        Self::start_with_grace(exit_when_idle, None)
    }

    fn start_with_grace(exit_when_idle: bool, idle_grace: Option<Duration>) -> Self {
        let dir = PathBuf::from(format!(
            "/tmp/al-d-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        Self::start_in(dir, exit_when_idle, idle_grace)
    }

    fn start_in(dir: PathBuf, exit_when_idle: bool, idle_grace: Option<Duration>) -> Self {
        let d = dir.clone();
        let handle = thread::spawn(move || {
            autolab_daemon::run(ServeOptions {
                runtime_dir: d,
                port: Some(0),
                exit_when_idle,
                idle_grace,
            })
        });
        assert!(
            wait_until(Duration::from_secs(10), || Client::connect_in(&dir).is_ok()),
            "daemon never became reachable"
        );
        let client = Client::connect_in(&dir).unwrap();
        Self {
            dir,
            handle: Some(handle),
            client,
        }
    }

    fn port(&self) -> u16 {
        runtime::read_daemon_file(&self.dir).unwrap().port
    }

    fn token(&self) -> String {
        runtime::read_daemon_file(&self.dir).unwrap().token
    }

    fn dhu(&self) -> StartRequest {
        StartRequest {
            dhu_path: Some(fake_dhu()),
            serial: Some("FAKE0001".into()),
            ..Default::default()
        }
    }

    fn finished(&self) -> bool {
        self.handle.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        if !self.finished() {
            let _ = self.client.shutdown();
        }
        if let Some(h) = self.handle.take() {
            let end = Instant::now() + Duration::from_secs(20);
            while !h.is_finished() && Instant::now() < end {
                thread::sleep(Duration::from_millis(25));
            }
            if h.is_finished() {
                let _ = h.join();
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// One raw HTTP/1.1 exchange, so headers (Host, Origin, Authorization) are fully controlled.
fn raw(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: Option<&str>,
) -> (u16, Value) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        req.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            b.len()
        ));
    }
    req.push_str("\r\n");
    if let Some(b) = body {
        req.push_str(b);
    }
    s.write_all(req.as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    let status: u16 = out.split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = out.split("\r\n\r\n").nth(1).unwrap_or("");
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

fn authed(d: &TestDaemon) -> Vec<(&'static str, String)> {
    vec![
        ("Host", format!("127.0.0.1:{}", d.port())),
        ("Authorization", format!("Bearer {}", d.token())),
    ]
}

fn error_of(v: &Value) -> (&str, u64) {
    (
        v["error"]["code"].as_str().unwrap_or("?"),
        v["error"]["exitCode"].as_u64().unwrap_or(0),
    )
}

// ---- security ------------------------------------------------------------------------------

#[test]
fn every_request_needs_the_token_a_local_host_and_no_origin() {
    let d = TestDaemon::start(false);
    let (port, tok) = (d.port(), d.token());
    let host = ("Host", format!("127.0.0.1:{port}"));

    let (st, body) = raw(port, "GET", "/v1/status", std::slice::from_ref(&host), None);
    assert_eq!((st, error_of(&body).0), (401, "unauthorized"), "{body}");
    let (st, _) = raw(
        port,
        "GET",
        "/v1/status",
        &[host.clone(), ("Authorization", "Bearer nope".into())],
        None,
    );
    assert_eq!(st, 401);
    let (st, _) = raw(
        port,
        "GET",
        "/v1/status",
        &[host.clone(), ("Authorization", format!("Bearer {tok}x"))],
        None,
    );
    assert_eq!(st, 401, "a token with extra characters must not match");
    let (st, _) = raw(
        port,
        "GET",
        "/v1/status",
        &[host.clone(), ("Authorization", tok.clone())],
        None,
    );
    assert_eq!(st, 401, "the Bearer scheme is required");
    let (st, _) = raw(
        port,
        "GET",
        "/no/such/route",
        std::slice::from_ref(&host),
        None,
    );
    assert_eq!(
        st, 401,
        "auth is checked before routing, so routes cannot be probed"
    );

    let good = ("Authorization", format!("Bearer {tok}"));
    let (st, body) = raw(
        port,
        "GET",
        "/v1/status",
        &[("Host", "evil.example".into()), good.clone()],
        None,
    );
    assert_eq!(
        (st, error_of(&body).0),
        (403, "forbidden"),
        "DNS rebinding: wrong Host"
    );
    let (st, _) = raw(
        port,
        "GET",
        "/v1/status",
        &[
            host.clone(),
            good.clone(),
            ("Origin", "http://evil.example".into()),
        ],
        None,
    );
    assert_eq!(
        st, 403,
        "cross-site requests carry an Origin and are refused"
    );
    let (st, _) = raw(
        port,
        "GET",
        "/v1/status",
        &[("Host", format!("localhost:{port}")), good.clone()],
        None,
    );
    assert_eq!(st, 200, "localhost is an accepted name for the daemon");
    let (st, _) = raw(port, "GET", "/v1/status", &[host, good], None);
    assert_eq!(st, 200);
}

#[test]
fn daemon_file_and_runtime_dir_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let d = TestDaemon::start(false);
    let file = std::fs::metadata(runtime::daemon_file_path(&d.dir))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    let dir = std::fs::metadata(&d.dir).unwrap().permissions().mode() & 0o777;
    assert_eq!((file, dir), (0o600, 0o700));
    let f = runtime::read_daemon_file(&d.dir).unwrap();
    assert_eq!(f.pid, std::process::id());
    assert_eq!(f.token.len(), 64);
}

// ---- uniform errors ------------------------------------------------------------------------

#[test]
fn framework_failures_use_the_same_error_schema() {
    let d = TestDaemon::start(false);
    let h = authed(&d);
    let p = d.port();

    let (st, b) = raw(p, "GET", "/v1/nothing", &h, None);
    assert_eq!((st, error_of(&b).0), (404, "not_found"), "{b}");
    let (st, b) = raw(p, "GET", "/v1/focus/video", &h, None);
    assert_eq!((st, error_of(&b).0), (405, "method_not_allowed"), "{b}");
    let (st, b) = raw(p, "POST", "/v1/focus/video", &h, Some("{not json"));
    assert_eq!((st, error_of(&b)), (400, ("invalid_args", 2)), "{b}");
    let (st, b) = raw(
        p,
        "POST",
        "/v1/focus/video",
        &h,
        Some(r#"{"enabled":true,"extra":1}"#),
    );
    assert_eq!(
        (st, error_of(&b).0),
        (400, "invalid_args"),
        "unknown field: {b}"
    );
    let (st, b) = raw(p, "POST", "/v1/focus/video", &h, Some("{}"));
    assert_eq!(
        (st, error_of(&b).0),
        (400, "invalid_args"),
        "missing field: {b}"
    );
    let (st, b) = raw(
        p,
        "POST",
        "/v1/focus/video",
        &h,
        Some(r#"{"enabled":"yes"}"#),
    );
    assert_eq!(
        (st, error_of(&b).0),
        (400, "invalid_args"),
        "wrong type: {b}"
    );
    assert!(b["error"]["message"].as_str().unwrap().len() > 5, "{b}");
}

#[test]
fn endpoints_that_exist_in_the_prd_but_not_here_say_so() {
    let d = TestDaemon::start(false);
    let h = authed(&d);
    for path in [
        "/v1/focus/audio",
        "/v1/focus/navigation",
        "/v1/display/configure",
        "/v1/input/tap",
        "/v1/input/swipe",
    ] {
        let (st, b) = raw(d.port(), "POST", path, &h, Some("{}"));
        assert_eq!((st, error_of(&b)), (501, ("unsupported", 5)), "{path}: {b}");
        assert!(
            b["error"]["message"]
                .as_str()
                .unwrap()
                .contains("not implemented"),
            "{b}"
        );
    }
}

// ---- lifecycle -----------------------------------------------------------------------------

#[test]
fn full_session_lifecycle_over_http() {
    let d = TestDaemon::start(false);
    let st = d.client.status().unwrap();
    assert!(st.session.is_none());
    assert_eq!(st.daemon.pid, std::process::id());
    assert_eq!(st.daemon.port, d.port());

    // No session yet: operations are 503 backend_unavailable with exit code 3.
    let (code, b) = raw(d.port(), "GET", "/v1/capabilities", &authed(&d), None);
    assert_eq!((code, error_of(&b)), (503, ("backend_unavailable", 3)));
    assert_eq!(d.client.focus_video(false).unwrap_err().exit_code(), 3);

    let started = d.client.start(&d.dhu()).unwrap();
    let s = started.session.unwrap();
    assert_eq!(s.phase, Phase::Running);
    assert_eq!(s.session_id.len(), 6);
    let details = s.details.unwrap();
    assert_eq!(details.head_unit.backend, "dhu");
    assert_eq!(details.head_unit.serial.as_deref(), Some("FAKE0001"));
    assert_eq!(details.transport, "usb");

    let again = d.client.start(&d.dhu()).unwrap_err();
    assert!(matches!(again, Error::SessionConflict(_)), "{again}");

    let caps = d.client.capabilities().unwrap();
    assert_eq!(caps.backend, "dhu");
    assert_eq!(
        serde_json::to_value(&caps).unwrap()["videoFocus"],
        "supported"
    );
    assert_eq!(
        serde_json::to_value(&caps).unwrap()["screenRecording"],
        "unsupported"
    );

    let off = d.client.focus_video(false).unwrap();
    assert_eq!(off.operation_id, "op-1");
    assert_eq!(off.result.backend_reported, FocusState::Off);
    assert_eq!(off.result.observed, FocusState::Unknown);
    let on = d.client.focus_video(true).unwrap();
    assert_eq!(on.operation_id, "op-2");

    let shot = std::env::temp_dir().join(format!("al-http-{}.png", std::process::id()));
    let r = d.client.screenshot(&shot).unwrap();
    assert_eq!((r.result.width, r.result.height), (1280, 720));
    assert_eq!(r.operation_id, "op-3");
    assert!(shot.is_file());
    let _ = std::fs::remove_file(&shot);

    let bad = d
        .client
        .screenshot(std::path::Path::new("relative.png"))
        .unwrap_err();
    assert_eq!(bad.exit_code(), 2, "{bad}");

    d.client.stop().unwrap();
    assert!(d.client.status().unwrap().session.is_none());
    assert_eq!(
        d.client.stop().unwrap_err().exit_code(),
        3,
        "stopping with no session"
    );
}

#[test]
fn devices_endpoint_answers_without_a_session() {
    let d = TestDaemon::start(false);
    // The result depends on the machine's adb; the contract is that it answers in the schema.
    match d.client.devices() {
        Ok(list) => assert!(list.devices.iter().all(|x| !x.serial.is_empty())),
        Err(e) => assert!(matches!(e, Error::BackendUnavailable(_)), "{e}"),
    }
}

// ---- events --------------------------------------------------------------------------------

struct Sse {
    rx: mpsc::Receiver<(String, Value)>,
}

impl Sse {
    fn open(d: &TestDaemon) -> Self {
        let (base, token) = d.client.endpoint();
        let (url, auth) = (format!("{base}/v1/events"), format!("Bearer {token}"));
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_global(None)
                .build()
                .into();
            let Ok(mut resp) = agent.get(&url).header("Authorization", auth).call() else {
                return;
            };
            let mut lines = BufReader::new(resp.body_mut().as_reader()).lines();
            let (mut event, mut data) = (String::new(), String::new());
            while let Some(Ok(line)) = lines.next() {
                if let Some(v) = line.strip_prefix("event: ") {
                    event = v.into();
                } else if let Some(v) = line.strip_prefix("data: ") {
                    data = v.into();
                } else if line.is_empty() && !event.is_empty() {
                    let v = serde_json::from_str(&data).unwrap_or(Value::Null);
                    if tx.send((std::mem::take(&mut event), v)).is_err() {
                        return;
                    }
                    data.clear();
                }
            }
            let _ = tx.send(("stream.closed".into(), Value::Null));
        });
        let s = Self { rx };
        let (first, _) = s.next(Duration::from_secs(10));
        assert_eq!(first, "stream.opened");
        s
    }

    fn next(&self, limit: Duration) -> (String, Value) {
        self.rx.recv_timeout(limit).expect("event did not arrive")
    }

    fn until(&self, kind: &str) -> Vec<(String, Value)> {
        let mut seen = Vec::new();
        loop {
            let ev = self.next(Duration::from_secs(15));
            let done = ev.0 == kind;
            seen.push(ev);
            if done {
                return seen;
            }
        }
    }
}

#[test]
fn events_distinguish_requested_from_accepted_and_never_claim_confirmation() {
    let d = TestDaemon::start(false);
    let sse = Sse::open(&d);
    d.client.start(&d.dhu()).unwrap();
    d.client.focus_video(false).unwrap();
    d.client.stop().unwrap();
    let seen = sse.until("session.stopped");
    let kinds: Vec<&str> = seen.iter().map(|(k, _)| k.as_str()).collect();

    let pos = |k: &str| {
        kinds
            .iter()
            .position(|x| *x == k)
            .unwrap_or_else(|| panic!("no {k} in {kinds:?}"))
    };
    assert!(pos("session.starting") < pos("session.started"));
    assert!(pos("session.started") < pos("focus.video.requested"));
    assert!(
        pos("focus.video.requested") < pos("focus.video.accepted"),
        "{kinds:?}"
    );
    assert!(pos("focus.video.accepted") < pos("session.stopping"));
    assert!(pos("session.stopping") < pos("session.stopped"));

    let req = &seen[pos("focus.video.requested")].1;
    let acc = &seen[pos("focus.video.accepted")].1;
    assert_eq!(req["data"]["operationId"], acc["data"]["operationId"]);
    assert_eq!(req["data"]["enabled"], false);
    assert_eq!(acc["data"]["backendReported"], "off");

    let ids: Vec<u64> = seen.iter().filter_map(|(_, v)| v["id"].as_u64()).collect();
    assert!(
        ids.windows(2).all(|w| w[0] < w[1]),
        "ids must increase: {ids:?}"
    );
    assert!(
        seen.iter()
            .filter_map(|(_, v)| v["timestamp"].as_str())
            .all(|t| t.len() == 20 && t.ends_with('Z'))
    );
    assert!(
        kinds
            .iter()
            .all(|k| !k.contains("observed") && !k.contains("confirmed")),
        "no event may claim phone confirmation: {kinds:?}"
    );
}

#[test]
fn the_final_events_are_delivered_before_the_stream_ends_on_an_exiting_daemon() {
    // Regression: on a daemon that exits when its session ends, the shutdown signal raced the last
    // event and the subscriber never saw `session.stopped`.
    for _ in 0..5 {
        let d = TestDaemon::start(true);
        let sse = Sse::open(&d);
        d.client.start(&d.dhu()).unwrap();
        d.client.stop().unwrap();
        let seen = sse.until("stream.closed");
        let kinds: Vec<&str> = seen.iter().map(|(k, _)| k.as_str()).collect();
        let stopped = kinds.iter().position(|k| *k == "session.stopped");
        let closed = kinds.iter().position(|k| *k == "stream.closed");
        assert!(
            stopped.is_some_and(|s| s < closed.unwrap()),
            "session.stopped was lost: {kinds:?}"
        );
    }
}

#[test]
fn the_event_stream_ends_when_the_daemon_shuts_down() {
    let d = TestDaemon::start(false);
    let sse = Sse::open(&d);
    d.client.shutdown().unwrap();
    let ev = sse.until("stream.closed");
    assert_eq!(
        ev.last().unwrap().0,
        "stream.closed",
        "an open subscriber must not block shutdown"
    );
    assert!(
        wait_until(Duration::from_secs(10), || d.finished()),
        "daemon never exited"
    );
}

// ---- concurrency ---------------------------------------------------------------------------

#[test]
fn parallel_clients_cannot_corrupt_the_session() {
    let d = TestDaemon::start(false);
    d.client.start(&d.dhu()).unwrap();

    let threads: Vec<_> = (0..20)
        .map(|i| {
            let c = d.client.clone();
            thread::spawn(move || c.focus_video(i % 2 == 0).unwrap())
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();

    assert!(results.iter().all(|r| r.result.status.is_success()));
    let ids: HashSet<&str> = results.iter().map(|r| r.operation_id.as_str()).collect();
    assert_eq!(ids.len(), 20, "operation ids must be unique");
    let video = d
        .client
        .status()
        .unwrap()
        .session
        .unwrap()
        .details
        .unwrap()
        .head_unit
        .video_focus;
    assert_ne!(video.requested, FocusState::Unknown);
    assert_eq!(
        video.requested, video.backend_reported,
        "AutoLab's idea and the DHU's must agree"
    );
}

#[test]
fn two_simultaneous_starts_yield_exactly_one_session() {
    let d = TestDaemon::start(false);
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let (c, req) = (d.client.clone(), d.dhu());
            thread::spawn(move || c.start(&req))
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "{results:?}"
    );
    let loser = results.iter().find_map(|r| r.as_ref().err()).unwrap();
    assert_eq!(loser.exit_code(), 8);
}

#[test]
fn status_answers_immediately_while_a_slow_start_is_in_progress() {
    let d = TestDaemon::start(false);
    let slow = StartRequest {
        config: Some("/tmp/slow.ini".into()),
        ..d.dhu()
    }; // fake DHU waits 3 s
    let c = d.client.clone();
    let starting = thread::spawn(move || c.start(&slow));

    let mut saw_starting = false;
    let begun = Instant::now();
    while begun.elapsed() < Duration::from_millis(2500) && !saw_starting {
        let t = Instant::now();
        let st = d.client.status().unwrap();
        assert!(
            t.elapsed() < Duration::from_secs(1),
            "status blocked behind the start"
        );
        if let Some(s) = st.session {
            if s.phase == Phase::Starting {
                assert!(s.details.is_none());
                saw_starting = true;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(saw_starting, "never observed the starting phase");
    assert_eq!(
        d.client.start(&d.dhu()).unwrap_err().exit_code(),
        8,
        "second start during Starting"
    );
    assert_eq!(
        d.client.stop().unwrap_err().exit_code(),
        8,
        "cannot stop a session that is still starting"
    );
    starting.join().unwrap().unwrap();
    assert_eq!(
        d.client.status().unwrap().session.unwrap().phase,
        Phase::Running
    );
}

// ---- lifecycle policy ----------------------------------------------------------------------

#[test]
fn a_daemon_started_for_one_session_exits_when_it_stops() {
    let d = TestDaemon::start(true);
    d.client.start(&d.dhu()).unwrap();
    let st = d.client.status().unwrap();
    assert!(st.daemon.exit_when_idle);
    d.client.stop().unwrap();
    assert!(
        wait_until(Duration::from_secs(10), || d.finished()),
        "ephemeral daemon should exit"
    );
    assert!(
        !runtime::daemon_file_path(&d.dir).exists(),
        "daemon.json must be removed"
    );
}

#[test]
fn a_failed_start_also_ends_an_ephemeral_daemon() {
    let d = TestDaemon::start(true);
    let err = d
        .client
        .start(&StartRequest {
            dhu_path: Some("/no/such/dhu".into()),
            serial: Some("X".into()),
            ..Default::default()
        })
        .unwrap_err();
    assert_eq!(err.exit_code(), 3);
    assert!(err.to_string().contains("/no/such/dhu"));
    assert!(wait_until(Duration::from_secs(10), || d.finished()));
}

#[test]
fn a_persistent_daemon_survives_the_session_and_can_start_another() {
    let d = TestDaemon::start(false);
    d.client.start(&d.dhu()).unwrap();
    d.client.stop().unwrap();
    assert!(!d.finished());
    d.client.start(&d.dhu()).unwrap();
    assert_eq!(
        d.client.status().unwrap().session.unwrap().phase,
        Phase::Running
    );
}

#[test]
fn when_the_dhu_dies_the_session_ends_and_an_event_says_why() {
    let d = TestDaemon::start(false);
    let sse = Sse::open(&d);
    d.client.start(&d.dhu()).unwrap();
    let dhu = d
        .client
        .status()
        .unwrap()
        .session
        .unwrap()
        .details
        .unwrap()
        .dhu_pid;
    std::process::Command::new("kill")
        .args(["-9", &dhu.to_string()])
        .status()
        .unwrap();

    let seen = sse.until("session.ended");
    let ended = &seen.last().unwrap().1;
    assert!(
        ended["data"]["reason"]
            .as_str()
            .unwrap()
            .contains("the DHU exited"),
        "{ended}"
    );
    assert!(
        d.client.status().unwrap().session.is_none(),
        "no stale session may be reported"
    );
    assert!(!d.finished(), "a persistent daemon stays up");
}

#[test]
fn shutdown_stops_the_session_the_dhu_and_removes_the_file() {
    let d = TestDaemon::start(false);
    d.client.start(&d.dhu()).unwrap();
    let dhu = d
        .client
        .status()
        .unwrap()
        .session
        .unwrap()
        .details
        .unwrap()
        .dhu_pid;
    d.client.shutdown().unwrap();
    assert!(wait_until(Duration::from_secs(15), || d.finished()));
    assert!(
        wait_until(Duration::from_secs(5), || !alive(dhu)),
        "the DHU must not outlive the daemon"
    );
    assert!(!runtime::daemon_file_path(&d.dir).exists());
}

#[test]
fn a_stale_daemon_file_is_not_trusted_and_does_not_block_a_new_daemon() {
    let dir = PathBuf::from(format!("/tmp/al-d-stale-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    runtime::check_dir(&dir).unwrap();
    // A crashed daemon's leftovers: an unused port and the pid of a live process (this one).
    runtime::write_daemon_file(
        &dir,
        &runtime::DaemonFile {
            port: 1,
            pid: std::process::id(),
            token: "old".into(),
            version: "x".into(),
        },
    )
    .unwrap();
    let err = Client::connect_in(&dir).err().unwrap();
    assert_eq!(err.exit_code(), 3);
    assert!(
        !runtime::daemon_file_path(&dir).exists(),
        "an unreachable daemon's file is removed"
    );

    runtime::write_daemon_file(
        &dir,
        &runtime::DaemonFile {
            port: 1,
            pid: std::process::id(),
            token: "old".into(),
            version: "x".into(),
        },
    )
    .unwrap();
    let d = TestDaemon::start_in(dir, false, None);
    assert!(
        d.client.status().is_ok(),
        "a new daemon replaces the stale file"
    );
}

#[test]
fn a_second_daemon_in_the_same_runtime_dir_is_refused() {
    let d = TestDaemon::start(false);
    let err = autolab_daemon::run(ServeOptions {
        runtime_dir: d.dir.clone(),
        port: Some(0),
        exit_when_idle: false,
        idle_grace: None,
    })
    .unwrap_err();
    assert!(matches!(err, Error::SessionConflict(_)), "{err}");
    assert!(
        d.client.status().is_ok(),
        "the first daemon must be unharmed"
    );
}

// ---- idle timeout ---------------------------------------------------------------------------

#[test]
fn an_exiting_daemon_that_never_gets_a_session_exits_on_its_own() {
    // The CLI that spawned it may have died before sending `start`.
    let d = TestDaemon::start_with_grace(true, Some(Duration::from_millis(600)));
    assert!(
        wait_until(Duration::from_secs(8), || d.finished()),
        "an idle ephemeral daemon must not linger"
    );
    assert!(!runtime::daemon_file_path(&d.dir).exists());
}

#[test]
fn the_idle_timeout_never_ends_a_running_session_or_a_persistent_daemon() {
    let busy = TestDaemon::start_with_grace(true, Some(Duration::from_millis(600)));
    busy.client.start(&busy.dhu()).unwrap();
    thread::sleep(Duration::from_millis(1800)); // three times the grace
    assert!(
        !busy.finished(),
        "a daemon with a running session must stay up"
    );
    assert_eq!(
        busy.client.status().unwrap().session.unwrap().phase,
        Phase::Running
    );

    let persistent = TestDaemon::start_with_grace(false, Some(Duration::from_millis(600)));
    thread::sleep(Duration::from_millis(1800));
    assert!(
        !persistent.finished(),
        "a daemon started with `autolab daemon` stays up while idle"
    );
}

#[test]
fn the_typed_client_stream_delivers_the_same_events_in_order() {
    let d = TestDaemon::start(false);
    let mut stream = d.client.events().unwrap();
    let first = stream.next().unwrap().unwrap();
    assert_eq!(first.event, "stream.opened");
    assert_eq!(first.id, None);
    d.client.start(&d.dhu()).unwrap();
    d.client.focus_video(false).unwrap();
    d.client.stop().unwrap();
    let mut kinds = Vec::new();
    let mut ids = Vec::new();
    for m in stream.by_ref() {
        let m = m.unwrap();
        kinds.push(m.event.clone());
        ids.extend(m.id);
        if m.event == "session.stopped" {
            break;
        }
    }
    let pos = |k: &str| kinds.iter().position(|x| x == k).unwrap();
    assert!(pos("session.started") < pos("focus.video.accepted"));
    assert!(pos("focus.video.accepted") < pos("session.stopped"));
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "{ids:?}");
}

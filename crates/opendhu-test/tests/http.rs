//! Real loopback HTTP fixtures verify runner assertions and mutation request counts.
use opendhu_test::mock::{FakeClock, MockBackend};
use opendhu_test::runner::{Clock, RunConfig, run_test};
use opendhu_test::{Outcome, parse};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

struct Fixture {
    url: String,
    calls: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Fixture {
    fn new(responses: Vec<(u16, &'static str)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/fixture", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (recorded, stopping) = (Arc::clone(&calls), Arc::clone(&stop));
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(_) => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                };
                // Accepted sockets can inherit O_NONBLOCK on macOS. Reads below deliberately
                // use a bounded blocking deadline, independent of listener accept polling.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 2048];
                let complete = loop {
                    let n = match stream.read(&mut buffer) {
                        Ok(0) => break false,
                        Ok(n) => n,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        // A client may time out or close before sending its full request.
                        Err(_) => break false,
                    };
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..end]);
                        let length = header
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break true;
                        }
                    }
                };
                if !complete {
                    continue;
                }
                let mut calls = recorded.lock().unwrap();
                calls.push(String::from_utf8(bytes).unwrap());
                let index = (calls.len() - 1).min(responses.len() - 1);
                let (status, body) = responses[index];
                drop(calls);
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        Self {
            url,
            calls,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let joined = self.thread.take().unwrap().join();
        if !std::thread::panicking() {
            joined.unwrap();
        }
    }
}

#[test]
fn http_requests_poll_observations_but_never_replay_mutations() {
    let dir = std::env::temp_dir().join(format!("aa-http-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (method, responses, expected, wanted, count) in [
        (
            "GET",
            vec![(200, r#"{"ride":{"state":"ready","extra":3},"ids":[1,2]}"#)],
            "{ride: {state: ready}, ids: [1, 2]}",
            Outcome::Passed,
            1,
        ),
        (
            "GET",
            vec![
                (200, r#"{"ready":false}"#),
                (200, r#"{"ready":true,"extra":1}"#),
            ],
            "{ready: true}",
            Outcome::Passed,
            2,
        ),
        (
            "GET",
            vec![(200, r#"{"ids":[1,2,3]}"#)],
            "{ids: [1, 2]}",
            Outcome::Failed,
            4,
        ),
        (
            "GET",
            vec![(503, r#"{"ready":true}"#)],
            "{ready: true}",
            Outcome::Failed,
            4,
        ),
        (
            "POST",
            vec![(200, r#"{"ready":false}"#)],
            "{ready: true}",
            Outcome::Failed,
            1,
        ),
        (
            "POST",
            vec![(201, r#"{"ready":true}"#)],
            "{ready: true}",
            Outcome::Passed,
            1,
        ),
    ] {
        let server = Fixture::new(responses);
        let clock = FakeClock::default();
        let request = if method == "POST" {
            "\n    json: {ride: test-1}"
        } else {
            ""
        };
        let status = if method == "POST" && wanted == Outcome::Passed {
            201
        } else {
            200
        };
        let test = parse(&format!("name: HTTP fixture\nsteps:\n  - action: http.request\n    method: {method}\n    url: {}\n    timeout: 1s{request}\n    expect:\n      status: {status}\n      json: {expected}\n", server.url)).unwrap();
        let result = run_test(
            &test,
            "http.yaml",
            &mut MockBackend::new(),
            &RunConfig {
                application: None,
                app_label: None,
                artifacts_dir: &dir,
                skip_unsupported: false,
                clock: &clock,
            },
        );
        assert_eq!(result.outcome, wanted, "{:?}", result.message);
        let calls = server.calls.lock().unwrap();
        assert_eq!(calls.len(), count);
        if method == "POST" {
            assert!(calls[0].contains(r#"{"ride":"test-1"}"#));
        }
        if wanted == Outcome::Failed && method == "GET" {
            assert_eq!(clock.now(), Duration::from_secs(1));
            assert_eq!(result.exit_code, 6);
        }
    }
    assert!(std::fs::read_dir(dir.join("http")).unwrap().count() > 0);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn http_schema_rejects_unsafe_or_empty_expectations() {
    for tail in [
        "method: GET\n    url: http://127.0.0.1:99999/x\n    expect: {status: 200}",
        "method: GET\n    url: http://127.0.0.1:bad/x\n    expect: {status: 200}",
        "method: DELETE\n    url: http://127.0.0.1:8080/x\n    expect: {status: 200}",
        "method: GET\n    url: https://127.0.0.1/x\n    expect: {status: 200}",
        "method: GET\n    url: http://example.com/x\n    expect: {status: 200}",
        "method: GET\n    url: http://user@127.0.0.1/x\n    expect: {status: 200}",
        "method: GET\n    url: http://127.0.0.1:8080/x",
        "method: GET\n    url: http://127.0.0.1:8080/x\n    expect: {status: 200, json: {}}",
        "method: GET\n    url: http://127.0.0.1:8080/x\n    expect: {status: 200, typo: yes}",
    ] {
        assert!(
            parse(&format!(
                "name: HTTP\nsteps:\n  - action: http.request\n    timeout: 1s\n    {tail}\n"
            ))
            .is_err(),
            "{tail}"
        );
    }
}

#[test]
fn http_polling_obeys_cancellation() {
    let server = Fixture::new(vec![(200, r#"{"ready":false}"#)]);
    let test = parse(&format!("name: Cancel\nsteps:\n  - action: http.request\n    method: GET\n    url: {}\n    timeout: 10s\n    expect: {{status: 200, json: {{ready: true}}}}\n", server.url)).unwrap();
    let clock = FakeClock::default();
    clock.cancel_after_sleeps.set(Some(1));
    let dir = std::env::temp_dir().join(format!("aa-http-cancel-{}", std::process::id()));
    let result = run_test(
        &test,
        "http.yaml",
        &mut MockBackend::new(),
        &RunConfig {
            application: None,
            app_label: None,
            artifacts_dir: &dir,
            skip_unsupported: false,
            clock: &clock,
        },
    );
    assert_eq!((result.outcome, result.exit_code), (Outcome::Error, 130));
    assert_eq!(server.calls.lock().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_stalled_http_request_respects_its_wall_clock_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        std::thread::sleep(Duration::from_millis(300));
    });
    let test = parse(&format!("name: Slow fixture\nsteps:\n  - action: http.request\n    method: POST\n    url: http://{address}/fixture\n    timeout: 50ms\n    expect: {{status: 200}}\n")).unwrap();
    let dir = std::env::temp_dir().join(format!("aa-http-timeout-{}", std::process::id()));
    let clock = opendhu_test::runner::SystemClock::new();
    let started = std::time::Instant::now();
    let result = run_test(
        &test,
        "slow.yaml",
        &mut MockBackend::new(),
        &RunConfig {
            application: None,
            app_label: None,
            artifacts_dir: &dir,
            skip_unsupported: false,
            clock: &clock,
        },
    );
    assert_eq!(result.exit_code, 6, "{:?}", result.message);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(result.message.unwrap().contains("timed out"));
    server.join().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

//! Bounded HTTP assertions against local fixture APIs. Never follows redirects or uses proxies.
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use autolab_core::{Error, Result};
use serde_json::{Value, json};

use crate::Outcome;
use crate::runner::RunConfig;
use crate::schema::{HttpExpect, HttpMethod};

static REQUEST_ID: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn validate(
    method: HttpMethod,
    url: &str,
    body: Option<&Value>,
    expect: &HttpExpect,
) -> Result<()> {
    let invalid = || {
        Error::InvalidArgs("HTTP fixture URL must use http and an explicit loopback IP without credentials or fragments".into())
    };
    let uri: ureq::http::Uri = url.parse().map_err(|_| invalid())?;
    if uri.scheme_str() != Some("http") || url.contains('#') {
        return Err(invalid());
    }
    let authority = uri.authority().ok_or_else(invalid)?;
    if authority.as_str().contains('@') {
        return Err(invalid());
    }
    let host = authority
        .host()
        .trim_start_matches('[')
        .trim_end_matches(']');
    let ip: std::net::IpAddr = host.parse().map_err(|_| invalid())?;
    if !ip.is_loopback()
        || (authority.as_str() != authority.host()
            && authority.port_u16().is_none_or(|port| port == 0))
    {
        return Err(invalid());
    }
    if method == HttpMethod::Get && body.is_some() {
        return Err(Error::InvalidArgs(
            "GET fixture requests cannot have a JSON body".into(),
        ));
    }
    if !(100..=599).contains(&expect.status) {
        return Err(Error::InvalidArgs(
            "expected HTTP status must be between 100 and 599".into(),
        ));
    }
    if expect
        .json
        .as_ref()
        .is_some_and(|value| value.as_object().is_some_and(|object| object.is_empty()))
    {
        return Err(Error::InvalidArgs(
            "expected JSON object must not be empty".into(),
        ));
    }
    Ok(())
}

/// Object expectations are recursive subsets. Arrays and scalar values must match exactly.
fn contains(actual: &Value, expected: &Value) -> bool {
    match expected {
        Value::Object(expected) => actual.as_object().is_some_and(|actual| {
            expected.iter().all(|(key, value)| {
                actual
                    .get(key)
                    .is_some_and(|actual| contains(actual, value))
            })
        }),
        _ => actual == expected,
    }
}

fn save(path: &Path, data: &Value) -> Result<()> {
    std::fs::write(
        path,
        serde_json::to_vec_pretty(data).expect("JSON value serializes"),
    )
    .map_err(|error| {
        Error::Other(format!(
            "cannot write HTTP evidence {}: {error}",
            path.display()
        ))
    })
}

pub(crate) struct HttpResult {
    pub outcome: Outcome,
    pub detail: String,
    pub code: u8,
}

pub(crate) fn execute(
    method: HttpMethod,
    url: &str,
    body: Option<&Value>,
    expect: &HttpExpect,
    timeout: Duration,
    cfg: &RunConfig,
) -> Result<HttpResult> {
    validate(method, url, body, expect)?;
    let dir = cfg.artifacts_dir.join("http");
    std::fs::create_dir_all(&dir).map_err(|error| Error::Other(error.to_string()))?;
    let id = REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let request_path = dir.join(format!("request-{id:04}.json"));
    let response_path = dir.join(format!("response-{id:04}.json"));
    save(
        &request_path,
        &json!({"method":method,"url":url,"json":body,"expect":expect}),
    )?;
    let deadline = cfg.clock.now() + timeout;
    let mut attempts = 0;
    let mut last_mismatch = String::new();
    loop {
        if cfg.clock.cancelled() {
            return Ok(HttpResult {
                outcome: Outcome::Error,
                detail: "interrupted while observing HTTP fixture".into(),
                code: 130,
            });
        }
        let remaining = deadline.saturating_sub(cfg.clock.now());
        if remaining.is_zero() {
            return Ok(HttpResult {
                outcome: Outcome::Failed,
                detail: format!("HTTP fixture assertion timed out: {last_mismatch}"),
                code: 6,
            });
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(remaining.min(Duration::from_secs(2))))
            .build()
            .into();
        attempts += 1;
        let response = match method {
            HttpMethod::Get => agent.get(url).call(),
            HttpMethod::Post => match body {
                Some(body) => agent
                    .post(url)
                    .header("Content-Type", "application/json")
                    .send(serde_json::to_vec(body).expect("JSON serializes")),
                None => agent.post(url).send_empty(),
            },
        };
        let observed = response.and_then(|mut response| {
            let status = response.status().as_u16();
            response
                .body_mut()
                .read_to_string()
                .map(|text| (status, text))
        });
        let (status, text) = match observed {
            Ok(value) => value,
            Err(error) => {
                save(
                    &response_path,
                    &json!({"attempts":attempts,"error":error.to_string()}),
                )?;
                if cfg.clock.cancelled() {
                    return Ok(HttpResult {
                        outcome: Outcome::Error,
                        detail: "interrupted during HTTP fixture request".into(),
                        code: 130,
                    });
                }
                return Err(match error {
                    ureq::Error::Timeout(_) => {
                        Error::Timeout("HTTP fixture request timed out".into())
                    }
                    _ => Error::BackendUnavailable(format!("HTTP fixture request failed: {error}")),
                });
            }
        };
        let actual = serde_json::from_str::<Value>(&text).ok();
        save(
            &response_path,
            &json!({"attempts":attempts,"status":status,"json":actual,"body":text}),
        )?;
        if cfg.clock.cancelled() {
            return Ok(HttpResult {
                outcome: Outcome::Error,
                detail: "interrupted during HTTP fixture request".into(),
                code: 130,
            });
        }
        let matches = status == expect.status
            && expect.json.as_ref().is_none_or(|expected| {
                actual
                    .as_ref()
                    .is_some_and(|actual| contains(actual, expected))
            });
        if matches {
            return Ok(HttpResult {
                outcome: Outcome::Passed,
                detail: format!(
                    "HTTP {status} and fixture expectation matched after {attempts} request(s); {}",
                    response_path.display()
                ),
                code: 0,
            });
        }
        let mismatch = format!(
            "HTTP fixture mismatch: expected status {} and JSON {:?}; received status {status} and JSON {actual:?}; evidence {}",
            expect.status,
            expect.json,
            response_path.display()
        );
        if method == HttpMethod::Post {
            return Ok(HttpResult {
                outcome: Outcome::Failed,
                detail: mismatch,
                code: 7,
            });
        }
        let now = cfg.clock.now();
        if now >= deadline {
            return Ok(HttpResult {
                outcome: Outcome::Failed,
                detail: format!("HTTP fixture assertion timed out: {mismatch}"),
                code: 6,
            });
        }
        last_mismatch = mismatch;
        cfg.clock
            .sleep(Duration::from_millis(250).min(deadline - now));
    }
}

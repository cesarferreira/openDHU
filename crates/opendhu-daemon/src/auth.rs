//! Access control for the loopback API. Loopback TCP is reachable by every local user and by
//! browsers, so a bearer token alone is not the whole story:
//!
//! - `Authorization: Bearer <token>` is required, compared in constant time. The token lives only
//!   in `daemon.json`, inside the 0700 runtime directory.
//! - `Host` must be this daemon's own address, which blocks DNS rebinding.
//! - Any `Origin` header is refused: no browser client is supported, which blocks cross-site
//!   requests.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use serde_json::json;

use crate::error::plain;
use crate::log;

pub struct Guard {
    pub token: String,
    pub hosts: Vec<String>,
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub async fn guard(State(g): State<Arc<Guard>>, req: Request, next: Next) -> Response {
    let started = Instant::now();
    let (method, path) = (req.method().to_string(), req.uri().path().to_string());
    let headers = req.headers();

    let denied = if headers.contains_key(header::ORIGIN) {
        Some((
            StatusCode::FORBIDDEN,
            "forbidden",
            "cross-origin requests are not allowed",
        ))
    } else if !headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| g.hosts.iter().any(|ok| ok == h))
    {
        Some((StatusCode::FORBIDDEN, "forbidden", "unexpected Host header"))
    } else {
        let presented = headers
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "));
        match presented {
            Some(t) if ct_eq(t.as_bytes(), g.token.as_bytes()) => None,
            _ => Some((
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "missing or invalid bearer token",
            )),
        }
    };

    let response = match denied {
        Some((status, code, message)) => plain(status, code, message),
        None => next.run(req).await,
    };
    log::info(
        "http.request",
        json!({
            "method": method,
            "path": path,
            "status": response.status().as_u16(),
            "ms": started.elapsed().as_millis() as u64,
        }),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_compare_is_correct() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"abcd"));
        assert!(!ct_eq(b"", b"a"));
        assert!(ct_eq(b"", b""));
    }
}

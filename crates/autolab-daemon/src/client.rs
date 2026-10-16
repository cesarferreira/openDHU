//! Blocking client for the daemon API. This is how the CLI (and the test runner) reach the
//! session, so the CLI and any other client share exactly one interface.

use std::path::Path;
use std::time::Duration;

use autolab_core::{Capabilities, Error, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::api::{
    DevicesResponse, ErrorEnvelope, FocusRequest, OperationResponse, ScreenshotRequest,
    ScreenshotResponse, StartRequest, StatusResponse,
};
use crate::runtime;

const SHORT: Duration = Duration::from_secs(30);
/// Starting a session waits for the DHU handshake and the phone's return to adb.
const LONG: Duration = Duration::from_secs(150);

#[derive(Clone)]
pub struct Client {
    base: String,
    token: String,
    short: ureq::Agent,
    long: ureq::Agent,
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(timeout))
        .build()
        .into()
}

fn no_daemon() -> Error {
    Error::BackendUnavailable(
        "no AutoLab session is running; start one with `autolab start`".into(),
    )
}

impl Client {
    /// Connect to the daemon of the default runtime directory.
    pub fn connect() -> Result<Self> {
        Self::connect_in(&runtime::runtime_dir()?)
    }

    /// Connect using `daemon.json` in `dir`. A file whose daemon does not answer (a crash left it
    /// behind) is treated as absent and removed, never trusted on the pid alone.
    pub fn connect_in(dir: &Path) -> Result<Self> {
        let file = runtime::read_daemon_file(dir).ok_or_else(no_daemon)?;
        let client = Self {
            base: format!("http://127.0.0.1:{}", file.port),
            token: file.token.clone(),
            short: agent(SHORT),
            long: agent(LONG),
        };
        match client.status() {
            Ok(_) => Ok(client),
            Err(Error::BackendUnavailable(m)) if m.starts_with("cannot reach") => {
                runtime::remove_daemon_file_if_ours(dir, &file.token);
                Err(no_daemon())
            }
            Err(e) => Err(e),
        }
    }

    fn request<T: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<String>,
        long: bool,
    ) -> Result<T> {
        let agent = if long { &self.long } else { &self.short };
        let url = format!("{}{path}", self.base);
        let result = match (method, body) {
            ("GET", _) => agent
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.token))
                .call(),
            (_, Some(b)) => agent
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.token))
                .header("Content-Type", "application/json")
                .send(b),
            (_, None) => agent
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.token))
                .send_empty(),
        };
        let mut response = result.map_err(|e| match e {
            ureq::Error::Timeout(_) => {
                Error::Timeout("the AutoLab daemon did not answer in time".into())
            }
            ureq::Error::ConnectionFailed | ureq::Error::Io(_) => Error::BackendUnavailable(
                format!("cannot reach the AutoLab daemon at {}", self.base),
            ),
            other => Error::Other(format!("request to the AutoLab daemon failed: {other}")),
        })?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|e| Error::Other(format!("unreadable daemon response: {e}")))?;
        if (200..300).contains(&status) {
            return serde_json::from_str(&text)
                .map_err(|e| Error::Other(format!("unexpected daemon response: {e}")));
        }
        match serde_json::from_str::<ErrorEnvelope>(&text) {
            Ok(env) => Err(Error::from_code(env.error.exit_code, env.error.message)),
            Err(_) => Err(Error::Other(format!(
                "the AutoLab daemon answered HTTP {status}"
            ))),
        }
    }

    fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
        long: bool,
    ) -> Result<T> {
        let text = serde_json::to_string(body).expect("request serializes");
        self.request("POST", path, Some(text), long)
    }

    pub fn status(&self) -> Result<StatusResponse> {
        self.request("GET", "/v1/status", None, false)
    }

    pub fn capabilities(&self) -> Result<Capabilities> {
        self.request("GET", "/v1/capabilities", None, false)
    }

    pub fn devices(&self) -> Result<DevicesResponse> {
        self.request("GET", "/v1/devices", None, false)
    }

    pub fn start(&self, req: &StartRequest) -> Result<StatusResponse> {
        self.post_json("/v1/session/start", req, true)
    }

    pub fn stop(&self) -> Result<()> {
        let _: serde_json::Value = self.request("POST", "/v1/session/stop", None, true)?;
        Ok(())
    }

    pub fn focus_video(&self, enabled: bool) -> Result<OperationResponse> {
        self.post_json("/v1/focus/video", &FocusRequest { enabled }, false)
    }

    pub fn screenshot(&self, path: &Path) -> Result<ScreenshotResponse> {
        self.post_json(
            "/v1/screenshot",
            &ScreenshotRequest {
                path: path.to_path_buf(),
            },
            false,
        )
    }

    pub fn shutdown(&self) -> Result<()> {
        let _: serde_json::Value = self.request("POST", "/v1/daemon/shutdown", None, true)?;
        Ok(())
    }

    /// Base URL and token, for callers that stream `/v1/events` themselves.
    pub fn endpoint(&self) -> (&str, &str) {
        (&self.base, &self.token)
    }
}

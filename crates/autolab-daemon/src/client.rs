//! Blocking client for the daemon API. This is how the CLI (and the test runner) reach the
//! session, so the CLI and any other client share exactly one interface.

use std::io::{BufRead, Read};
use std::path::Path;
use std::time::Duration;

use autolab_core::{Capabilities, Error, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::api::{
    DevicesResponse, ErrorEnvelope, FocusRequest, InputResponse, KeyRequest, OperationResponse,
    ScreenshotRequest, ScreenshotResponse, StartRequest, StatusResponse, SwipeRequest, TapRequest,
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
    Error::BackendUnavailable("no AutoLab session is running; start one with `aa start`".into())
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

    pub fn tap(&self, x: u32, y: u32) -> Result<InputResponse> {
        self.post_json("/v1/input/tap", &TapRequest { x, y }, false)
    }

    pub fn key(&self, key: autolab_core::Key, long_press: bool) -> Result<InputResponse> {
        self.post_json("/v1/input/key", &KeyRequest { key, long_press }, false)
    }

    pub fn swipe(
        &self,
        from: (u32, u32),
        to: (u32, u32),
        duration: Duration,
    ) -> Result<InputResponse> {
        let body = SwipeRequest {
            x1: from.0,
            y1: from.1,
            x2: to.0,
            y2: to.1,
            duration_ms: u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
        };
        self.post_json("/v1/input/swipe", &body, false)
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

/// One message from `GET /v1/events`. `stream.opened` has no `id` and an empty object as data.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamMessage {
    pub id: Option<u64>,
    pub event: String,
    pub data: serde_json::Value,
}

/// Iterator over the server-sent events of an open stream. It ends when the daemon closes the
/// stream, and yields an error if the connection breaks.
pub struct EventStream {
    reader: Box<dyn BufRead + Send>,
}

impl EventStream {
    pub fn from_reader(reader: impl BufRead + Send + 'static) -> Self {
        Self {
            reader: Box::new(reader),
        }
    }
}

impl Iterator for EventStream {
    type Item = Result<StreamMessage>;

    fn next(&mut self) -> Option<Self::Item> {
        let (mut id, mut event, mut data) = (None, None::<String>, Vec::<String>::new());
        let mut line = String::new();
        loop {
            line.clear();
            match self.reader.read_line(&mut line) {
                Ok(0) => return None,
                Ok(_) => {}
                Err(e) => {
                    return Some(Err(Error::BackendUnavailable(format!(
                        "the event stream broke: {e}"
                    ))));
                }
            }
            let l = line.trim_end_matches(['\r', '\n']);
            if l.is_empty() {
                // A blank line ends a message. Comments (keepalives) leave nothing to deliver.
                if event.is_none() && data.is_empty() {
                    continue;
                }
                let text = data.join("\n");
                let value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
                return Some(Ok(StreamMessage {
                    id,
                    event: event.unwrap_or_else(|| "message".into()),
                    data: value,
                }));
            }
            if l.starts_with(':') {
                continue;
            }
            let (field, value) = l.split_once(':').unwrap_or((l, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "id" => id = value.parse().ok(),
                "event" => event = Some(value.to_string()),
                "data" => data.push(value.to_string()),
                _ => {}
            }
        }
    }
}

impl Client {
    /// Open `GET /v1/video`: the phone's raw H.264 (Annex-B), starting at a keyframe. Native backend
    /// only. Reads block until the next chunk, and end when the session ends or the daemon drops the
    /// viewer for lagging.
    pub fn video(&self) -> Result<Box<dyn Read + Send>> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(5)))
            .build()
            .into();
        let mut response = agent
            .get(format!("{}/v1/video", self.base))
            .header("Authorization", format!("Bearer {}", self.token))
            .call()
            .map_err(|_| {
                Error::BackendUnavailable(format!(
                    "cannot reach the AutoLab daemon at {}",
                    self.base
                ))
            })?;
        let status = response.status().as_u16();
        if status != 200 {
            let text = response.body_mut().read_to_string().unwrap_or_default();
            return Err(match serde_json::from_str::<ErrorEnvelope>(&text) {
                Ok(env) => Error::from_code(env.error.exit_code, env.error.message),
                Err(_) => Error::Other(format!("the video stream answered HTTP {status}")),
            });
        }
        Ok(Box::new(response.into_body().into_reader()))
    }

    /// Open `GET /v1/audio/{channel}`: raw 16-bit PCM from the phone (`AudioChannel::format`). Native
    /// backend only. Reads block until the phone plays something, and end when the session ends.
    pub fn audio(&self, channel: autolab_core::AudioChannel) -> Result<Box<dyn Read + Send>> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(5)))
            .build()
            .into();
        let mut response = agent
            .get(format!("{}/v1/audio/{}", self.base, channel.name()))
            .header("Authorization", format!("Bearer {}", self.token))
            .call()
            .map_err(|_| {
                Error::BackendUnavailable(format!(
                    "cannot reach the AutoLab daemon at {}",
                    self.base
                ))
            })?;
        let status = response.status().as_u16();
        if status != 200 {
            let text = response.body_mut().read_to_string().unwrap_or_default();
            return Err(match serde_json::from_str::<ErrorEnvelope>(&text) {
                Ok(env) => Error::from_code(env.error.exit_code, env.error.message),
                Err(_) => Error::Other(format!("the audio stream answered HTTP {status}")),
            });
        }
        Ok(Box::new(response.into_body().into_reader()))
    }

    /// Open `GET /v1/events`. The first message is `stream.opened`; from then on the subscription
    /// is live. The call returns once the daemon has answered, and reads then block until an event
    /// or a keepalive (every 15 s) arrives.
    pub fn events(&self) -> Result<EventStream> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(5)))
            .build()
            .into();
        let response = agent
            .get(format!("{}/v1/events", self.base))
            .header("Authorization", format!("Bearer {}", self.token))
            .call()
            .map_err(|_| {
                Error::BackendUnavailable(format!(
                    "cannot reach the AutoLab daemon at {}",
                    self.base
                ))
            })?;
        let status = response.status().as_u16();
        if status != 200 {
            return Err(Error::Other(format!(
                "the event stream answered HTTP {status}"
            )));
        }
        Ok(EventStream::from_reader(std::io::BufReader::new(
            response.into_body().into_reader(),
        )))
    }
}

#[cfg(test)]
mod stream_tests {
    use super::*;

    fn parse(text: &str) -> Vec<StreamMessage> {
        EventStream::from_reader(std::io::Cursor::new(text.to_string()))
            .map(|m| m.unwrap())
            .collect()
    }

    #[test]
    fn parses_messages_skips_keepalives_and_tolerates_missing_id() {
        let got = parse(
            "event: stream.opened\ndata: {}\n\n: keepalive\n\nid: 7\nevent: focus.video.accepted\ndata: {\"a\":1}\n\n",
        );
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].id, got[0].event.as_str()), (None, "stream.opened"));
        assert_eq!(
            (got[1].id, got[1].event.as_str()),
            (Some(7), "focus.video.accepted")
        );
        assert_eq!(got[1].data["a"], 1);
    }

    #[test]
    fn joins_multiline_data_and_ignores_unfinished_trailing_message() {
        let got = parse("event: x\ndata: [1,\ndata: 2]\n\nevent: cut\ndata: {");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].data, serde_json::json!([1, 2]));
    }
}

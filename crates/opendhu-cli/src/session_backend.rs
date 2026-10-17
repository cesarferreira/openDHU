//! `HeadUnitBackend` over the daemon's HTTP API. This is how the test runner (which knows only the
//! trait) reaches the session, so it never talks to the DHU directly.

use std::path::Path;
use std::time::Duration;

use opendhu_core::{
    Capabilities, Error, HeadUnitBackend, HeadUnitStatus, InputResult, Key, OperationResult,
    Result, ScreenshotResult,
};
use opendhu_daemon::Client;
use opendhu_daemon::api::Phase;

pub struct DaemonBackend {
    client: Client,
}

impl DaemonBackend {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

impl HeadUnitBackend for DaemonBackend {
    fn capabilities(&mut self) -> Result<Capabilities> {
        self.client.capabilities()
    }

    fn status(&mut self) -> Result<HeadUnitStatus> {
        let session = self.client.status()?.session.ok_or_else(|| {
            Error::BackendUnavailable(
                "no openDHU session is running; start one with `aa start`".into(),
            )
        })?;
        match session.details {
            Some(d) if session.phase == Phase::Running => Ok(d.head_unit),
            _ => Err(Error::BackendUnavailable(
                format!("the session is {:?}", session.phase).to_lowercase(),
            )),
        }
    }

    fn set_video_focus(&mut self, enabled: bool) -> Result<OperationResult> {
        Ok(self.client.focus_video(enabled)?.result)
    }

    fn screenshot(&mut self, path: &Path) -> Result<ScreenshotResult> {
        Ok(self.client.screenshot(path)?.result)
    }

    fn tap(&mut self, x: u32, y: u32) -> Result<InputResult> {
        Ok(self.client.tap(x, y)?.result)
    }

    fn swipe(
        &mut self,
        from: (u32, u32),
        to: (u32, u32),
        duration: Duration,
    ) -> Result<InputResult> {
        Ok(self.client.swipe(from, to, duration)?.result)
    }

    fn press_key(&mut self, key: Key, long_press: bool) -> Result<InputResult> {
        Ok(self.client.key(key, long_press)?.result)
    }

    fn stop(&mut self) -> Result<()> {
        self.client.stop()
    }
}

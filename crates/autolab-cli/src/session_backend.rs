//! `HeadUnitBackend` over the daemon's HTTP API. This is how the test runner (which knows only the
//! trait) reaches the session, so it never talks to the DHU directly.

use std::path::Path;

use autolab_core::{
    Capabilities, Error, HeadUnitBackend, HeadUnitStatus, OperationResult, Result, ScreenshotResult,
};
use autolab_daemon::Client;
use autolab_daemon::api::Phase;

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
                "no AutoLab session is running; start one with `aa start`".into(),
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

    fn stop(&mut self) -> Result<()> {
        self.client.stop()
    }
}

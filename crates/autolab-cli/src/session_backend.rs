//! `HeadUnitBackend` over the session socket. This is how the test runner (which knows only the
//! trait) reaches the DHU supervisor, so it never talks to the DHU directly.

use std::path::Path;

use autolab_core::{
    Capabilities, Error, HeadUnitBackend, HeadUnitStatus, OperationResult, Result, ScreenshotResult,
};

use crate::session::{self, Request, Response};

pub struct SessionBackend;

impl HeadUnitBackend for SessionBackend {
    fn capabilities(&mut self) -> Result<Capabilities> {
        match session::request(&Request::Capabilities)? {
            Response::Capabilities(c) => Ok(c),
            other => Err(Error::Other(format!(
                "unexpected reply to capabilities: {other:?}"
            ))),
        }
    }

    fn status(&mut self) -> Result<HeadUnitStatus> {
        Ok(session::status()?.head_unit)
    }

    fn set_video_focus(&mut self, enabled: bool) -> Result<OperationResult> {
        session::set_video_focus(enabled)
    }

    fn screenshot(&mut self, path: &Path) -> Result<ScreenshotResult> {
        session::screenshot(path)
    }

    fn stop(&mut self) -> Result<()> {
        session::stop()
    }
}

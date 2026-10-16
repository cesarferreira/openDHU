//! The test run owns only its viewer process group, using the same GUI as `aa gui`.
use autolab_core::{Error, Result};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

pub(crate) fn program() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("autolab-gui")))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("autolab-gui"))
}

pub(crate) struct Viewer(Child);
impl Viewer {
    pub(crate) fn start(runtime: Option<&Path>, artifacts: &Path) -> Result<Self> {
        std::fs::create_dir_all(artifacts).map_err(|e| Error::Other(e.to_string()))?;
        let log = std::fs::File::create(artifacts.join("head-unit-viewer.log"))
            .map_err(|e| Error::Other(e.to_string()))?;
        let mut command = Command::new(program());
        if let Some(runtime) = runtime {
            command.env("AUTOLAB_RUNTIME_DIR", runtime);
        }
        command
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| Error::Other(e.to_string()))?)
            .stderr(log)
            .spawn()
            .map(Self)
            .map_err(|e| {
                Error::BackendUnavailable(format!(
                    "cannot show the head unit: {e}; install the GUI with `make install-gui`"
                ))
            })
    }
}
impl Drop for Viewer {
    fn drop(&mut self) {
        // Stop the window and its decoder together; never target another GUI or session.
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.wait();
    }
}

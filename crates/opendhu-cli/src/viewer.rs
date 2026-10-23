//! The test run owns only its viewer process group, using the same GUI as `aa gui`.
use opendhu_core::{Error, Result};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

pub(crate) fn program() -> PathBuf {
    sibling("opendhu-gui")
}

/// `name` installed next to `aa`, or else whatever `PATH` finds.
pub(crate) fn sibling(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

pub(crate) struct Viewer(Child);
impl Viewer {
    pub(crate) fn start(runtime: Option<&Path>, artifacts: &Path) -> Result<Self> {
        std::fs::create_dir_all(artifacts).map_err(|e| Error::Other(e.to_string()))?;
        let log = std::fs::File::create(artifacts.join("head-unit-viewer.log"))
            .map_err(|e| Error::Other(e.to_string()))?;
        let mut command = Command::new(program());
        if let Some(runtime) = runtime {
            command.env("OPENDHU_RUNTIME_DIR", runtime);
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

/// The terminal viewer for `aa test --tui`: `opendhu-tui` on this terminal, fed the run's progress
/// through a pipe. Dropping it ends the feed, waits for the user to close the viewer, then prints
/// the run into the scrollback.
pub(crate) struct TuiViewer {
    child: Child,
    output: std::sync::Arc<crate::test_output::Output>,
}

impl TuiViewer {
    pub(crate) fn start(
        runtime: Option<&Path>,
        output: std::sync::Arc<crate::test_output::Output>,
    ) -> Result<Self> {
        use std::os::fd::FromRawFd;

        let mut fds = [0i32; 2];
        // `pipe2` does not exist on macOS, so create the pipe and set close-on-exec separately.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return Err(Error::Other(std::io::Error::last_os_error().to_string()));
        }
        for fd in fds {
            if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
                let err = std::io::Error::last_os_error().to_string();
                unsafe {
                    libc::close(fds[0]);
                    libc::close(fds[1]);
                }
                return Err(Error::Other(err));
            }
        }
        let (read, write) = (fds[0], fds[1]);
        // SAFETY: `write` is a fresh descriptor nothing else owns.
        let pipe = unsafe { std::fs::File::from_raw_fd(write) };
        let mut command = Command::new(sibling("opendhu-tui"));
        command.env("OPENDHU_TUI_LOG_FD", read.to_string());
        if let Some(runtime) = runtime {
            command.env("OPENDHU_RUNTIME_DIR", runtime);
        }
        // Only the viewer inherits the read end; the write end stays here so closing it is the
        // viewer's end-of-run signal.
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(read, libc::F_SETFD, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let spawned = command.spawn();
        unsafe { libc::close(read) };
        let child = spawned.map_err(|e| {
            Error::BackendUnavailable(format!(
                "cannot show the head unit in the terminal: {e}; install it with `make install-tui`"
            ))
        })?;
        output.attach_viewer(pipe);
        Ok(Self { child, output })
    }
}

impl Drop for TuiViewer {
    fn drop(&mut self) {
        // Closing the pipe tells the viewer the run is over; it stays until the user closes it.
        let transcript = self.output.detach_viewer();
        let _ = self.child.wait();
        self.output.print(&transcript);
    }
}

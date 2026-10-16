//! The runtime directory and the `daemon.json` file clients use to find and authenticate to the
//! daemon. The directory is the access boundary: it must be a real directory, mode 0700, owned
//! by the current user, so only that user can read the token.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use autolab_core::{Error, Result};
use serde::{Deserialize, Serialize};

pub const DAEMON_FILE: &str = "daemon.json";
pub const LOG_FILE: &str = "daemon.log";

/// Where a running daemon says it can be reached. `token` is a secret: never log it.
#[derive(Clone, Serialize, Deserialize)]
pub struct DaemonFile {
    pub port: u16,
    pub pid: u32,
    pub token: String,
    pub version: String,
}

impl std::fmt::Debug for DaemonFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaemonFile")
            .field("port", &self.port)
            .field("pid", &self.pid)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// The runtime directory, created with mode 0700 if missing. `AUTOLAB_RUNTIME_DIR` overrides it.
pub fn runtime_dir() -> Result<PathBuf> {
    let dir = match std::env::var_os("AUTOLAB_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => {
            let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
            std::env::temp_dir().join(format!("autolab-{user}"))
        }
    };
    check_dir(&dir)?;
    Ok(dir)
}

/// Create (if needed) and verify a runtime directory.
pub fn check_dir(dir: &Path) -> Result<()> {
    let io = |e: std::io::Error| Error::Other(format!("runtime dir {}: {e}", dir.display()));
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(io)?;
    let meta = fs::symlink_metadata(dir).map_err(io)?;
    if !meta.is_dir() || meta.mode() & 0o077 != 0 {
        return Err(Error::Other(format!(
            "runtime dir {} must be a real directory with mode 0700",
            dir.display()
        )));
    }
    if let Some(home) = std::env::var_os("HOME")
        && let Ok(h) = fs::metadata(home)
        && h.uid() != meta.uid()
    {
        return Err(Error::Other(format!(
            "runtime dir {} is not owned by the current user",
            dir.display()
        )));
    }
    Ok(())
}

pub fn daemon_file_path(dir: &Path) -> PathBuf {
    dir.join(DAEMON_FILE)
}

pub fn read_daemon_file(dir: &Path) -> Option<DaemonFile> {
    serde_json::from_slice(&fs::read(daemon_file_path(dir)).ok()?).ok()
}

/// Write `daemon.json` atomically with mode 0600, so a reader never sees a partial file.
pub fn write_daemon_file(dir: &Path, file: &DaemonFile) -> Result<()> {
    let io = |e: std::io::Error| Error::Other(format!("cannot write {DAEMON_FILE}: {e}"));
    let tmp = dir.join(format!("{DAEMON_FILE}.{}.tmp", std::process::id()));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(io)?;
    f.write_all(&serde_json::to_vec_pretty(file).expect("serializes"))
        .map_err(io)?;
    f.sync_all().map_err(io)?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600)).map_err(io)?;
    fs::rename(&tmp, daemon_file_path(dir)).map_err(io)
}

/// Remove `daemon.json` only if it still describes this process, so a replacement daemon's file
/// is never deleted by its predecessor.
pub fn remove_daemon_file_if_ours(dir: &Path, token: &str) {
    if read_daemon_file(dir).is_some_and(|f| f.token == token) {
        let _ = fs::remove_file(daemon_file_path(dir));
    }
}

fn random_hex(bytes: usize) -> Result<String> {
    let mut buf = vec![0u8; bytes];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .map_err(|e| Error::Other(format!("cannot read /dev/urandom: {e}")))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// 256 bits from the OS, hex encoded.
pub fn new_token() -> Result<String> {
    random_hex(32)
}

/// Short id for a session, e.g. `8f31c2`.
pub fn new_session_id() -> Result<String> {
    random_hex(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = PathBuf::from(format!("/tmp/al-rt-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn tokens_are_long_hex_and_unique() {
        let (a, b) = (new_token().unwrap(), new_token().unwrap());
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
        assert_eq!(new_session_id().unwrap().len(), 6);
    }

    #[test]
    fn daemon_file_is_private_atomic_and_round_trips() {
        let d = tmp("file");
        check_dir(&d).unwrap();
        let f = DaemonFile {
            port: 8765,
            pid: 7,
            token: "tok".into(),
            version: "0.1.0".into(),
        };
        write_daemon_file(&d, &f).unwrap();
        let mode = fs::metadata(daemon_file_path(&d)).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(read_daemon_file(&d).unwrap().port, 8765);
        assert!(
            fs::read_dir(&d).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp"))
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_token_never_appears_in_debug_output() {
        let f = DaemonFile {
            port: 1,
            pid: 1,
            token: "SECRET".into(),
            version: "x".into(),
        };
        assert!(!format!("{f:?}").contains("SECRET"));
    }

    #[test]
    fn a_predecessor_does_not_delete_its_successors_file() {
        let d = tmp("succ");
        check_dir(&d).unwrap();
        write_daemon_file(
            &d,
            &DaemonFile {
                port: 1,
                pid: 1,
                token: "new".into(),
                version: "x".into(),
            },
        )
        .unwrap();
        remove_daemon_file_if_ours(&d, "old");
        assert!(daemon_file_path(&d).exists());
        remove_daemon_file_if_ours(&d, "new");
        assert!(!daemon_file_path(&d).exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn loose_permissions_are_refused() {
        let d = tmp("loose");
        fs::create_dir_all(&d).unwrap();
        fs::set_permissions(&d, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(check_dir(&d).unwrap_err().to_string().contains("0700"));
        let _ = fs::remove_dir_all(&d);
    }
}

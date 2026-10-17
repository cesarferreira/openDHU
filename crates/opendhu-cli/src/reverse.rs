//! An ADB reverse mapping acquired without replacing another owner's mapping.
use opendhu_core::{Error, Result};
use std::path::{Path, PathBuf};

/// A mapping is owned only after the atomic `--no-rebind` acquisition succeeds.
pub struct ReverseLease {
    adb: PathBuf,
    serial: String,
    device: String,
    host: String,
    owned: bool,
}

impl ReverseLease {
    pub fn acquire(adb: &Path, serial: &str, device: u16, host: u16) -> Result<Self> {
        if device == 0 || host == 0 {
            return Err(Error::InvalidArgs("reverse ports must be nonzero".into()));
        }
        let mut lease = Self {
            adb: adb.into(),
            serial: serial.into(),
            device: format!("tcp:{device}"),
            host: format!("tcp:{host}"),
            owned: false,
        };
        if lease.current_host()?.is_some() {
            return Err(Error::SessionConflict(format!(
                "device reverse {} already exists; refusing to replace it",
                lease.device
            )));
        }
        crate::device::adb(
            &lease.adb,
            &lease.serial,
            &["reverse", "--no-rebind", &lease.device, &lease.host],
        )?;
        lease.owned = true;
        Ok(lease)
    }

    fn current_host(&self) -> Result<Option<String>> {
        let mappings = crate::device::adb(&self.adb, &self.serial, &["reverse", "--list"])?;
        Ok(mappings.lines().find_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            (fields.get(1) == Some(&self.device.as_str()))
                .then(|| fields.get(2).map(|host| host.to_string()))
                .flatten()
        }))
    }

    /// Idempotent and explicit so teardown failures are visible to the runner.
    pub fn cleanup(&mut self) -> Result<()> {
        if !self.owned {
            return Ok(());
        }
        if self.current_host()?.as_deref() == Some(self.host.as_str()) {
            crate::device::adb(
                &self.adb,
                &self.serial,
                &["reverse", "--remove", &self.device],
            )?;
        }
        // An absent or changed mapping no longer belongs to this lease.
        self.owned = false;
        Ok(())
    }
}
impl Drop for ReverseLease {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static ID: AtomicUsize = AtomicUsize::new(0);
    struct Adb {
        dir: PathBuf,
        program: PathBuf,
    }
    impl Adb {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "aa-reverse-{}-{}",
                std::process::id(),
                ID.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let program = dir.join("adb");
            let script = r#"#!/bin/sh
state='STATE'
printf '%s\n' "$*" >> "$state/calls"
[ "$1" = -s ] || exit 8
[ "$2" = emulator-5554 ] || exit 8
shift 2
case "$1 $2" in
  'reverse --list') [ ! -f "$state/mappings" ] || cat "$state/mappings";;
  'reverse --no-rebind')
    [ ! -f "$state/fail" ] || exit 9
    if [ -f "$state/mappings" ] && awk -v port="$3" '$2 == port {found=1} END {exit !found}' "$state/mappings"; then exit 9; fi
    printf 'emulator-5554 %s %s\n' "$3" "$4" >> "$state/mappings";;
  'reverse --remove')
    awk -v port="$3" '$2 != port' "$state/mappings" > "$state/next"
    mv "$state/next" "$state/mappings";;
  *) exit 8;;
esac
"#.replace("STATE", &dir.to_string_lossy());
            std::fs::write(&program, script).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self { dir, program }
        }
        fn mappings(&self) -> String {
            std::fs::read_to_string(self.dir.join("mappings")).unwrap_or_default()
        }
        fn calls(&self) -> String {
            std::fs::read_to_string(self.dir.join("calls")).unwrap_or_default()
        }
        fn seed(&self, content: &str) {
            std::fs::write(self.dir.join("mappings"), content).unwrap();
        }
    }
    impl Drop for Adb {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn preexisting_mapping_is_not_rebound_or_removed() {
        let adb = Adb::new();
        let before = "emulator-5554 tcp:8080 tcp:9999\n";
        adb.seed(before);
        assert!(ReverseLease::acquire(&adb.program, "emulator-5554", 8080, 8080).is_err());
        assert_eq!(adb.mappings(), before);
        assert!(!adb.calls().contains("--no-rebind"));
        assert!(!adb.calls().contains("--remove"));
    }

    #[test]
    fn failed_atomic_acquisition_never_owns_or_removes_a_mapping() {
        let adb = Adb::new();
        std::fs::write(adb.dir.join("fail"), "").unwrap();
        assert!(ReverseLease::acquire(&adb.program, "emulator-5554", 8080, 8080).is_err());
        assert!(adb.mappings().is_empty());
        assert!(adb.calls().contains("--no-rebind"));
        assert!(!adb.calls().contains("--remove"));
    }

    #[test]
    fn partial_setup_cleanup_removes_only_its_successfully_acquired_mapping() {
        let adb = Adb::new();
        adb.seed("emulator-5554 tcp:8081 tcp:9999\n");
        let first = ReverseLease::acquire(&adb.program, "emulator-5554", 8080, 8080).unwrap();
        assert!(ReverseLease::acquire(&adb.program, "emulator-5554", 8081, 8081).is_err());
        assert!(adb.mappings().contains("tcp:8080 tcp:8080"));
        drop(first);
        assert_eq!(adb.mappings(), "emulator-5554 tcp:8081 tcp:9999\n");
    }

    #[test]
    fn cleanup_preserves_a_mapping_replaced_by_another_owner() {
        let adb = Adb::new();
        let mut lease = ReverseLease::acquire(&adb.program, "emulator-5554", 8080, 8080).unwrap();
        let replacement = "emulator-5554 tcp:8080 tcp:9999\n";
        adb.seed(replacement);
        lease.cleanup().unwrap();
        drop(lease);
        assert_eq!(adb.mappings(), replacement);
        assert!(!adb.calls().contains("--remove"));
    }
}

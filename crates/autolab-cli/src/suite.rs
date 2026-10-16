//! Own one isolated emulator projection session while executing ordinary YAML tests.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::{device, test_cmd};
use autolab_core::{Error, Result};
use fs2::FileExt;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Suite {
    version: u32,
    environment: Environment,
    tests: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Environment {
    emulator: String,
    #[serde(default = "yes")]
    headless: bool,
    apk: PathBuf,
    package: String,
    app_label: String,
}
fn yes() -> bool {
    true
}

pub struct Application {
    pub label: String,
    adb: PathBuf,
    serial: String,
    package: String,
}
impl Application {
    pub fn reset(&self) -> Result<()> {
        device::adb(
            &self.adb,
            &self.serial,
            &["shell", "am", "force-stop", &self.package],
        )?;
        Ok(())
    }
}

pub fn is_suite(path: &Path) -> Result<bool> {
    if !path.is_file() {
        return Ok(false);
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| Error::InvalidArgs(format!("cannot read {}: {e}", path.display())))?;
    let value: serde_norway::Value = match serde_norway::from_str(&text) {
        Ok(value) => value,
        Err(_) => return Ok(false), // The ordinary parser supplies the syntax diagnostic.
    };
    Ok(value.get("environment").is_some() || value.get("version").is_some())
}

fn load(path: &Path) -> Result<(Suite, Vec<PathBuf>)> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::InvalidArgs(e.to_string()))?;
    let mut suite: Suite = serde_norway::from_str(&text)
        .map_err(|e| Error::InvalidArgs(format!("invalid suite {}: {e}", path.display())))?;
    let env = &suite.environment;
    if suite.version != 1 {
        return Err(Error::InvalidArgs("suite version must be 1".into()));
    }
    if env.emulator.trim().is_empty()
        || env.emulator.starts_with('-')
        || !env
            .emulator
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        || env.app_label.trim().is_empty()
        || env.apk.as_os_str().is_empty()
    {
        return Err(Error::InvalidArgs(
            "suite emulator, apk and appLabel must be nonempty".into(),
        ));
    }
    if !env.package.contains('.')
        || !env
            .package
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
    {
        return Err(Error::InvalidArgs(
            "suite package must be an Android package identifier".into(),
        ));
    }
    let base = path.parent().unwrap_or(Path::new("."));
    if !suite.environment.apk.is_absolute() {
        suite.environment.apk = base.join(&suite.environment.apk);
    }
    let mut tests = Vec::new();
    for pattern in &suite.tests {
        let pattern = base.join(pattern);
        let matches = glob::glob(&pattern.to_string_lossy())
            .map_err(|e| Error::InvalidArgs(e.to_string()))?;
        let mut found = false;
        for file in matches {
            let file = file.map_err(|e| Error::InvalidArgs(e.to_string()))?;
            if file.is_file() {
                found = true;
                tests.push(file);
            }
        }
        if !found {
            return Err(Error::InvalidArgs(format!(
                "suite test pattern matches no files: {}",
                pattern.display()
            )));
        }
    }
    tests.sort();
    tests.dedup();
    if tests.is_empty() {
        return Err(Error::InvalidArgs(
            "suite must include at least one test file".into(),
        ));
    }
    test_cmd::parse_all(&tests)?; // Validate every test before touching any device.
    Ok((suite, tests))
}

fn find_emulator(adb: &Path, name: &str) -> Result<Option<String>> {
    let devices = device::command(adb, &["devices"], Duration::from_secs(20))?;
    for line in devices.lines().skip(1) {
        let mut fields = line.split_whitespace();
        let Some(serial) = fields.next() else {
            continue;
        };
        if !serial.starts_with("emulator-") || fields.next() != Some("device") {
            continue;
        }
        let avd = device::adb(adb, serial, &["emu", "avd", "name"])?;
        if avd.lines().next() == Some(name) {
            return Ok(Some(serial.into()));
        }
    }
    Ok(None)
}

fn suite_cli(exe: &Path, runtime: &Path, args: &[&str], cancel: Option<&AtomicBool>) -> Result<()> {
    let mut child = Command::new(exe)
        .args(args)
        .env("AUTOLAB_RUNTIME_DIR", runtime)
        .env("AUTOLAB_NATIVE_RESOLUTION", "480p")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| Error::BackendUnavailable(format!("cannot start aa: {e}")))?;
    let deadline = Instant::now() + Duration::from_secs(100);
    loop {
        if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Other("suite setup interrupted".into()));
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(Error::BackendUnavailable(format!(
                    "suite head-unit command failed: {status}"
                )));
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            other => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Timeout(format!(
                    "suite head-unit command did not finish: {other:?}"
                )));
            }
        }
    }
}

/// Ownership is recorded as soon as acquired. A failed startup still tears down its forwarding and
/// private daemon, and never stops a phone session or an emulator it did not start.
struct Lease {
    exe: PathBuf,
    runtime: PathBuf,
    adb: PathBuf,
    serial: Option<String>,
    forward: Option<String>,
    emulator_child: Option<std::process::Child>,
    _lock: std::fs::File,
    logcat: Option<std::process::Child>,
    artifacts: PathBuf,
}
impl Lease {
    fn cleanup(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        if self.runtime.exists() {
            // A failed start can leave a daemon without a session; terminate only this private PID.
            let _ = suite_cli(&self.exe, &self.runtime, &["stop"], None);
            if let Ok(text) = std::fs::read_to_string(self.runtime.join("daemon.json"))
                && let Ok(info) = serde_json::from_str::<serde_json::Value>(&text)
                && let Some(pid) = info["pid"].as_u64()
            {
                let _ = device::command(
                    Path::new("kill"),
                    &["-TERM", &pid.to_string()],
                    Duration::from_secs(5),
                );
            }
            let _ = std::fs::copy(
                self.runtime.join("daemon.log"),
                self.artifacts.join("head-unit.log"),
            );
            let _ = std::fs::remove_dir_all(&self.runtime);
        }
        if let Some(mut child) = self.logcat.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(serial) = &self.serial {
            if let Some(port) = self.forward.take()
                && let Err(e) = device::adb(&self.adb, serial, &["forward", "--remove", &port])
            {
                errors.push(e.message().to_string());
            }
        }
        if let Some(mut child) = self.emulator_child.take() {
            if let Some(serial) = &self.serial {
                let _ = device::adb(&self.adb, serial, &["emu", "kill"]);
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(Error::Other(format!(
                "suite cleanup failed: {}",
                errors.join("; ")
            )))
        }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

pub fn run(opts: test_cmd::Options) -> Result<u8> {
    let (suite, tests) = load(&opts.paths[0])?;
    if opts.validate {
        if opts.json {
            println!(
                "{}",
                serde_json::json!({"valid":true,"tests":tests.len(),"suiteVersion":1})
            );
        } else {
            println!("Validated suite and {} test files", tests.len());
        }
        return Ok(0);
    }
    let cancel = Arc::new(AtomicBool::new(false));
    test_cmd::install_interrupt_handler(&cancel)?;
    let result = run_prepared(opts, suite, tests, Arc::clone(&cancel));
    if cancel.load(Ordering::SeqCst) {
        Ok(130)
    } else {
        result
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::SeqCst) {
        Err(Error::Other("suite setup interrupted".into()))
    } else {
        Ok(())
    }
}

fn run_prepared(
    mut opts: test_cmd::Options,
    suite: Suite,
    tests: Vec<PathBuf>,
    cancel: Arc<AtomicBool>,
) -> Result<u8> {
    check_cancel(&cancel)?;
    if !suite.environment.apk.is_file() {
        return Err(Error::InvalidArgs(format!(
            "suite APK not found: {}; build it before running the suite",
            suite.environment.apk.display()
        )));
    }
    // Fail before starting hardware when OCR isn't installed.
    device::command(
        Path::new("tesseract"),
        &["--version"],
        Duration::from_secs(10),
    )?;
    let cwd = std::env::current_dir().map_err(|e| Error::Other(e.to_string()))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let artifacts = opts
        .artifacts_dir
        .take()
        .unwrap_or_else(|| PathBuf::from(format!("test-results/run-{nonce}")));
    let artifacts = if artifacts.is_absolute() {
        artifacts
    } else {
        cwd.join(artifacts)
    };
    std::fs::create_dir_all(&artifacts).map_err(|e| Error::Other(e.to_string()))?;
    let lock_dir = std::env::temp_dir().join("autolab-emulator-locks");
    std::fs::create_dir_all(&lock_dir).map_err(|e| Error::Other(e.to_string()))?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_dir.join(format!("{}.lock", suite.environment.emulator)))
        .map_err(|e| Error::Other(e.to_string()))?;
    lock.try_lock_exclusive().map_err(|_| {
        Error::SessionConflict(format!(
            "emulator {} is already leased by another suite",
            suite.environment.emulator
        ))
    })?;
    let adb = PathBuf::from("adb");
    let mut lease = Lease {
        exe: std::env::current_exe().map_err(|e| Error::Other(e.to_string()))?,
        runtime: std::env::temp_dir().join(format!("autolab-suite-{}-{nonce}", std::process::id())),
        adb,
        serial: None,
        forward: None,
        emulator_child: None,
        _lock: lock,
        logcat: None,
        artifacts: artifacts.clone(),
    };
    let serial = match find_emulator(&lease.adb, &suite.environment.emulator)? {
        Some(serial) => serial,
        None => {
            let emulator = std::env::var_os("ANDROID_HOME")
                .or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
                .map(|sdk| PathBuf::from(sdk).join("emulator/emulator"))
                .filter(|path| path.is_file())
                .unwrap_or_else(|| PathBuf::from("emulator"));
            let file = std::fs::File::create(artifacts.join("emulator.log"))
                .map_err(|e| Error::Other(e.to_string()))?;
            let mut command = Command::new(emulator);
            command.args(["-avd", &suite.environment.emulator, "-no-snapshot-save"]);
            if suite.environment.headless {
                command.arg("-no-window");
            }
            lease.emulator_child = Some(
                command
                    .stdin(Stdio::null())
                    .stdout(file.try_clone().map_err(|e| Error::Other(e.to_string()))?)
                    .stderr(file)
                    .spawn()
                    .map_err(|e| {
                        Error::BackendUnavailable(format!("cannot start Android SDK emulator: {e}"))
                    })?,
            );
            let deadline = Instant::now() + Duration::from_secs(180);
            loop {
                check_cancel(&cancel)?;
                if let Some(serial) = find_emulator(&lease.adb, &suite.environment.emulator)? {
                    lease.serial = Some(serial.clone());
                    check_cancel(&cancel)?;
                    if device::adb(
                        &lease.adb,
                        &serial,
                        &["shell", "getprop", "sys.boot_completed"],
                    )
                    .is_ok_and(|value| value.trim() == "1")
                    {
                        break serial;
                    }
                }
                if Instant::now() >= deadline {
                    return Err(Error::Timeout(
                        "emulator did not boot within 180s; see emulator.log".into(),
                    ));
                }
                if lease
                    .emulator_child
                    .as_mut()
                    .unwrap()
                    .try_wait()
                    .map_err(|e| Error::Other(e.to_string()))?
                    .is_some()
                {
                    return Err(Error::BackendUnavailable(
                        "emulator exited during startup; see emulator.log".into(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    };
    lease.serial = Some(serial.clone());
    let apk = suite
        .environment
        .apk
        .to_str()
        .ok_or_else(|| Error::InvalidArgs("APK path is not UTF-8".into()))?;
    check_cancel(&cancel)?;
    device::adb(&lease.adb, &serial, &["install", "-r", "-t", apk])?;
    check_cancel(&cancel)?;
    device::restart_head_unit_server(&lease.adb, &serial)?;
    check_cancel(&cancel)?;
    let port = device::adb(&lease.adb, &serial, &["forward", "tcp:0", "tcp:5277"])?;
    let port: u16 = port
        .trim()
        .parse()
        .map_err(|_| Error::Other("adb did not return its allocated forwarding port".into()))?;
    lease.forward = Some(format!("tcp:{port}"));
    let log = std::fs::File::create(artifacts.join("logcat.txt"))
        .map_err(|e| Error::Other(e.to_string()))?;
    lease.logcat = Some(
        Command::new(&lease.adb)
            .args(["-s", &serial, "logcat", "-T", "1"])
            .stdin(Stdio::null())
            .stdout(log)
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Error::Other(e.to_string()))?,
    );
    let endpoint = format!("127.0.0.1:{port}");
    suite_cli(
        &lease.exe,
        &lease.runtime,
        &[
            "start",
            "--backend",
            "native",
            "--serial",
            &serial,
            "--native-tcp",
            &endpoint,
        ],
        Some(&cancel),
    )?;
    suite_cli(
        &lease.exe,
        &lease.runtime,
        &["focus", "video", "on"],
        Some(&cancel),
    )?;
    let metadata = serde_json::json!({"emulator": suite.environment.emulator, "serial": serial,
        "package": suite.environment.package, "apk": suite.environment.apk, "transport":"tcp"});
    std::fs::write(
        artifacts.join("suite.json"),
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .map_err(|e| Error::Other(e.to_string()))?;
    opts.cancel = Some(Arc::clone(&cancel));
    opts.paths = tests;
    opts.runtime_dir = Some(lease.runtime.clone());
    opts.app = Some(Application {
        label: suite.environment.app_label,
        adb: lease.adb.clone(),
        serial,
        package: suite.environment.package,
    });
    opts.artifacts_dir = Some(artifacts);
    if opts.report.is_empty() {
        opts.report = vec![test_cmd::ReportFormat::Json, test_cmd::ReportFormat::Junit];
    }
    let result = test_cmd::run(opts);
    // Capture daemon protocol logs before removing its private runtime (never archive daemon.json).
    let _ = std::fs::copy(
        lease.runtime.join("daemon.log"),
        lease.artifacts.join("head-unit.log"),
    );
    lease.cleanup()?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unknown_suite_fields_and_versions_before_devices() {
        let dir = std::env::temp_dir().join(format!("aa-suite-schema-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("suite.yaml");
        for text in [
            "version: 2\nenvironment: {emulator: medium_phone, apk: app.apk, package: test.app, appLabel: Test}\ntests: []",
            "version: 1\nenvironment: {emulator: medium_phone, apk: app.apk, package: test.app, appLabel: Test, typo: true}\ntests: []",
        ] {
            std::fs::write(&path, text).unwrap();
            assert!(load(&path).is_err());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

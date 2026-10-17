//! Own one isolated phone or emulator projection session while executing ordinary YAML tests.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::{device, test_cmd};
use fs2::FileExt;
use opendhu_core::{Error, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Suite {
    version: u32,
    #[serde(default, rename = "artifactsDir")]
    artifacts_dir: Option<PathBuf>,
    environment: Environment,
    tests: Vec<String>,
    #[serde(default)]
    services: Vec<crate::services::Service>,
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
    #[serde(default)]
    permissions: Vec<String>,
    launch_activity: Option<String>,
    #[serde(default)]
    reverse_ports: Vec<ReversePort>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReversePort {
    device: u16,
    host: u16,
}

fn yes() -> bool {
    true
}

#[derive(Clone)]
pub struct Application {
    pub label: String,
    adb: PathBuf,
    serial: String,
    package: String,
    pub monitor: Arc<crate::crash_monitor::CrashMonitor>,
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

impl opendhu_test::runner::ApplicationControl for Application {
    fn check_health(&self) -> Result<Option<String>> {
        self.monitor.check()
    }
    fn restart(&self) -> Result<()> {
        self.reset()
    }
    fn input_text(&self, text: &str) -> Result<()> {
        opendhu_test::schema::validate_input_text(text)?;
        device::adb(
            &self.adb,
            &self.serial,
            &["shell", "input", "text", &text.replace(' ', "%s")],
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
    if env.permissions.iter().any(|permission| {
        !permission.starts_with("android.permission.")
            || !permission
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
    }) {
        return Err(Error::InvalidArgs(
            "suite permissions must be Android permission names".into(),
        ));
    }
    if env.launch_activity.as_ref().is_some_and(|activity| {
        activity.is_empty()
            || !activity
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '$')
    }) {
        return Err(Error::InvalidArgs(
            "suite launchActivity must be a class name".into(),
        ));
    }
    let mut ports = std::collections::BTreeSet::new();
    for port in &env.reverse_ports {
        if port.device == 0 || port.host == 0 || !ports.insert(port.device) {
            return Err(Error::InvalidArgs(
                "suite reverse port numbers must be nonzero and device ports unique".into(),
            ));
        }
    }
    let mut services = std::collections::BTreeSet::new();
    let mut ready_ports = std::collections::BTreeSet::new();
    for service in &suite.services {
        service.validate()?;
        if !services.insert(&service.name) || !ready_ports.insert(service.ready_port) {
            return Err(Error::InvalidArgs(
                "suite service names and ready ports must be unique".into(),
            ));
        }
    }
    let base = path.parent().unwrap_or(Path::new("."));
    if !suite.environment.apk.is_absolute() {
        suite.environment.apk = base.join(&suite.environment.apk);
    }
    if let Some(dir) = &mut suite.artifacts_dir {
        if dir.as_os_str().is_empty() {
            return Err(Error::InvalidArgs(
                "suite artifactsDir must not be empty".into(),
            ));
        }
        if !dir.is_absolute() {
            *dir = base.join(&*dir);
        }
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

fn find_usb_phone(adb: &Path) -> Result<String> {
    let output = device::command(adb, &["devices", "-l"], Duration::from_secs(20))?;
    let devices = opendhu_dhu::adb::parse_devices(&output);
    let phones: Vec<_> = devices.iter().filter(|device| device.usb).collect();
    match phones.as_slice() {
        [] => Err(Error::DeviceDisconnected(
            "no USB phone found; connect a phone, enable USB debugging and authorize this computer"
                .into(),
        )),
        [phone] if phone.state == "device" => Ok(phone.serial.clone()),
        [phone] if phone.state == "unauthorized" => Err(Error::DeviceDisconnected(format!(
            "USB phone {} is unauthorized; unlock it and accept the USB debugging prompt",
            phone.serial,
        ))),
        [phone] => Err(Error::DeviceDisconnected(format!(
            "USB phone {} is {}; reconnect it and check `adb devices -l`",
            phone.serial, phone.state,
        ))),
        _ => Err(Error::InvalidArgs(format!(
            "multiple USB phones attached ({}); unplug the others before using --usb",
            phones
                .iter()
                .map(|phone| phone.serial.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ))),
    }
}

fn suite_cli(exe: &Path, runtime: &Path, args: &[&str], cancel: Option<&AtomicBool>) -> Result<()> {
    let mut child = Command::new(exe)
        .args(args)
        .env("OPENDHU_RUNTIME_DIR", runtime)
        .env("OPENDHU_NATIVE_RESOLUTION", "480p")
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
    reverses: Vec<crate::reverse::ReverseLease>,
    services: Vec<crate::services::OwnedService>,
    emulator_child: Option<std::process::Child>,
    _lock: std::fs::File,
    logcat: Option<Arc<crate::crash_monitor::CrashMonitor>>,
    artifacts: PathBuf,
}
impl Lease {
    fn acquire_reverses(
        &mut self,
        serial: &str,
        ports: &[ReversePort],
        output: &crate::test_output::Output,
        cancel: &AtomicBool,
    ) -> Result<()> {
        for port in ports {
            check_cancel(cancel)?;
            test_cmd::progress(
                output,
                format!(
                    "Forwarding device port {} to host {}...",
                    port.device, port.host
                ),
            );
            self.reverses.push(crate::reverse::ReverseLease::acquire(
                &self.adb,
                serial,
                port.device,
                port.host,
            )?);
        }
        Ok(())
    }

    fn cleanup(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        // Leaving USB accessory mode disconnects ADB, so remove tunnels before stopping projection.
        for mut reverse in self.reverses.drain(..).rev() {
            if let Err(e) = reverse.cleanup() {
                errors.push(e.message().to_string());
            }
        }
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
        if let Some(monitor) = self.logcat.take() {
            monitor.stop();
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
        while let Some(service) = self.services.pop() {
            drop(service);
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

pub(crate) fn inspect(
    path: &Path,
    filter: Option<&str>,
) -> Result<Vec<(String, opendhu_test::TestFile)>> {
    let (_, tests) = load(path)?;
    crate::test_selection::matching(test_cmd::parse_all(&tests)?, filter)
}

pub fn run(mut opts: test_cmd::Options) -> Result<u8> {
    let output = Arc::clone(&opts.output);
    let (suite, tests) = load(&opts.paths[0])?;
    let selected =
        crate::test_selection::filter(test_cmd::parse_all(&tests)?, opts.filter.take().as_deref())?;
    let tests: Vec<PathBuf> = selected
        .into_iter()
        .map(|(file, _)| PathBuf::from(file))
        .collect();
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
    test_cmd::progress(
        &output,
        format!("Suite: {} ({} tests)", opts.paths[0].display(), tests.len()),
    );
    let cancel = match &opts.cancel {
        Some(cancel) => Arc::clone(cancel),
        None => {
            let cancel = Arc::new(AtomicBool::new(false));
            test_cmd::install_interrupt_handler(&cancel)?;
            cancel
        }
    };
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

fn artifact_path(explicit: Option<PathBuf>, suite_base: Option<PathBuf>, nonce: u128) -> PathBuf {
    explicit.unwrap_or_else(|| {
        suite_base
            .unwrap_or_else(|| PathBuf::from("build/test-results"))
            .join(format!("run-{nonce}"))
    })
}

fn run_prepared(
    mut opts: test_cmd::Options,
    suite: Suite,
    tests: Vec<PathBuf>,
    cancel: Arc<AtomicBool>,
) -> Result<u8> {
    let output = Arc::clone(&opts.output);
    check_cancel(&cancel)?;
    if !suite.environment.apk.is_file() {
        return Err(Error::InvalidArgs(format!(
            "suite APK not found: {}; build it before running the suite",
            suite.environment.apk.display()
        )));
    }
    test_cmd::progress(&output, "Checking test tools...");
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
    let artifacts = artifact_path(
        opts.artifacts_dir.take(),
        suite.artifacts_dir.clone(),
        nonce,
    );
    let artifacts = if artifacts.is_absolute() {
        artifacts
    } else {
        cwd.join(artifacts)
    };
    std::fs::create_dir_all(&artifacts).map_err(|e| Error::Other(e.to_string()))?;
    test_cmd::progress(&output, format!("Artifacts: {}", artifacts.display()));
    let adb = PathBuf::from("adb");
    let usb_serial = if opts.usb {
        test_cmd::progress(&output, "Finding a USB phone...");
        Some(find_usb_phone(&adb)?)
    } else {
        None
    };
    let lock_dir = std::env::temp_dir().join(if opts.usb {
        "opendhu-usb-locks"
    } else {
        "opendhu-emulator-locks"
    });
    let device_name = usb_serial.as_deref().unwrap_or(&suite.environment.emulator);
    // USB has one global lease because the native USB transport cannot select among phones.
    let lock_name = if opts.usb {
        "phone"
    } else {
        &suite.environment.emulator
    };
    std::fs::create_dir_all(&lock_dir).map_err(|e| Error::Other(e.to_string()))?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_dir.join(format!("{lock_name}.lock")))
        .map_err(|e| Error::Other(e.to_string()))?;
    lock.try_lock_exclusive().map_err(|_| {
        Error::SessionConflict(format!(
            "device {device_name} is already leased by another suite"
        ))
    })?;
    let mut lease = Lease {
        exe: std::env::current_exe().map_err(|e| Error::Other(e.to_string()))?,
        runtime: std::env::temp_dir().join(format!("opendhu-suite-{}-{nonce}", std::process::id())),
        adb,
        serial: None,
        forward: None,
        reverses: Vec::new(),
        services: Vec::new(),
        emulator_child: None,
        _lock: lock,
        logcat: None,
        artifacts: artifacts.clone(),
    };
    let base = opts.paths[0].parent().unwrap_or(Path::new("."));
    for spec in &suite.services {
        check_cancel(&cancel)?;
        test_cmd::progress(
            &output,
            format!(
                "Starting service {} (waiting for port {})...",
                spec.name, spec.ready_port
            ),
        );
        lease.services.push(crate::services::OwnedService::spawn(
            spec, base, &artifacts,
        )?);
        lease
            .services
            .last_mut()
            .unwrap()
            .wait_ready(spec, &cancel)?;
        test_cmd::progress(&output, format!("Service {} ready", spec.name));
    }
    let serial = if let Some(serial) = usb_serial {
        test_cmd::progress(&output, format!("Using USB phone {serial}"));
        serial
    } else {
        test_cmd::progress(
            &output,
            format!("Finding emulator {}...", suite.environment.emulator),
        );
        match find_emulator(&lease.adb, &suite.environment.emulator)? {
            Some(serial) => {
                test_cmd::progress(&output, format!("Reusing emulator {serial}"));
                serial
            }
            None => {
                test_cmd::progress(
                    &output,
                    format!(
                        "Booting emulator {}{} (up to 180s)...",
                        suite.environment.emulator,
                        if suite.environment.headless {
                            " headlessly"
                        } else {
                            ""
                        }
                    ),
                );
                let emulator = std::env::var_os("ANDROID_HOME")
                    .or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
                    .map(|sdk| PathBuf::from(sdk).join("emulator/emulator"))
                    .filter(|path| path.is_file())
                    .unwrap_or_else(|| PathBuf::from("emulator"));
                let file = std::fs::File::create(artifacts.join("emulator.log"))
                    .map_err(|e| Error::Other(e.to_string()))?;
                let mut command = Command::new(emulator);
                command.args(["-avd", &suite.environment.emulator, "-no-snapshot"]);
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
                            Error::BackendUnavailable(format!(
                                "cannot start Android SDK emulator: {e}"
                            ))
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
        }
    };
    lease.serial = Some(serial.clone());
    let apk = suite
        .environment
        .apk
        .to_str()
        .ok_or_else(|| Error::InvalidArgs("APK path is not UTF-8".into()))?;
    check_cancel(&cancel)?;
    test_cmd::progress(
        &output,
        format!(
            "Installing {} on {serial}...",
            suite.environment.apk.display()
        ),
    );
    device::adb(&lease.adb, &serial, &["install", "-r", "-t", apk])?;
    check_cancel(&cancel)?;
    for permission in &suite.environment.permissions {
        check_cancel(&cancel)?;
        test_cmd::progress(&output, format!("Granting {permission}..."));
        device::adb(
            &lease.adb,
            &serial,
            &[
                "shell",
                "pm",
                "grant",
                &suite.environment.package,
                permission,
            ],
        )?;
    }
    if !opts.usb {
        lease.acquire_reverses(&serial, &suite.environment.reverse_ports, &output, &cancel)?;
    }
    let endpoint = if opts.usb {
        None
    } else {
        test_cmd::progress(&output, "Restarting the Android Auto head-unit server...");
        device::restart_head_unit_server(&lease.adb, &serial)?;
        check_cancel(&cancel)?;
        let port = device::adb(&lease.adb, &serial, &["forward", "tcp:0", "tcp:5277"])?;
        let port: u16 = port
            .trim()
            .parse()
            .map_err(|_| Error::Other("adb did not return its allocated forwarding port".into()))?;
        lease.forward = Some(format!("tcp:{port}"));
        Some(format!("127.0.0.1:{port}"))
    };
    test_cmd::progress(
        &output,
        "Starting the native head unit and waiting for projection...",
    );
    let mut start_args = vec!["start", "--backend", "native", "--serial", &serial];
    if let Some(endpoint) = &endpoint {
        start_args.extend(["--native-tcp", endpoint]);
    }
    suite_cli(&lease.exe, &lease.runtime, &start_args, Some(&cancel))?;
    if opts.usb {
        test_cmd::progress(&output, "Waiting for USB phone to return to ADB...");
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            check_cancel(&cancel)?;
            if device::command(
                &lease.adb,
                &["-s", &serial, "get-state"],
                Duration::from_secs(2),
            )
            .is_ok_and(|state| state.trim() == "device")
            {
                break;
            }
            if Instant::now() >= deadline {
                return Err(Error::DeviceDisconnected(format!(
                    "USB phone {serial} did not return to ADB within 30s after projection started",
                )));
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        lease.acquire_reverses(&serial, &suite.environment.reverse_ports, &output, &cancel)?;
    }
    let monitor = Arc::new(crate::crash_monitor::CrashMonitor::start(
        &lease.adb,
        &serial,
        &suite.environment.package,
        &artifacts.join("logcat.txt"),
    )?);
    lease.logcat = Some(Arc::clone(&monitor));
    if let Some(activity) = &suite.environment.launch_activity {
        let component = format!("{}/{activity}", suite.environment.package);
        test_cmd::progress(&output, format!("Launching {component}..."));
        device::adb(
            &lease.adb,
            &serial,
            &["shell", "am", "start", "-W", "-n", &component],
        )?;
    }
    test_cmd::progress(&output, "Requesting video focus...");
    suite_cli(
        &lease.exe,
        &lease.runtime,
        &["focus", "video", "on"],
        Some(&cancel),
    )?;
    let metadata = serde_json::json!({
        "emulator": if opts.usb { None } else { Some(&suite.environment.emulator) },
        "serial": serial, "package": suite.environment.package, "apk": suite.environment.apk,
        "transport": if opts.usb { "usb" } else { "tcp" },
    });
    std::fs::write(
        artifacts.join("suite.json"),
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .map_err(|e| Error::Other(e.to_string()))?;
    opts.cancel = Some(Arc::clone(&cancel));
    opts.usb = false; // Device setup is complete; ordinary tests use the leased session.
    opts.paths = tests;
    opts.runtime_dir = Some(lease.runtime.clone());
    opts.app = Some(Application {
        label: suite.environment.app_label,
        monitor,
        adb: lease.adb.clone(),
        serial,
        package: suite.environment.package,
    });
    opts.artifacts_dir = Some(artifacts);
    if opts.report.is_empty() {
        opts.report = vec![
            test_cmd::ReportFormat::Json,
            test_cmd::ReportFormat::Junit,
            test_cmd::ReportFormat::Html,
        ];
    }
    let result = test_cmd::run(opts);
    test_cmd::progress(
        &output,
        "Cleaning up the suite session and owned resources...",
    );
    // Capture daemon protocol logs before removing its private runtime (never archive daemon.json).
    let _ = std::fs::copy(
        lease.runtime.join("daemon.log"),
        lease.artifacts.join("head-unit.log"),
    );
    lease.cleanup()?;
    test_cmd::progress(&output, "Suite cleanup complete");
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

    #[test]
    fn suite_artifacts_are_relative_to_yaml_and_cli_override_is_exact() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/android/e2e/suite.yaml");
        let (suite, _) = load(&path).unwrap();
        let base = path.parent().unwrap().join("../build/test-results");
        assert_eq!(suite.artifacts_dir, Some(base.clone()));
        assert_eq!(
            artifact_path(None, suite.artifacts_dir.clone(), 42),
            base.join("run-42")
        );
        assert_eq!(
            artifact_path(Some("custom".into()), suite.artifacts_dir, 42),
            PathBuf::from("custom")
        );
        assert_eq!(
            artifact_path(None, None, 42),
            PathBuf::from("build/test-results/run-42")
        );
    }
}

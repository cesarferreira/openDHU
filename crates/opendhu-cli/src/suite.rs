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
pub(crate) struct Suite {
    version: u32,
    #[serde(default, rename = "artifactsDir")]
    artifacts_dir: Option<PathBuf>,
    pub(crate) environment: Environment,
    tests: Vec<String>,
    #[serde(default)]
    pub(crate) services: Vec<crate::services::Service>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Environment {
    pub(crate) emulator: String,
    #[serde(default = "yes")]
    headless: bool,
    #[serde(default)]
    dns_servers: Vec<std::net::IpAddr>,
    /// `-gpu` mode for an emulator openDHU boots. Defaults to `swiftshader_indirect` when headless.
    gpu: Option<String>,
    /// Keep Android's window/transition/animator animations on. Off by default: they dominate
    /// cost under software rendering and make OCR frames flaky.
    #[serde(default)]
    animations: bool,
    /// Extra emulator arguments, one per entry, e.g. `["-cores", "6", "-memory", "3072"]`.
    #[serde(default)]
    emulator_args: Vec<String>,
    pub(crate) apk: PathBuf,
    /// Run before every install so the APK always matches the checked-out branch.
    build: Option<Build>,
    package: String,
    app_label: String,
    #[serde(default)]
    permissions: Vec<String>,
    launch_activity: Option<String>,
    #[serde(default)]
    reverse_ports: Vec<ReversePort>,
    /// App preferences to set before launch, e.g. a developer feature flag a journey needs.
    preferences: Option<Preferences>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Build {
    /// Program and arguments, e.g. `["./gradlew", ":app:assembleDebug"]`.
    command: Vec<String>,
    /// Defaults to the directory of the suite file that declares `build`.
    cwd: Option<PathBuf>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Preferences {
    /// SharedPreferences file name, without `.xml`.
    pub(crate) file: String,
    #[serde(default)]
    pub(crate) booleans: std::collections::BTreeMap<String, bool>,
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
    fn projection_blocker(&self) -> Result<Option<String>> {
        let windows = device::adb(&self.adb, &self.serial, &["shell", "dumpsys", "window"])?;
        Ok(first_run_setup_blocker(&windows))
    }
    fn restart(&self) -> Result<()> {
        self.reset()
    }
    fn phone_ui_dump(&self) -> Result<String> {
        // While projecting, the phone sits on an Android Auto screen and `uiautomator` dumps the
        // active window, so put the app's own phone UI in front first. A no-op if it already is.
        device::adb(
            &self.adb,
            &self.serial,
            &[
                "shell",
                "monkey",
                "-p",
                &self.package,
                "-c",
                "android.intent.category.LAUNCHER",
                "1",
            ],
        )?;
        std::thread::sleep(std::time::Duration::from_millis(700));
        device::adb(
            &self.adb,
            &self.serial,
            &["exec-out", "uiautomator", "dump", "/dev/tty"],
        )
    }
    fn car_template(&self) -> Result<String> {
        device::adb(
            &self.adb,
            &self.serial,
            &[
                "shell",
                "content",
                "query",
                "--uri",
                &format!("content://{}.opendhu/template", self.package),
            ],
        )
    }
    fn car_app_display(&self) -> Result<Option<bool>> {
        let displays = device::adb(&self.adb, &self.serial, &["shell", "dumpsys", "display"])?;
        Ok(Some(has_car_app_display(&displays, &self.package)))
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

/// Android Auto gives each projecting app a `CarAppService` virtual display owned by its package.
fn has_car_app_display(dumpsys_display: &str, package: &str) -> bool {
    let owner = format!("virtual:{package},");
    dumpsys_display
        .lines()
        .any(|l| l.contains(&owner) && l.contains(",CarAppService,"))
}

/// Android Auto keeps a phone-side setup screen in front until first-run welcome and consent
/// are finished, and projection never starts behind it.
fn first_run_setup_blocker(dumpsys_window: &str) -> Option<String> {
    let focus = dumpsys_window
        .lines()
        .find(|line| line.contains("mCurrentFocus="))?;
    (focus.contains("com.google.android.projection.gearhead") && focus.contains(".preflight."))
        .then(|| {
            "Android Auto has not finished first-run setup on the phone (it is still on its \
             welcome/authorization screen). Open Android Auto on this emulator, accept the \
             welcome and permission prompts, and keep that state as the emulator baseline"
                .into()
        })
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
    Ok(value.get("environment").is_some()
        || value.get("version").is_some()
        || value.get("extends").is_some())
}

pub(crate) fn load(path: &Path) -> Result<(Suite, Vec<PathBuf>)> {
    let suite: Suite = serde_norway::from_value(crate::suite_config::resolve(path)?)
        .map_err(|e| Error::InvalidArgs(format!("invalid suite {}: {e}", path.display())))?;
    let env = &suite.environment;
    if env.dns_servers.len() > 4 {
        return Err(Error::InvalidArgs(
            "suite dnsServers must contain at most four IP addresses".into(),
        ));
    }
    if env.gpu.as_deref().is_some_and(|gpu| {
        gpu.is_empty() || !gpu.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    }) {
        return Err(Error::InvalidArgs(
            "suite gpu must be an emulator -gpu mode such as swiftshader_indirect or host".into(),
        ));
    }
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
    if env
        .build
        .as_ref()
        .is_some_and(|b| b.command.is_empty() || b.command[0].is_empty())
    {
        return Err(Error::InvalidArgs(
            "suite build requires a nonempty command".into(),
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
    if let Some(preferences) = &env.preferences {
        let name_ok = |name: &str| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        };
        if !name_ok(&preferences.file) || !preferences.booleans.keys().all(|k| name_ok(k)) {
            return Err(Error::InvalidArgs(
                "suite preferences file and keys may only use letters, digits, '.', '_' and '-'"
                    .into(),
            ));
        }
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
    if suite
        .artifacts_dir
        .as_ref()
        .is_some_and(|dir| dir.as_os_str().is_empty())
    {
        return Err(Error::InvalidArgs(
            "suite artifactsDir must not be empty".into(),
        ));
    }
    let mut tests = Vec::new();
    for pattern in &suite.tests {
        let pattern = PathBuf::from(pattern);
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

/// `sdk.dir` from the nearest `local.properties` at or above `start` (Gradle/Android Studio).
fn sdk_dir_from_local_properties(start: &Path) -> Option<PathBuf> {
    start.ancestors().find_map(|dir| {
        let text = std::fs::read_to_string(dir.join("local.properties")).ok()?;
        text.lines().find_map(|line| {
            let value = line.trim().strip_prefix("sdk.dir")?.trim_start();
            let value = value.strip_prefix(['=', ':'])?.trim();
            // Java properties escape `\\`, `\:` and friends.
            let mut path = String::new();
            let mut chars = value.chars();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    path.extend(chars.next());
                } else {
                    path.push(c);
                }
            }
            Some(PathBuf::from(path))
        })
    })
}

/// The SDK root implied by a tool's real location, e.g. `<sdk>/platform-tools/adb`.
fn sdk_root_of_tool(tool: &Path) -> Option<PathBuf> {
    let real = tool.canonicalize().ok()?;
    real.ancestors()
        .skip(1)
        .find(|dir| {
            matches!(
                dir.file_name().and_then(|n| n.to_str()),
                Some("platform-tools" | "emulator" | "cmdline-tools" | "tools")
            )
        })
        .and_then(|dir| dir.parent())
        .map(Path::to_path_buf)
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

/// Find an Android SDK tool without assuming where the SDK lives. In order:
/// `ANDROID_HOME`/`ANDROID_SDK_ROOT` and `PATH` (in the order `path_first` selects, since adb
/// has always preferred `PATH` and the emulator the SDK), the project's `local.properties` `sdk.dir`, the
/// SDK that any other SDK tool on `PATH` belongs to, then common install directories.
/// A bare `name` is the last resort, so errors name the missing tool.
fn sdk_tool(name: &str, relative: &str, path_first: bool) -> PathBuf {
    let in_roots = |roots: Vec<PathBuf>| {
        roots
            .into_iter()
            .map(|root| root.join(relative))
            .find(|path| path.is_file())
    };
    let from_env = || in_roots(opendhu_dhu::adb::sdk_roots_from_env());
    let from_path = || on_path(name);
    let found = if path_first {
        from_path().or_else(from_env)
    } else {
        from_env().or_else(from_path)
    };
    found
        .or_else(|| {
            let cwd = std::env::current_dir().ok()?;
            in_roots(sdk_dir_from_local_properties(&cwd).into_iter().collect())
        })
        .or_else(|| {
            in_roots(
                ["adb", "emulator", "avdmanager", "sdkmanager"]
                    .into_iter()
                    .filter_map(on_path)
                    .filter_map(|tool| sdk_root_of_tool(&tool))
                    .collect(),
            )
        })
        .or_else(|| {
            let home = PathBuf::from(std::env::var_os("HOME")?);
            in_roots(vec![
                home.join("Android/Sdk"),
                home.join("Library/Android/sdk"),
            ])
        })
        .unwrap_or_else(|| PathBuf::from(name))
}

pub(crate) fn adb_program() -> PathBuf {
    sdk_tool("adb", "platform-tools/adb", true)
}

pub(crate) fn emulator_program() -> PathBuf {
    sdk_tool("emulator", "emulator/emulator", false)
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

pub(crate) fn find_usb_phone(adb: &Path) -> Result<String> {
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

/// Run-scoped ownership keeps compatible suites on the same emulator, with its AVD locked.
#[derive(Default)]
pub struct EmulatorPool {
    locks: std::collections::BTreeMap<String, std::fs::File>,
    owned: Vec<(PathBuf, String, std::process::Child)>,
}

impl Drop for EmulatorPool {
    fn drop(&mut self) {
        for (adb, serial, child) in &mut self.owned {
            stop_emulator(adb, Some(serial), child);
        }
    }
}

fn stop_emulator(adb: &Path, serial: Option<&str>, child: &mut std::process::Child) {
    if child.try_wait().is_ok_and(|status| status.is_none())
        && let Some(serial) = serial
    {
        let _ = device::adb(adb, serial, &["emu", "kill"]);
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn lock_device(usb: bool, device_name: &str) -> Result<std::fs::File> {
    let lock_dir = std::env::temp_dir().join(if usb {
        "opendhu-usb-locks"
    } else {
        "opendhu-emulator-locks"
    });
    // USB has one global lease because the native transport cannot select among phones.
    let lock_name = if usb { "phone" } else { device_name };
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
    Ok(lock)
}

/// Ownership is recorded as soon as acquired. A failed startup still tears down its forwarding and
/// private daemon, and never stops a phone session or an emulator it did not start.
struct Lease {
    exe: PathBuf,
    runtime: PathBuf,
    adb: PathBuf,
    serial: Option<String>,
    first_run: Option<crate::phone_setup::FirstRunWatcher>,
    forward: Option<String>,
    reverses: Vec<crate::reverse::ReverseLease>,
    services: Vec<crate::services::OwnedService>,
    emulator_child: Option<std::process::Child>,
    _lock: Option<std::fs::File>,
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
            stop_emulator(&self.adb, self.serial.as_deref(), &mut child);
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

pub fn run(mut opts: test_cmd::Options, emulators: Option<&mut EmulatorPool>) -> Result<u8> {
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
        match opts.position.suite {
            Some((n, of)) => format!(
                "Suite: {} ({} tests) [suite {n}/{of}]",
                opts.paths[0].display(),
                tests.len()
            ),
            None => format!("Suite: {} ({} tests)", opts.paths[0].display(), tests.len()),
        },
    );
    let cancel = match &opts.cancel {
        Some(cancel) => Arc::clone(cancel),
        None => {
            let cancel = Arc::new(AtomicBool::new(false));
            test_cmd::install_interrupt_handler(&cancel)?;
            cancel
        }
    };
    let result = run_prepared(opts, suite, tests, Arc::clone(&cancel), emulators);
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

/// Without `--artifacts-dir` a suite reports into one fixed folder that each run replaces.
fn artifact_path(explicit: Option<PathBuf>, suite_base: Option<PathBuf>) -> PathBuf {
    explicit.unwrap_or_else(|| suite_base.unwrap_or_else(|| PathBuf::from("build/test-results")))
}

/// Print a setup phase, and how long the previous one took when that was noticeable, so a slow
/// suite start shows which step to look at.
fn phase(output: &crate::test_output::Output, message: impl std::fmt::Display) {
    static LAST: std::sync::Mutex<Option<(String, Instant)>> = std::sync::Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((previous, started)) = last.take()
        && started.elapsed() >= Duration::from_secs(1)
    {
        test_cmd::progress(
            output,
            // A trailing "..." would make the output layer draw this as a transient spinner.
            format!(
                "  took {:.1}s: {}",
                started.elapsed().as_secs_f64(),
                previous.trim_end_matches('.')
            ),
        );
    }
    let message = message.to_string();
    if message.is_empty() {
        return;
    }
    *last = Some((message.clone(), Instant::now()));
    test_cmd::progress(output, message);
}

/// Build the APK before installing it, so switching branches never installs a stale one.
fn run_build(
    build: &Build,
    artifacts: &Path,
    output: &Arc<crate::test_output::Output>,
    cancel: &AtomicBool,
) -> Result<()> {
    check_cancel(cancel)?;
    phase(output, format!("Building: {}...", build.command.join(" ")));
    let log_path = artifacts.join("build.log");
    let log = std::fs::File::create(&log_path).map_err(|e| Error::Other(e.to_string()))?;
    let status = Command::new(&build.command[0])
        .args(&build.command[1..])
        .current_dir(build.cwd.as_deref().unwrap_or(Path::new(".")))
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| Error::Other(e.to_string()))?)
        .stderr(log)
        .status()
        .map_err(|e| Error::Other(format!("cannot run build {}: {e}", build.command[0])))?;
    if !status.success() {
        return Err(Error::Other(format!(
            "suite build failed ({status}); see {}",
            log_path.display()
        )));
    }
    check_cancel(cancel)
}

/// Identity of the APK file on the host: its size and modification time change on every rebuild.
fn apk_identity(apk: &Path) -> Option<(u64, String)> {
    let meta = std::fs::metadata(apk).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((
        meta.len(),
        format!("{}-{}", meta.len(), modified.as_nanos()),
    ))
}

fn apk_marker(environment: &Environment) -> String {
    format!("/data/local/tmp/opendhu-apk-{}", environment.package)
}

/// Installing a large debug APK takes many seconds, so a device that already runs exactly this
/// build (same size and modification time as when it was last installed by a suite, and the
/// package still installed at that size) keeps it. Any doubt means a normal install.
fn apk_is_installed(adb: &Path, serial: &str, environment: &Environment) -> bool {
    let Some((size, identity)) = apk_identity(&environment.apk) else {
        return false;
    };
    let recorded = device::adb(adb, serial, &["shell", "cat", &apk_marker(environment)]);
    if !recorded.is_ok_and(|text| text.trim() == identity) {
        return false;
    }
    let Ok(path) = device::adb(adb, serial, &["shell", "pm", "path", &environment.package]) else {
        return false;
    };
    let Some(path) = path
        .lines()
        .next()
        .and_then(|l| l.trim().strip_prefix("package:"))
    else {
        return false;
    };
    device::adb(adb, serial, &["shell", "stat", "-c", "%s", path.trim()])
        .is_ok_and(|text| text.trim().parse::<u64>() == Ok(size))
}

fn record_installed_apk(adb: &Path, serial: &str, environment: &Environment) {
    if let Some((_, identity)) = apk_identity(&environment.apk) {
        let _ = device::adb(
            adb,
            serial,
            &["shell", "echo", &identity, ">", &apk_marker(environment)],
        );
    }
}

fn run_prepared(
    mut opts: test_cmd::Options,
    suite: Suite,
    tests: Vec<PathBuf>,
    cancel: Arc<AtomicBool>,
    mut emulators: Option<&mut EmulatorPool>,
) -> Result<u8> {
    let setup_started = Instant::now();
    let suite_path = opts.paths[0].clone();
    let output = Arc::clone(&opts.output);
    check_cancel(&cancel)?;
    phase(&output, "Checking test tools...");
    // Fail before starting hardware when a required tool is missing.
    crate::suite_doctor::preflight()?;
    let cwd = std::env::current_dir().map_err(|e| Error::Other(e.to_string()))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let default_dir = opts.artifacts_dir.is_none();
    let artifacts = artifact_path(opts.artifacts_dir.take(), suite.artifacts_dir.clone());
    let artifacts = if artifacts.is_absolute() {
        artifacts
    } else {
        cwd.join(artifacts)
    };
    if default_dir {
        test_cmd::reset_artifacts(&artifacts, &[]);
    }
    std::fs::create_dir_all(&artifacts).map_err(|e| Error::Other(e.to_string()))?;
    output.artifacts(&artifacts);
    if let Some(build) = &suite.environment.build {
        run_build(build, &artifacts, &output, &cancel)?;
    }
    if !suite.environment.apk.is_file() {
        return Err(Error::InvalidArgs(format!(
            "suite APK not found: {}; build it before running the suite, or set environment.build",
            suite.environment.apk.display()
        )));
    }
    let adb = adb_program();
    let usb_serial = if opts.usb {
        phase(&output, "Finding a USB phone...");
        Some(find_usb_phone(&adb)?)
    } else {
        None
    };
    let device_name = usb_serial.as_deref().unwrap_or(&suite.environment.emulator);
    let lock = if !opts.usb
        && let Some(pool) = emulators.as_deref_mut()
    {
        if !pool.locks.contains_key(device_name) {
            pool.locks
                .insert(device_name.into(), lock_device(false, device_name)?);
        }
        None
    } else {
        Some(lock_device(opts.usb, device_name)?)
    };
    let mut lease = Lease {
        exe: std::env::current_exe().map_err(|e| Error::Other(e.to_string()))?,
        runtime: std::env::temp_dir().join(format!("opendhu-suite-{}-{nonce}", std::process::id())),
        adb,
        serial: None,
        forward: None,
        first_run: None,
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
        phase(
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
        phase(&output, format!("Service {} ready", spec.name));
    }
    let serial = if let Some(serial) = usb_serial {
        phase(&output, format!("Using USB phone {serial}"));
        serial
    } else {
        phase(
            &output,
            format!("Finding emulator {}...", suite.environment.emulator),
        );
        match find_emulator(&lease.adb, &suite.environment.emulator)? {
            Some(serial) => {
                phase(&output, format!("Reusing emulator {serial}"));
                serial
            }
            None => {
                phase(
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
                let emulator = emulator_program();
                let file = std::fs::File::create(artifacts.join("emulator.log"))
                    .map_err(|e| Error::Other(e.to_string()))?;
                let mut command = Command::new(emulator);
                command.args([
                    "-avd",
                    &suite.environment.emulator,
                    "-no-snapshot",
                    "-no-audio",
                    "-no-boot-anim",
                ]);
                if !suite.environment.dns_servers.is_empty() {
                    command.arg("-dns-server").arg(
                        suite
                            .environment
                            .dns_servers
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                }
                if suite.environment.headless {
                    command.arg("-no-window");
                }
                // Headless has no display or GPU: render in software explicitly instead of
                // letting `auto` probe.
                let gpu = suite
                    .environment
                    .gpu
                    .as_deref()
                    .or(suite.environment.headless.then_some("swiftshader_indirect"));
                if let Some(gpu) = gpu {
                    command.args(["-gpu", gpu]);
                }
                command.args(&suite.environment.emulator_args);
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
                            // A failure here is only a slowdown, so it is ignored.
                            for key in [
                                "window_animation_scale",
                                "transition_animation_scale",
                                "animator_duration_scale",
                            ]
                            .into_iter()
                            .filter(|_| !suite.environment.animations)
                            {
                                let _ = device::adb(
                                    &lease.adb,
                                    &serial,
                                    &["shell", "settings", "put", "global", key, "0"],
                                );
                            }
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
    if let Some(pool) = emulators
        && let Some(child) = lease.emulator_child.take()
    {
        pool.owned.push((lease.adb.clone(), serial.clone(), child));
    }
    let apk = suite
        .environment
        .apk
        .to_str()
        .ok_or_else(|| Error::InvalidArgs("APK path is not UTF-8".into()))?;
    check_cancel(&cancel)?;
    phase(
        &output,
        format!(
            "Installing {} on {serial}...",
            suite.environment.apk.display()
        ),
    );
    if apk_is_installed(&lease.adb, &serial, &suite.environment) {
        phase(
            &output,
            "APK unchanged since the last install; skipping it...",
        );
        // Installing stops the app; do the same so it starts fresh.
        device::adb(
            &lease.adb,
            &serial,
            &["shell", "am", "force-stop", &suite.environment.package],
        )?;
    } else {
        device::adb(&lease.adb, &serial, &["install", "-r", "-t", apk])?;
        record_installed_apk(&lease.adb, &serial, &suite.environment);
    }
    check_cancel(&cancel)?;
    for permission in &suite.environment.permissions {
        check_cancel(&cancel)?;
        phase(&output, format!("Granting {permission}..."));
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
    if let Some(preferences) = &suite.environment.preferences
        && !preferences.booleans.is_empty()
    {
        check_cancel(&cancel)?;
        phase(
            &output,
            format!(
                "Setting {} app preference(s)...",
                preferences.booleans.len()
            ),
        );
        device::set_boolean_preferences(
            &lease.adb,
            &serial,
            &suite.environment.package,
            &preferences.file,
            &preferences.booleans,
        )?;
    }
    if !opts.usb {
        lease.acquire_reverses(&serial, &suite.environment.reverse_ports, &output, &cancel)?;
    }
    let mut keep_server = false;
    let endpoint = if opts.usb {
        None
    } else {
        // The welcome sheet appears once the head unit connects; accept it for this run.
        if opts.phone_setup {
            lease.first_run = Some(crate::phone_setup::FirstRunWatcher::start(
                lease.adb.clone(),
                serial.clone(),
            ));
        }
        // The restart discards stale projection state. After a suite of this same run ended cleanly
        // on this emulator and its server is still up, there is none, so keep it.
        keep_server = cleanly_used(&serial)
            && device::head_unit_server_running(&lease.adb, &serial).unwrap_or(false);
        if keep_server {
            phase(&output, "Keeping the running Android Auto head-unit server");
        } else {
            phase(&output, "Restarting the Android Auto head-unit server...");
            device::restart_head_unit_server(&lease.adb, &serial)?;
        }
        check_cancel(&cancel)?;
        let port = device::adb(&lease.adb, &serial, &["forward", "tcp:0", "tcp:5277"])?;
        let port: u16 = port
            .trim()
            .parse()
            .map_err(|_| Error::Other("adb did not return its allocated forwarding port".into()))?;
        lease.forward = Some(format!("tcp:{port}"));
        Some(format!("127.0.0.1:{port}"))
    };
    phase(
        &output,
        "Starting the native head unit and waiting for projection...",
    );
    let mut start_args = vec!["start", "--backend", "native", "--serial", &serial];
    if let Some(endpoint) = &endpoint {
        start_args.extend(["--native-tcp", endpoint]);
    }
    if let Err(error) = suite_cli(&lease.exe, &lease.runtime, &start_args, Some(&cancel)) {
        if !keep_server {
            return Err(error);
        }
        // The kept server may have been in a bad state after all: restart it once and retry.
        check_cancel(&cancel)?;
        phase(
            &output,
            "Head unit did not connect; restarting the Android Auto server and retrying...",
        );
        device::restart_head_unit_server(&lease.adb, &serial)?;
        suite_cli(&lease.exe, &lease.runtime, &start_args, Some(&cancel))?;
    }
    if opts.usb {
        phase(&output, "Waiting for USB phone to return to ADB...");
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
        phase(&output, format!("Launching {component}..."));
        device::adb(
            &lease.adb,
            &serial,
            &["shell", "am", "start", "-W", "-n", &component],
        )?;
    }
    phase(&output, "Requesting video focus...");
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
    let mut timings = crate::timings::Timings::load();
    timings.record_setup(
        &suite_path,
        u64::try_from(setup_started.elapsed().as_millis()).unwrap_or(u64::MAX),
    );
    timings.save();
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
    phase(&output, "");
    let verbose = opts.verbose;
    let result = test_cmd::run_with_emulators(opts, None);
    if verbose {
        test_cmd::progress(
            &output,
            "Cleaning up the suite session and owned resources...",
        );
    }
    // Capture daemon protocol logs before removing its private runtime (never archive daemon.json).
    let _ = std::fs::copy(
        lease.runtime.join("daemon.log"),
        lease.artifacts.join("head-unit.log"),
    );
    let serial = lease.serial.clone();
    lease.cleanup()?;
    if let Some(serial) = serial {
        note_cleanly_used(&serial);
    }
    result
}

/// Emulators whose head-unit session an earlier suite of this run ended cleanly.
static CLEANLY_USED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn cleanly_used(serial: &str) -> bool {
    CLEANLY_USED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|s| s == serial)
}

fn note_cleanly_used(serial: &str) {
    let mut used = CLEANLY_USED.lock().unwrap_or_else(|e| e.into_inner());
    if !used.iter().any(|s| s == serial) {
        used.push(serial.to_string());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn car_app_display_is_matched_by_owner_package() {
        let dump = r#"DisplayDeviceInfo{"CarAppService": uniqueId="virtual:dev.opendhu.demo,10218,CarAppService,15", 1133 x 601}
DisplayDeviceInfo{"CarAppService": uniqueId="virtual:dev.opendhu.demo.other,10219,CarAppService,16", 1133 x 601}
DisplayDeviceInfo{"GhFacetBar": uniqueId="virtual:com.google.android.projection.gearhead,10135,GhFacetBar,18"}"#;
        assert!(super::has_car_app_display(dump, "dev.opendhu.demo"));
        assert!(!super::has_car_app_display(dump, "dev.opendhu"));
        assert!(!super::has_car_app_display(
            dump,
            "com.google.android.projection.gearhead"
        ));
    }

    use super::*;

    #[test]
    fn apk_identity_changes_whenever_the_file_does() {
        let dir = std::env::temp_dir().join(format!("aa-apk-identity-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let apk = dir.join("app.apk");
        std::fs::write(&apk, b"one").unwrap();
        let (size, first) = apk_identity(&apk).unwrap();
        assert_eq!(size, 3);
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&apk, b"three").unwrap();
        assert_ne!(apk_identity(&apk).unwrap().1, first);
        assert!(apk_identity(&dir.join("missing.apk")).is_none());
    }

    #[test]
    fn preferences_are_parsed_and_validated() {
        let dir = std::env::temp_dir().join(format!("aa-suite-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("test.yaml"),
            "name: T\nsteps: [{action: wait, duration: 1ms}]\n",
        )
        .unwrap();
        let suite = |prefs: &str| {
            let path = dir.join("suite.yaml");
            std::fs::write(&path, format!("version: 1\nenvironment: {{emulator: medium_phone, apk: app.apk, package: test.app, appLabel: Test, {prefs}}}\ntests: [test.yaml]\n")).unwrap();
            load(&path)
        };
        let ok = suite("preferences: {file: APPLICATION_SHARED_PREFERENCES, booleans: {EnableRobotaxiMode: true}}").unwrap().0;
        let prefs = ok.environment.preferences.unwrap();
        assert_eq!(prefs.file, "APPLICATION_SHARED_PREFERENCES");
        assert_eq!(prefs.booleans.get("EnableRobotaxiMode"), Some(&true));
        assert!(suite("preferences: {file: '../x', booleans: {}}").is_err());
        assert!(suite("preferences: {file: p, booleans: {'a b': true}}").is_err());
        assert!(suite("preferences: {file: p, strings: {a: b}}").is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn detects_android_auto_first_run_screen_in_front() {
        let stuck = "  mCurrentFocus=Window{4fbbe64 u0 com.google.android.projection.gearhead/com.google.android.apps.auto.components.preflight.phone.PreflightPhoneWelcomeActivity}\n";
        assert!(
            first_run_setup_blocker(stuck)
                .unwrap()
                .contains("first-run setup")
        );
        let normal =
            "  mCurrentFocus=Window{1 u0 dev.opendhu.demo/dev.opendhu.demo.MainActivity}\n";
        assert_eq!(first_run_setup_blocker(normal), None);
        let car = "  mCurrentFocus=Window{2 u0 com.google.android.projection.gearhead/com.google.android.gearhead.vanagon.VnLauncher}\n";
        assert_eq!(first_run_setup_blocker(car), None);
        assert_eq!(first_run_setup_blocker("no focus line"), None);
    }

    #[test]
    fn infers_sdk_from_local_properties_in_an_ancestor() {
        let root = std::env::temp_dir().join(format!("aa-sdk-props-{}", std::process::id()));
        let nested = root.join("app/e2e");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            root.join("local.properties"),
            "# c\nsdk.dir=/opt/my\\ sdk\\:x\nother=1\n",
        )
        .unwrap();
        assert_eq!(
            sdk_dir_from_local_properties(&nested),
            Some(PathBuf::from("/opt/my sdk:x"))
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn infers_sdk_root_from_a_symlinked_tool() {
        let root = std::env::temp_dir().join(format!("aa-sdk-tool-{}", std::process::id()));
        let tools = root.join("sdk/platform-tools");
        std::fs::create_dir_all(&tools).unwrap();
        std::fs::write(tools.join("adb"), "").unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::os::unix::fs::symlink(tools.join("adb"), root.join("bin/adb")).unwrap();
        assert_eq!(
            sdk_root_of_tool(&root.join("bin/adb")),
            Some(root.join("sdk").canonicalize().unwrap())
        );
        assert_eq!(sdk_root_of_tool(&root.join("missing")), None);
        std::fs::remove_dir_all(root).unwrap();
    }
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
    fn build_command_defaults_its_cwd_to_the_suite_directory() {
        let dir = std::env::temp_dir().join(format!("aa-suite-build-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("e2e")).unwrap();
        let path = dir.join("e2e/suite.yaml");
        std::fs::write(
            dir.join("e2e/test.yaml"),
            "name: t\nbackend: native\nsteps:\n  - action: app.open\n",
        )
        .unwrap();
        std::fs::write(&path, "version: 1\nenvironment: {emulator: medium_phone, apk: app.apk, package: test.app, appLabel: Test, build: {command: [./gradlew, assembleDebug], cwd: ..}}\ntests: [test.yaml]\n").unwrap();
        let (suite, _) = load(&path).unwrap();
        let build = suite.environment.build.unwrap();
        assert_eq!(build.command, ["./gradlew", "assembleDebug"]);
        assert_eq!(build.cwd, Some(dir.join("e2e/..")));
        std::fs::write(&path, "version: 1\nenvironment: {emulator: medium_phone, apk: app.apk, package: test.app, appLabel: Test, build: {command: []}}\ntests: [test.yaml]\n").unwrap();
        assert!(load(&path).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn inherited_paths_and_lists_use_the_declaring_file_and_explicit_replacement() {
        let dir = std::env::temp_dir().join(format!("aa-suite-inheritance-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("shared")).unwrap();
        std::fs::create_dir_all(dir.join("child")).unwrap();
        std::fs::write(dir.join("shared/base.yaml"), "version: 1\nartifactsDir: evidence\nenvironment: {emulator: test_phone, apk: app.apk, package: example.app, appLabel: Base, permissions: [android.permission.ACCESS_FINE_LOCATION], reversePorts: [{device: 10101, host: 19101}]}\nservices: [{name: backend, command: [node, start.mjs], cwd: '', readyPort: 56551}]\n").unwrap();
        std::fs::write(
            dir.join("shared/middle.yaml"),
            "extends: base.yaml\nenvironment: {appLabel: Middle}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("child/test.yaml"),
            "name: Child\nsteps: [{action: wait, duration: 1ms}]\n",
        )
        .unwrap();
        let path = dir.join("child/suite.yaml");
        std::fs::write(&path, "extends: ../shared/middle.yaml\nenvironment: {appLabel: Child, permissions: []}\ntests: [test.yaml]\n").unwrap();
        let (suite, tests) = load(&path).unwrap();
        assert_eq!(suite.environment.app_label, "Child");
        assert_eq!(suite.environment.apk, dir.join("child/../shared/app.apk"));
        assert!(suite.environment.permissions.is_empty());
        assert_eq!(suite.environment.reverse_ports[0].host, 19101);
        assert_eq!(suite.services[0].cwd, Some(dir.join("child/../shared")));
        assert_eq!(
            suite.artifacts_dir,
            Some(dir.join("child/../shared/evidence"))
        );
        assert_eq!(tests, vec![dir.join("child/test.yaml")]);
        std::fs::write(&path, "extends: ../shared/base.yaml\nenvironment: {apk: child.apk, reversePorts: []}\nservices: []\ntests: [test.yaml]\n").unwrap();
        let (suite, _) = load(&path).unwrap();
        assert_eq!(suite.environment.apk, dir.join("child/child.apk"));
        assert!(suite.services.is_empty());
        assert!(suite.environment.reverse_ports.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn suite_artifacts_are_relative_to_yaml_and_cli_override_is_exact() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../android-sample/e2e/suite.yaml");
        let (suite, _) = load(&path).unwrap();
        let base = path.parent().unwrap().join("../build/test-results");
        assert_eq!(suite.artifacts_dir, Some(base.clone()));
        assert_eq!(artifact_path(None, suite.artifacts_dir.clone()), base);
        assert_eq!(
            artifact_path(Some("custom".into()), suite.artifacts_dir),
            PathBuf::from("custom")
        );
        assert_eq!(
            artifact_path(None, None),
            PathBuf::from("build/test-results")
        );
    }
}

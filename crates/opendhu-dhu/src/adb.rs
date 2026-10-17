//! Minimal adb helpers: locate adb and list attached devices. ADB visibility alone does not
//! mean Android Auto projection is connected.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdbDevice {
    pub serial: String,
    pub state: String,
    pub model: Option<String>,
}

/// Parse `adb devices -l` output.
pub fn parse_devices(out: &str) -> Vec<AdbDevice> {
    out.lines()
        .skip_while(|l| !l.starts_with("List of devices"))
        .skip(1)
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let serial = it.next()?.to_string();
            let state = it.next()?.to_string();
            let model = it
                .find_map(|t| t.strip_prefix("model:"))
                .map(str::to_string);
            Some(AdbDevice {
                serial,
                state,
                model,
            })
        })
        .collect()
}

/// `ANDROID_HOME` and `ANDROID_SDK_ROOT`, when set.
pub fn sdk_roots_from_env() -> Vec<PathBuf> {
    ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect()
}

/// Find adb on PATH, then in `ANDROID_HOME`/`ANDROID_SDK_ROOT`.
pub fn find_adb(path_var: Option<&str>, sdk_roots: &[PathBuf]) -> Option<PathBuf> {
    let from_path = path_var
        .into_iter()
        .flat_map(|p| p.split(':'))
        .map(|d| Path::new(d).join("adb"));
    let from_sdk = sdk_roots.iter().map(|r| r.join("platform-tools/adb"));
    from_path.chain(from_sdk).find(|p| p.is_file())
}

pub fn list_devices(adb: &Path) -> std::io::Result<Vec<AdbDevice>> {
    let out = run_bounded(adb, &["devices", "-l"], Duration::from_secs(10)).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "`adb devices` did not answer in time",
        )
    })?;
    Ok(parse_devices(&out.stdout))
}

/// Captured result of a bounded child process.
#[derive(Debug)]
pub struct RunOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Run `program args` and give up after `limit`. adb can hang while a phone re-enumerates, so
/// nothing here may wait without a deadline. `None` means it could not start or timed out.
pub fn run_bounded(program: &Path, args: &[&str], limit: Duration) -> Option<RunOutput> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let end = Instant::now() + limit;
    loop {
        match child.try_wait().ok()? {
            Some(status) => {
                let out = child.wait_with_output().ok()?;
                return Some(RunOutput {
                    success: status.success(),
                    stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
                });
            }
            None if Instant::now() >= end => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

const ADB_CALL: Duration = Duration::from_secs(5);

/// Poll until adb reports `serial` as a ready device, or `limit` passes. After a DHU attaches,
/// the phone switches USB mode and is briefly missing from adb.
pub fn wait_for_device(adb: &Path, serial: &str, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        let state = run_bounded(adb, &["-s", serial, "get-state"], ADB_CALL);
        if state.is_some_and(|o| o.success && o.stdout.trim() == "device") {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// `adb -s SERIAL reverse tcp:PORT tcp:PORT`. Adds exactly this one tunnel.
pub fn reverse(adb: &Path, serial: &str, port: u16) -> Result<(), String> {
    let spec = format!("tcp:{port}");
    match run_bounded(adb, &["-s", serial, "reverse", &spec, &spec], ADB_CALL) {
        Some(o) if o.success => Ok(()),
        Some(o) => Err(o.stderr.trim().to_string()),
        None => Err("adb did not answer in time".into()),
    }
}

/// Ports that `adb reverse --list` shows as `tcp:P -> tcp:P` on the device. `None` if adb could
/// not be queried, which is different from an empty list.
pub fn reversed_ports(adb: &Path, serial: &str) -> Option<Vec<u16>> {
    let out = run_bounded(adb, &["-s", serial, "reverse", "--list"], ADB_CALL)?;
    out.success.then(|| parse_reverse_list(&out.stdout))
}

/// Parse `adb reverse --list` lines such as `UsbFfs tcp:10101 tcp:10101`. Only same-port tcp
/// tunnels are reported.
pub fn parse_reverse_list(out: &str) -> Vec<u16> {
    let port = |t: &str| t.strip_prefix("tcp:")?.parse::<u16>().ok();
    out.lines()
        .filter_map(|l| {
            let t: Vec<&str> = l.split_whitespace().collect();
            let [.., a, b] = t.as_slice() else {
                return None;
            };
            let (a, b) = (port(a)?, port(b)?);
            (a == b).then_some(a)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_adb_output() {
        let out = "List of devices attached\n1A141FDF600AF2         device usb:1114112X product:oriole_beta model:Pixel_6 device:oriole transport_id:23\n\n";
        assert_eq!(
            parse_devices(out),
            vec![AdbDevice {
                serial: "1A141FDF600AF2".into(),
                state: "device".into(),
                model: Some("Pixel_6".into())
            }]
        );
    }

    #[test]
    fn keeps_unauthorized_devices_visible_and_ignores_daemon_noise() {
        let out = "* daemon not running; starting now at tcp:5037\n* daemon started successfully\nList of devices attached\nABC123 unauthorized usb:1 transport_id:2\n";
        let d = parse_devices(out);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].state, "unauthorized");
        assert_eq!(d[0].model, None);
    }

    #[test]
    fn parses_reverse_list_formats() {
        assert_eq!(
            parse_reverse_list("UsbFfs tcp:10101 tcp:10101\nUsbFfs tcp:8080 tcp:8080\n"),
            vec![10101, 8080]
        );
        assert_eq!(parse_reverse_list("tcp:10101 tcp:10101\n"), vec![10101]);
        assert!(parse_reverse_list("\n").is_empty());
        assert!(parse_reverse_list("").is_empty());
        // different ports on each side and non-tcp tunnels are not "port N" tunnels
        assert!(
            parse_reverse_list("UsbFfs tcp:1 tcp:2\nUsbFfs localabstract:x tcp:3\n").is_empty()
        );
    }

    #[test]
    fn run_bounded_enforces_its_deadline() {
        let started = Instant::now();
        let out = run_bounded(Path::new("/bin/sleep"), &["5"], Duration::from_millis(200));
        assert!(out.is_none());
        assert!(started.elapsed() < Duration::from_secs(2));
        let ok = run_bounded(Path::new("/bin/echo"), &["hi"], Duration::from_secs(2)).unwrap();
        assert!(ok.success);
        assert_eq!(ok.stdout.trim(), "hi");
    }

    #[test]
    fn empty_list() {
        assert!(parse_devices("List of devices attached\n\n").is_empty());
    }
}

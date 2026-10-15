//! Minimal adb helpers: locate adb and list attached devices. ADB visibility alone does not
//! mean Android Auto projection is connected.

use std::path::{Path, PathBuf};
use std::process::Command;

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
    let out = Command::new(adb).args(["devices", "-l"]).output()?;
    Ok(parse_devices(&String::from_utf8_lossy(&out.stdout)))
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
    fn empty_list() {
        assert!(parse_devices("List of devices attached\n\n").is_empty());
    }
}

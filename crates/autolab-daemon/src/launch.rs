//! Turning a `StartRequest` into something launchable: find the DHU, choose the device.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use autolab_core::{Error, Result};
use autolab_dhu::{DhuBackend, DhuLaunch, adb, discovery};
use autolab_native::{NativeBackend, NativeLaunch};

use crate::api::{BackendKind, StartRequest};
use crate::backend::AnyBackend;

/// How long to wait for a phone that has just dropped off adb. After a DHU session ends the
/// phone leaves USB accessory mode and is briefly invisible to adb.
const DEVICE_WAIT: Duration = Duration::from_secs(10);

/// Start the head unit the request asks for. Returns it with the phone's adb serial, which the
/// session needs for tunnels and status.
pub fn launch_backend(req: &StartRequest) -> Result<(AnyBackend, String)> {
    if let Some(tcp) = &req.native_tcp {
        if req.backend != BackendKind::Native {
            return Err(Error::InvalidArgs(
                "`nativeTcp` belongs to the native backend".into(),
            ));
        }
        autolab_native::validate_tcp_endpoint(tcp)?;
    }
    match req.backend {
        BackendKind::Dhu => {
            let launch = resolve(req)?;
            let backend = DhuBackend::launch(&launch)?;
            Ok((AnyBackend::Dhu(backend), launch.serial))
        }
        BackendKind::Native => {
            let (launch, serial) = resolve_native(req)?;
            let backend = NativeBackend::launch(&launch)?;
            Ok((AnyBackend::Native(backend), serial))
        }
    }
}

/// Where the native head unit program is: `AUTOLAB_NATIVE_HU`, next to this executable, or on `PATH`.
pub fn find_native_hu(
    env: Option<PathBuf>,
    exe_dir: Option<PathBuf>,
    path: Option<&str>,
) -> std::result::Result<PathBuf, Vec<PathBuf>> {
    const NAME: &str = "autolab-native-hu";
    let mut tried = Vec::new();
    // A path the user named is a statement of intent: do not quietly use another one.
    if let Some(p) = env {
        return if p.is_file() { Ok(p) } else { Err(vec![p]) };
    }
    let mut candidates: Vec<PathBuf> = exe_dir.into_iter().map(|d| d.join(NAME)).collect();
    if let Some(path) = path {
        candidates.extend(std::env::split_paths(path).map(|d| d.join(NAME)));
    }
    for c in candidates {
        if c.is_file() {
            return Ok(c);
        }
        tried.push(c);
    }
    Err(tried)
}

fn resolve_native(req: &StartRequest) -> Result<(NativeLaunch, String)> {
    if req.dhu_path.is_some() || req.config.is_some() || req.headless {
        return Err(Error::InvalidArgs(
            "`dhuPath`, `config` and `headless` belong to the dhu backend; the native backend has no use for them"
                .into(),
        ));
    }
    let env = std::env::var_os("AUTOLAB_NATIVE_HU")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(PathBuf::from));
    let program = find_native_hu(env, exe_dir, std::env::var("PATH").ok().as_deref()).map_err(
        |tried| {
            let list: Vec<String> = tried.iter().map(|p| p.display().to_string()).collect();
            Error::BackendUnavailable(format!(
                "autolab-native-hu not found (build it with `cargo build -p autolab-native-hu`, or set AUTOLAB_NATIVE_HU). Tried: {}",
                list.join(", ")
            ))
        },
    )?;
    // The native head unit takes whichever Android phone is on USB, so a second phone would make
    // the choice ambiguous. Refuse instead of guessing.
    if req.native_tcp.is_none()
        && let Ok(adb) = adb_path()
        && let Ok(devices) = adb::list_devices(&adb)
        && devices.iter().filter(|d| d.state == "device").count() > 1
    {
        return Err(Error::InvalidArgs(
            "several phones are attached; the native backend cannot choose between them, so unplug the others"
                .into(),
        ));
    }
    let serial = match &req.serial {
        Some(s) if s.trim().is_empty() => {
            return Err(Error::InvalidArgs("`serial` is empty".into()));
        }
        Some(s) => s.clone(),
        None => pick_device()?,
    };
    Ok((
        NativeLaunch {
            program,
            serial: Some(serial.clone()),
            tcp: req.native_tcp.clone(),
        },
        serial,
    ))
}

pub fn resolve(req: &StartRequest) -> Result<DhuLaunch> {
    let env = discovery::Env::from_process(req.dhu_path.clone());
    let found = discovery::find_dhu(&env).map_err(|tried| {
        let list: Vec<String> = tried.iter().map(|p| p.display().to_string()).collect();
        Error::BackendUnavailable(format!(
            "DHU executable not found (install extras;google;auto or pass --dhu-path). Tried: {}",
            list.join(", ")
        ))
    })?;
    let serial = match &req.serial {
        Some(s) if s.trim().is_empty() => {
            return Err(Error::InvalidArgs("`serial` is empty".into()));
        }
        Some(s) => s.clone(),
        None => pick_device()?,
    };
    Ok(DhuLaunch {
        program: found.path,
        serial,
        config: req.config.clone(),
        headless: req.headless,
    })
}

/// Pick the only authorized device. `Ok(None)` means none yet; several is always an error.
pub fn select_device(devices: &[adb::AdbDevice]) -> Result<Option<String>> {
    let ready: Vec<_> = devices.iter().filter(|d| d.state == "device").collect();
    match ready.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(one.serial.clone())),
        many => Err(Error::InvalidArgs(format!(
            "multiple devices attached ({}); pass a serial",
            many.iter()
                .map(|d| d.serial.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

pub fn adb_path() -> Result<PathBuf> {
    adb::find_adb(
        std::env::var("PATH").ok().as_deref(),
        &adb::sdk_roots_from_env(),
    )
    .ok_or_else(|| {
        Error::BackendUnavailable("adb not found; pass a serial or install platform-tools".into())
    })
}

/// Choose the device only when there is exactly one authorized candidate, waiting a bounded time
/// for it to appear.
fn pick_device() -> Result<String> {
    let adb = adb_path()?;
    let deadline = Instant::now() + DEVICE_WAIT;
    loop {
        let devices = adb::list_devices(&adb)
            .map_err(|e| Error::BackendUnavailable(format!("adb failed: {e}")))?;
        if let Some(serial) = select_device(&devices)? {
            return Ok(serial);
        }
        if Instant::now() >= deadline {
            return Err(Error::DeviceDisconnected(format!(
                "no authorized adb device appeared within {}s",
                DEVICE_WAIT.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_hu_lookup_prefers_env_then_exe_dir_then_path_and_never_falls_back_from_env() {
        let dir = std::env::temp_dir().join(format!("al-nhu-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let hu = dir.join("autolab-native-hu");
        std::fs::write(&hu, "").unwrap();
        assert_eq!(
            find_native_hu(None, Some(dir.clone()), None).unwrap(),
            hu,
            "next to the executable"
        );
        let on_path = dir.to_str().unwrap().to_string();
        assert_eq!(find_native_hu(None, None, Some(&on_path)).unwrap(), hu);
        let missing = dir.join("nope");
        assert_eq!(
            find_native_hu(Some(missing.clone()), Some(dir.clone()), Some(&on_path)).unwrap_err(),
            vec![missing],
            "an explicit path that is missing is an error, not a fallback"
        );
        assert!(find_native_hu(None, None, None).unwrap_err().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn dev(serial: &str, state: &str) -> adb::AdbDevice {
        adb::AdbDevice {
            serial: serial.into(),
            state: state.into(),
            model: None,
        }
    }

    #[test]
    fn selects_only_a_single_authorized_device() {
        assert_eq!(select_device(&[]).unwrap(), None);
        assert_eq!(
            select_device(&[dev("A", "unauthorized")]).unwrap(),
            None,
            "unauthorized is never chosen"
        );
        assert_eq!(
            select_device(&[dev("A", "device"), dev("B", "offline")]).unwrap(),
            Some("A".into())
        );
    }

    #[test]
    fn several_authorized_devices_is_an_error_not_a_guess() {
        let err = select_device(&[dev("A", "device"), dev("B", "device")]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().contains("A, B"));
    }

    #[test]
    fn an_explicit_dhu_path_that_does_not_exist_is_an_error_listing_it() {
        let req = StartRequest {
            dhu_path: Some("/no/such/dhu".into()),
            serial: Some("S".into()),
            ..Default::default()
        };
        let err = resolve(&req).unwrap_err();
        assert_eq!(err.exit_code(), 3);
        assert!(err.to_string().contains("/no/such/dhu"));
    }

    #[test]
    fn tcp_is_rejected_for_dhu_before_discovery_or_device_access() {
        let req = StartRequest {
            native_tcp: Some("127.0.0.1:5277".into()),
            ..Default::default()
        };
        let err = launch_backend(&req).err().unwrap();
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().contains("native backend"));
    }

    #[test]
    fn non_loopback_tcp_is_rejected_before_discovery_or_device_access() {
        let req = StartRequest {
            backend: BackendKind::Native,
            native_tcp: Some("192.168.1.2:5277".into()),
            ..Default::default()
        };
        assert_eq!(launch_backend(&req).err().unwrap().exit_code(), 2);
    }

    #[test]
    fn an_empty_serial_is_rejected() {
        let req = StartRequest {
            serial: Some("  ".into()),
            dhu_path: Some("/bin/sh".into()),
            ..Default::default()
        };
        assert_eq!(resolve(&req).unwrap_err().exit_code(), 2);
    }
}

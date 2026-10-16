//! Turning a `StartRequest` into something launchable: find the DHU, choose the device.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use autolab_core::{Error, Result};
use autolab_dhu::{DhuLaunch, adb, discovery};

use crate::api::StartRequest;

/// How long to wait for a phone that has just dropped off adb. After a DHU session ends the
/// phone leaves USB accessory mode and is briefly invisible to adb.
const DEVICE_WAIT: Duration = Duration::from_secs(10);

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
    fn an_empty_serial_is_rejected() {
        let req = StartRequest {
            serial: Some("  ".into()),
            dhu_path: Some("/bin/sh".into()),
            ..Default::default()
        };
        assert_eq!(resolve(&req).unwrap_err().exit_code(), 2);
    }
}

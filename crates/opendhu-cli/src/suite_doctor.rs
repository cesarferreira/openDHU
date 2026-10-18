//! Read-only checks for the exact setup used by a managed suite.
use crate::doctor::{Check, Level, check};
use opendhu_core::Result;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn run(path: &Path, usb: bool) -> Result<Vec<Check>> {
    let (suite, tests) = crate::suite::load(path)?;
    let mut checks = vec![check(
        Level::Ok,
        "suite",
        format!("{} ({} test files)", path.display(), tests.len()),
    )];
    checks.push(check(
        if suite.environment.apk.is_file() {
            Level::Ok
        } else {
            Level::Fail
        },
        "apk",
        format!(
            "{}; build the app if missing",
            suite.environment.apk.display()
        ),
    ));
    let cwd = std::env::current_dir().map_err(|e| opendhu_core::Error::Other(e.to_string()))?;
    for (name, path) in [
        ("native head unit", crate::doctor::native_head_unit().ok()),
        ("ffmpeg", opendhu_native::find_ffmpeg()),
    ] {
        let usable = path.as_ref().is_some_and(|p| executable(p));
        checks.push(check(
            if usable { Level::Ok } else { Level::Fail },
            name,
            path.map(|p| {
                format!(
                    "{}{}",
                    p.display(),
                    if usable { "" } else { " is not executable" }
                )
            })
            .unwrap_or_else(|| format!("{name} not found; install it before running the suite")),
        ));
    }
    let host_path = std::env::var_os("PATH").unwrap_or_default();
    let adb = program("adb", &cwd, &host_path);
    checks.push(check(
        if adb.is_some() {
            Level::Ok
        } else {
            Level::Fail
        },
        "adb",
        adb.as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "not executable on PATH; add Android SDK platform-tools".into()),
    ));
    match program("tesseract", &cwd, &host_path) {
        Some(p) => match crate::device::command(&p, &["--list-langs"], Duration::from_secs(10)) {
            Ok(langs) if langs.lines().any(|l| l.trim() == "eng") => {
                checks.push(check(Level::Ok, "ocr", "tesseract with English data"))
            }
            Ok(_) => checks.push(check(
                Level::Fail,
                "ocr",
                "install tesseract English language data (eng)",
            )),
            Err(e) => checks.push(check(Level::Fail, "ocr", e.to_string())),
        },
        None => checks.push(check(
            Level::Fail,
            "ocr",
            "tesseract is not executable on PATH; install it with English data",
        )),
    }
    if usb {
        if let Some(adb) = adb {
            match crate::suite::find_usb_phone(&adb) {
                Ok(serial) => checks.push(check(Level::Ok, "usb phone", serial)),
                Err(e) => checks.push(check(Level::Fail, "usb phone", e.to_string())),
            }
        }
    } else {
        let emulator = crate::suite::emulator_program();
        match crate::device::command(&emulator, &["-list-avds"], Duration::from_secs(10)) {
            Ok(avds)
                if avds
                    .lines()
                    .any(|name| name.trim() == suite.environment.emulator) =>
            {
                checks.push(check(Level::Ok, "avd", &suite.environment.emulator))
            }
            Ok(_) => checks.push(check(
                Level::Fail,
                "avd",
                format!(
                    "{} is not listed by {}; create or provision it first",
                    suite.environment.emulator,
                    emulator.display()
                ),
            )),
            Err(e) => checks.push(check(
                Level::Fail,
                "avd",
                format!("{}: {e}", emulator.display()),
            )),
        }
    }
    for service in &suite.services {
        let cwd = service.cwd.as_deref().unwrap_or(&cwd);
        let service_path = service
            .env
            .get("PATH")
            .map(std::ffi::OsString::from)
            .unwrap_or_else(|| host_path.clone());
        let command = program(&service.command[0], cwd, &service_path);
        checks.push(check(
            if cwd.is_dir() && command.is_some() {
                Level::Ok
            } else {
                Level::Fail
            },
            &format!("service {} command", service.name),
            format!(
                "{} in {}; checks executable and directory only, without running the command",
                service.command[0],
                cwd.display()
            ),
        ));
        let port = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, service.ready_port));
        checks.push(check(
            if port.is_ok() { Level::Ok } else { Level::Fail },
            &format!("service {} port", service.name),
            format!(
                "127.0.0.1:{} {}",
                service.ready_port,
                if port.is_ok() {
                    "available now"
                } else {
                    "unavailable; stop the owning service or choose another port"
                }
            ),
        ));
    }
    checks.push(check(Level::Info, "not checked", "Android Auto provisioning, head-unit credentials, app permissions, service health and projection require a real run; no services or devices were started"));
    Ok(checks)
}

fn program(name: &str, cwd: &Path, path: &std::ffi::OsStr) -> Option<PathBuf> {
    if Path::new(name).components().count() > 1 || Path::new(name).is_absolute() {
        let p = cwd.join(name);
        return executable(&p).then_some(p);
    }
    std::env::split_paths(path)
        .map(|dir| cwd.join(dir).join(name))
        .find(|p| executable(p))
}

fn executable(path: &Path) -> bool {
    path.is_file()
        && path
            .metadata()
            .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

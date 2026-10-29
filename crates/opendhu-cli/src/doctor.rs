//! `aa doctor`: read-only environment checks. It never starts a DHU or touches the phone,
//! so it is safe to run while a session is active.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use opendhu_dhu::{adb, console, discovery};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Warn,
    Fail,
    Info,
}

#[derive(Debug, Serialize)]
pub struct Check {
    pub level: Level,
    pub name: String,
    pub detail: String,
}

pub(crate) fn check(level: Level, name: &str, detail: impl Into<String>) -> Check {
    Check {
        level,
        name: name.into(),
        detail: detail.into(),
    }
}

/// Run `program args` with a hard deadline and return its stdout+stderr.
fn run_bounded(program: &Path, args: &[&str], limit: Duration) -> Option<String> {
    adb::run_bounded(program, args, limit).map(|o| format!("{}{}", o.stdout, o.stderr))
}

/// Parse `pgrep` output (one pid per line).
fn parse_pids(out: &str) -> Vec<u32> {
    out.lines().filter_map(|l| l.trim().parse().ok()).collect()
}

/// DHU processes other than the one openDHU's own session owns.
fn foreign_dhu_pids(running: &[u32], own: Option<u32>) -> Vec<u32> {
    running
        .iter()
        .copied()
        .filter(|p| Some(*p) != own)
        .collect()
}

pub fn config_error(e: &opendhu_core::Error) -> Check {
    check(Level::Fail, "config file", e.to_string())
}

pub fn run(explicit_dhu: Option<PathBuf>, config_file: Option<&Path>) -> Vec<Check> {
    let mut out = Vec::new();
    out.push(match config_file {
        Some(p) => check(Level::Ok, "config file", p.display().to_string()),
        None => check(
            Level::Info,
            "config file",
            "none loaded (optional; see docs/configuration.md)",
        ),
    });

    native_checks(&mut out);
    let env = discovery::Env::from_process(explicit_dhu);
    match discovery::find_dhu(&env) {
        Ok(found) => {
            out.push(check(
                Level::Ok,
                "dhu executable",
                format!("{} ({:?})", found.path.display(), found.source),
            ));
            match run_bounded(&found.path, &["--version"], Duration::from_secs(5)) {
                Some(text) => {
                    let info = console::parse_info(&console::clean_lines(&text));
                    out.push(check(
                        Level::Ok,
                        "dhu launches",
                        format!(
                            "version {} build {} (checked with --version; no device touched)",
                            info.version.as_deref().unwrap_or("unknown"),
                            info.build.as_deref().unwrap_or("unknown")
                        ),
                    ));
                }
                None => out.push(check(
                    Level::Fail,
                    "dhu launches",
                    "`--version` did not finish within 5s",
                )),
            }
        }
        Err(tried) => {
            let list: Vec<String> = tried.iter().map(|p| p.display().to_string()).collect();
            out.push(check(
                Level::Fail,
                "dhu executable",
                format!("not found. Install it via the Android SDK Manager (extras;google;auto) or pass --dhu-path. Tried: {}", list.join(", ")),
            ));
        }
    }

    let sdk_roots = adb::sdk_roots_from_env();
    let path_var = std::env::var("PATH").ok();
    match adb::find_adb(path_var.as_deref(), &sdk_roots) {
        None => out.push(check(
            Level::Fail,
            "adb",
            "not found on PATH or in ANDROID_HOME/platform-tools",
        )),
        Some(adb_path) => {
            out.push(check(Level::Ok, "adb", adb_path.display().to_string()));
            match adb::list_devices(&adb_path) {
                Err(e) => out.push(check(Level::Fail, "adb devices", e.to_string())),
                Ok(devs) if devs.is_empty() => {
                    out.push(check(Level::Fail, "adb devices", "no device attached"))
                }
                Ok(devs) => {
                    for d in devs {
                        if d.state == "device" {
                            out.push(crate::gearhead::check_device(
                                &adb_path,
                                &d.serial,
                                Level::Warn,
                            ));
                        }
                        let level = if d.state == "device" {
                            Level::Ok
                        } else {
                            Level::Fail
                        };
                        out.push(check(
                            level,
                            "adb device",
                            format!(
                                "{} {} ({})",
                                d.serial,
                                d.state,
                                d.model.as_deref().unwrap_or("unknown model")
                            ),
                        ));
                    }
                }
            }
        }
    }

    // Only managed test suites (`aa test`) need these, so they warn instead of failing.
    let cwd = std::env::current_dir().unwrap_or_default();
    let host_path = std::env::var_os("PATH").unwrap_or_default();
    let ocr = crate::suite_doctor::ocr_check(&cwd, &host_path);
    out.push(check(
        if ocr.level == Level::Fail {
            Level::Warn
        } else {
            ocr.level
        },
        "ocr (aa test)",
        ocr.detail,
    ));
    let emulator = crate::suite::emulator_program();
    let emulator_found = emulator.is_absolute() && emulator.is_file();
    out.push(check(
        if emulator_found {
            Level::Ok
        } else {
            Level::Warn
        },
        "emulator (aa test)",
        if emulator_found {
            emulator.display().to_string()
        } else {
            "not found; managed suites boot an AVD from the Android SDK emulator".into()
        },
    ));

    let session = opendhu_daemon::Client::connect()
        .and_then(|c| c.status())
        .ok()
        .and_then(|st| st.session)
        .and_then(|s| s.details);
    match &session {
        Some(d) => out.push(check(
            Level::Info,
            "session",
            format!(
                "running (dhu pid {}, serial {})",
                d.dhu_pid,
                d.head_unit.serial.as_deref().unwrap_or("unknown")
            ),
        )),
        None => out.push(check(Level::Info, "session", "none running")),
    }
    let running = Command::new("pgrep")
        .args(["-x", "desktop-head-unit"])
        .output()
        .map(|o| parse_pids(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    let foreign = foreign_dhu_pids(&running, session.as_ref().map(|d| d.dhu_pid));
    if !foreign.is_empty() {
        let pids: Vec<String> = foreign.iter().map(u32::to_string).collect();
        out.push(check(
            Level::Warn,
            "other dhu",
            format!(
                "a desktop-head-unit not started by openDHU is running (pid {}). It holds the phone's USB link and openDHU cannot control it; stop it before `aa session start`",
                pids.join(", ")
            ),
        ));
    }
    out.push(check(
        Level::Info,
        "not checked",
        "whether Android Auto projection will connect; that needs `aa session start` and a phone with Android Auto set up",
    ));
    out
}

pub fn has_failures(checks: &[Check]) -> bool {
    checks.iter().any(|c| c.level == Level::Fail)
}

pub(crate) fn native_head_unit() -> std::result::Result<PathBuf, Vec<PathBuf>> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(PathBuf::from));
    let env = std::env::var_os("OPENDHU_NATIVE_HU")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    opendhu_daemon::find_native_hu(env, exe_dir, std::env::var("PATH").ok().as_deref())
}

/// The native backend is optional, so a missing piece is information, not a failure.
fn native_checks(out: &mut Vec<Check>) {
    let hu = native_head_unit();
    out.push(match hu {
        Ok(p) => check(Level::Ok, "native head unit", p.display().to_string()),
        Err(_) => check(
            Level::Info,
            "native head unit",
            "opendhu-native-hu not found (needed for the default `aa session start`; use `aa session start --dhu` for Google's DHU)",
        ),
    });
    out.push(
        match opendhu_native::find_ffmpeg().map(|p| p.with_file_name("ffplay")) {
            Some(p) if p.is_file() => check(Level::Ok, "ffplay", p.display().to_string()),
            _ => check(
                Level::Info,
                "ffplay",
                "not found; the window plays the phone's audio with it (ships with ffmpeg)",
            ),
        },
    );
    out.push(match opendhu_native::find_ffmpeg() {
        Some(p) => check(Level::Ok, "ffmpeg", p.display().to_string()),
        None => check(
            Level::Info,
            "ffmpeg",
            "not found; the native backend needs it for screenshots (macOS: brew install ffmpeg)",
        ),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pgrep_output() {
        assert_eq!(parse_pids("35180\n35199\n"), vec![35180, 35199]);
        assert!(parse_pids("").is_empty());
    }

    #[test]
    fn own_dhu_is_not_foreign() {
        assert_eq!(foreign_dhu_pids(&[10, 20], Some(10)), vec![20]);
        assert_eq!(foreign_dhu_pids(&[10], Some(10)), Vec::<u32>::new());
        assert_eq!(foreign_dhu_pids(&[10], None), vec![10]);
    }
}

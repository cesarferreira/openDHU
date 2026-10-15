mod doctor;
mod output_path;
mod session;
mod session_backend;
mod test_cmd;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use autolab_core::{
    Error, FocusState, OpStatus, OperationResult, Result, ScreenshotResult, SessionState,
};
use autolab_dhu::{DhuLaunch, adb, discovery};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(
    name = "autolab",
    version,
    about = "Programmable Android Auto head unit control"
)]
struct Cli {
    /// Print machine-readable JSON instead of text
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Start a DHU session (launches Google's DHU over USB and keeps it under AutoLab's control)
    Start {
        /// Path to the desktop-head-unit executable [env: AUTOLAB_DHU_PATH]
        #[arg(long)]
        dhu_path: Option<PathBuf>,
        /// Android device serial (default: the only authorized adb device)
        #[arg(long)]
        serial: Option<String>,
        /// DHU .ini configuration file
        #[arg(long)]
        config: Option<PathBuf>,
        /// Run the DHU without its window
        #[arg(long)]
        headless: bool,
        /// Re-apply `adb reverse tcp:PORT tcp:PORT` after the phone attaches (repeatable). Starting
        /// a session re-enumerates the phone over USB, which drops existing tunnels.
        #[arg(long = "reverse", value_name = "PORT", value_parser = clap::value_parser!(u16).range(1..))]
        reverse: Vec<u16>,
    },
    /// Stop the running session
    Stop,
    /// Show the running session
    Status,
    /// Change head unit focus
    Focus {
        #[command(subcommand)]
        kind: FocusKind,
    },
    /// Save a PNG of the projected display
    Screenshot {
        /// Output file, must end in .png
        #[arg(conflicts_with = "output")]
        path: Option<PathBuf>,
        /// Output file, must end in .png (same as the positional argument)
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
    },
    /// Run YAML tests against the running session
    Test {
        /// Test files or directories (searched recursively for .yaml/.yml)
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Report to write into the artifacts directory (repeatable; default: json)
        #[arg(long, value_enum)]
        report: Vec<test_cmd::ReportFormat>,
        /// Where reports, logs and screenshots go [default: ./test-results]
        #[arg(long)]
        artifacts_dir: Option<PathBuf>,
        /// Skip, instead of fail, tests whose required capabilities the backend lacks
        #[arg(long)]
        skip_unsupported: bool,
    },
    /// Check the environment without touching a running session
    Doctor {
        #[arg(long)]
        dhu_path: Option<PathBuf>,
    },
    #[command(name = "__supervise", hide = true)]
    Supervise {
        #[arg(long)]
        dhu_path: PathBuf,
        #[arg(long)]
        serial: String,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        headless: bool,
        #[arg(long = "reverse")]
        reverse: Vec<u16>,
    },
}

#[derive(Subcommand, Debug)]
enum FocusKind {
    /// Take or release video focus
    Video { state: OnOff },
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum OnOff {
    On,
    Off,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let supervising = matches!(cli.command, Command::Supervise { .. });
    match run(cli) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            // The supervisor's stderr is read back by `start`, which re-wraps the message.
            if supervising {
                eprintln!("{}", e.message());
            } else {
                eprintln!("error: {e}");
            }
            ExitCode::from(e.exit_code())
        }
    }
}

fn run(cli: Cli) -> Result<u8> {
    let json = cli.json;
    match cli.command {
        Command::Start {
            dhu_path,
            serial,
            config,
            headless,
            reverse,
        } => {
            let launch = resolve_launch(dhu_path, serial, config, headless)?;
            let s = session::start(&launch, &reverse)?;
            print_status(&s, json);
            Ok(0)
        }
        Command::Stop => {
            session::stop()?;
            println!(
                "{}",
                if json {
                    "{\"stopped\":true}"
                } else {
                    "Session stopped"
                }
            );
            Ok(0)
        }
        Command::Status => {
            print_status(&session::status()?, json);
            Ok(0)
        }
        Command::Focus {
            kind: FocusKind::Video { state },
        } => {
            let r = session::set_video_focus(matches!(state, OnOff::On))?;
            print_focus(&r, json);
            Ok(status_exit_code(r.status))
        }
        Command::Screenshot { path, output } => {
            let raw = path.or(output).ok_or_else(|| {
                Error::InvalidArgs("give an output path: `autolab screenshot out.png`".into())
            })?;
            let cwd = std::env::current_dir()
                .map_err(|e| Error::Other(format!("cannot read current directory: {e}")))?;
            let abs = output_path::resolve_png(&raw, &cwd)?;
            print_screenshot(&session::screenshot(&abs)?, json);
            Ok(0)
        }
        Command::Test {
            paths,
            report,
            artifacts_dir,
            skip_unsupported,
        } => test_cmd::run(test_cmd::Options {
            paths,
            report,
            artifacts_dir,
            skip_unsupported,
            json,
        }),
        Command::Doctor { dhu_path } => {
            let checks = doctor::run(dhu_path);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&checks).expect("serializes")
                );
            } else {
                for c in &checks {
                    let tag = match c.level {
                        doctor::Level::Ok => "ok  ",
                        doctor::Level::Warn => "warn",
                        doctor::Level::Fail => "FAIL",
                        doctor::Level::Info => "info",
                    };
                    println!("{tag}  {:<15} {}", c.name, c.detail);
                }
            }
            Ok(if doctor::has_failures(&checks) { 3 } else { 0 })
        }
        Command::Supervise {
            dhu_path,
            serial,
            config,
            headless,
            reverse,
        } => {
            session::supervise(
                DhuLaunch {
                    program: dhu_path,
                    serial,
                    config,
                    headless,
                },
                reverse,
            )?;
            Ok(0)
        }
    }
}

fn status_exit_code(s: OpStatus) -> u8 {
    match s {
        OpStatus::Accepted | OpStatus::Acknowledged | OpStatus::Observed => 0,
        OpStatus::Failed => 1,
        OpStatus::Unsupported => 5,
        OpStatus::TimedOut => 6,
    }
}

fn resolve_launch(
    dhu_path: Option<PathBuf>,
    serial: Option<String>,
    config: Option<PathBuf>,
    headless: bool,
) -> Result<DhuLaunch> {
    let env = discovery::Env::from_process(dhu_path);
    let found = discovery::find_dhu(&env).map_err(|tried| {
        let list: Vec<String> = tried.iter().map(|p| p.display().to_string()).collect();
        Error::BackendUnavailable(format!(
            "DHU executable not found (install extras;google;auto or pass --dhu-path). Tried: {}",
            list.join(", ")
        ))
    })?;
    let serial = match serial {
        Some(s) => s,
        None => pick_device()?,
    };
    Ok(DhuLaunch {
        program: found.path,
        serial,
        config,
        headless,
    })
}

/// How long to wait for a phone that has just dropped off adb. After a DHU session ends the
/// phone leaves USB accessory mode and is briefly invisible to adb.
const DEVICE_WAIT: Duration = Duration::from_secs(10);

/// Pick the only authorized device. `Ok(None)` means none yet; several is always an error.
fn select_device(devices: &[adb::AdbDevice]) -> Result<Option<String>> {
    let ready: Vec<_> = devices.iter().filter(|d| d.state == "device").collect();
    match ready.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(one.serial.clone())),
        many => Err(Error::InvalidArgs(format!(
            "multiple devices attached ({}); pass --serial",
            many.iter()
                .map(|d| d.serial.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// Choose the device only when there is exactly one authorized candidate.
fn pick_device() -> Result<String> {
    let sdk_roots = adb::sdk_roots_from_env();
    let adb_path =
        adb::find_adb(std::env::var("PATH").ok().as_deref(), &sdk_roots).ok_or_else(|| {
            Error::BackendUnavailable(
                "adb not found; pass --serial or install platform-tools".into(),
            )
        })?;
    let deadline = Instant::now() + DEVICE_WAIT;
    loop {
        let devices = adb::list_devices(&adb_path)
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

fn focus_word(s: FocusState) -> &'static str {
    match s {
        FocusState::On => "on",
        FocusState::Off => "off",
        FocusState::Unknown => "unknown",
    }
}

fn print_focus(r: &OperationResult, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(r).expect("serializes"));
        return;
    }
    match r.status {
        OpStatus::Accepted => {
            println!("Video focus {} command accepted", focus_word(r.requested));
            println!(
                "  DHU reports:     {} (DHU console readback, not a phone acknowledgement)",
                focus_word(r.backend_reported)
            );
            println!("  Phone confirmed: {}", focus_word(r.observed));
        }
        other => {
            eprintln!(
                "Video focus {} command {:?}",
                focus_word(r.requested),
                other
            );
            for l in &r.evidence {
                eprintln!("  {l}");
            }
        }
    }
}

fn print_screenshot(r: &ScreenshotResult, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(r).expect("serializes"));
    } else {
        println!(
            "Screenshot saved: {} ({}x{})",
            r.path.display(),
            r.width,
            r.height
        );
    }
}

fn print_status(s: &session::StatusInfo, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(s).expect("serializes"));
        return;
    }
    let h = &s.head_unit;
    let v = &h.video_focus;
    let serial = h.serial.as_deref().unwrap_or("unknown");
    println!("AutoLab {}\n", env!("CARGO_PKG_VERSION"));
    println!(
        "Backend:      {} ({}, build {})",
        h.backend,
        h.backend_version.as_deref().unwrap_or("unknown version"),
        h.backend_build.as_deref().unwrap_or("unknown")
    );
    let state = match h.state {
        SessionState::Running => "running",
    };
    println!("State:        {state}");
    println!("Transport:    {} (serial {serial})", s.transport);
    println!(
        "Video focus:  requested {}; DHU reports {}; phone confirmed {}",
        focus_word(v.requested),
        focus_word(v.backend_reported),
        focus_word(v.observed)
    );
    for t in &s.tunnels {
        let again = match t.reapplied {
            0 => String::new(),
            n => format!(", re-applied {n}x"),
        };
        println!(
            "Tunnel:       adb reverse tcp:{} ({:?}{again})",
            t.port, t.state
        );
    }
    println!(
        "Session:      supervisor pid {}, dhu pid {}",
        s.supervisor_pid, s.dhu_pid
    );
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
            "unauthorized devices are never chosen"
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
}

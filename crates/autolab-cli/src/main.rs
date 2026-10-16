mod audio_cmd;
mod config;
mod daemon_ctl;
mod doctor;
mod output_path;
mod session_backend;
mod test_cmd;

use std::path::PathBuf;
use std::process::ExitCode;

use autolab_core::{Error, FocusState, OpStatus, Result, SessionState};
use autolab_daemon::api::{
    BackendKind, InputResponse, OperationResponse, ScreenshotResponse, SessionStatus, StartRequest,
};
use autolab_daemon::{ServeOptions, runtime};
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
    /// Start a DHU session (launches Google's DHU over USB; the AutoLab daemon keeps it running)
    Start {
        /// Head unit to run: Google's DHU, or AutoLab's own native one [default: dhu]
        #[arg(long, value_enum)]
        backend: Option<BackendArg>,
        /// Path to the desktop-head-unit executable (dhu backend) [env: AUTOLAB_DHU_PATH]
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
    /// Record what the phone plays through the car to a WAV file (native backend)
    ///
    /// The phone sends nothing while a channel is silent, so the file holds only the sound. The summary
    /// says how much arrived and how loud it was, which is also how to check audio without listening.
    Audio {
        /// The WAV file to write
        out: PathBuf,
        /// media (music, 48 kHz stereo), system (prompts, 16 kHz mono) or speech (assistant, 16 kHz mono)
        #[arg(long, default_value = "media", value_parser = parse_channel)]
        channel: autolab_core::AudioChannel,
        /// How long to listen, in seconds
        #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..=3600))]
        seconds: u64,
    },
    /// Touch a pixel of the projected screen (0,0 is the top left of a screenshot)
    Tap { x: u32, y: u32 },
    /// Press one of the car's buttons: home, back, up, down, left, right, enter, search, play_pause,
    /// next, previous, call, end_call
    Key {
        #[arg(value_parser = parse_key)]
        key: autolab_core::Key,
        /// Hold it for about a second, as a long press
        #[arg(long)]
        long: bool,
    },
    /// Drag from one pixel to another
    Swipe {
        x1: u32,
        y1: u32,
        x2: u32,
        y2: u32,
        /// How long the drag takes, 50 to 10000
        #[arg(long, default_value_t = 300)]
        duration_ms: u64,
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
    /// Run the AutoLab daemon in the foreground
    ///
    /// It serves the HTTP API on 127.0.0.1 until stopped with Ctrl+C or SIGTERM, which also stops the
    /// session. `autolab start` starts a daemon by itself when none is running; this one stays up.
    Daemon {
        /// Port to listen on [default: 8765, or any free port if that is taken]
        #[arg(long)]
        port: Option<u16>,
        /// Exit when the session ends. `autolab start` sets this for the daemon it launches.
        #[arg(long, hide = true)]
        exit_when_idle: bool,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendArg {
    Dhu,
    Native,
}

impl From<BackendArg> for BackendKind {
    fn from(b: BackendArg) -> Self {
        match b {
            BackendArg::Dhu => Self::Dhu,
            BackendArg::Native => Self::Native,
        }
    }
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
    match run(Cli::parse()) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(e.exit_code())
        }
    }
}

fn run(cli: Cli) -> Result<u8> {
    let json = cli.json;
    // Only commands that use settings read the file, so a typo in it never blocks `stop` or `status`.
    let load = || config::load(&config::Locator::from_process(), &current_dir()?);
    let dhu_env_set = std::env::var_os("AUTOLAB_DHU_PATH").is_some_and(|v| !v.is_empty());
    match cli.command {
        Command::Start {
            backend,
            dhu_path,
            serial,
            config,
            headless,
            reverse,
        } => {
            let cfg = load()?;
            let cwd = current_dir()?;
            // The daemon's working directory is not ours, so paths are resolved here.
            let backend = config::first(backend, cfg.backend).unwrap_or(BackendArg::Dhu);
            // `dhu.*` settings in the config file are for the DHU only: they never reach another backend.
            // A flag given on the command line still does, and the daemon then refuses it.
            let from_file = backend == BackendArg::Dhu;
            let req = StartRequest {
                backend: backend.into(),
                serial: config::first(serial, cfg.serial.clone()),
                dhu_path: config::dhu_path(
                    dhu_path.map(|p| output_path::absolute(&p, &cwd)),
                    cfg.dhu_path.clone().filter(|_| from_file),
                    dhu_env_set || !from_file,
                ),
                config: config::first(
                    config.map(|p| output_path::absolute(&p, &cwd)),
                    cfg.dhu_ini.clone().filter(|_| from_file),
                ),
                headless: headless || (from_file && cfg.headless),
                reverse: if reverse.is_empty() {
                    cfg.reverse.clone()
                } else {
                    reverse
                },
            };
            let status = daemon_ctl::ensure()?.start(&req)?;
            print_session(&expect_session(status.session)?, json);
            Ok(0)
        }
        Command::Stop => {
            daemon_ctl::connect()?.stop()?;
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
            let status = daemon_ctl::connect()?.status()?;
            print_session(&expect_session(status.session)?, json);
            Ok(0)
        }
        Command::Focus {
            kind: FocusKind::Video { state },
        } => {
            let r = daemon_ctl::connect()?.focus_video(matches!(state, OnOff::On))?;
            print_focus(&r, json);
            Ok(status_exit_code(r.result.status))
        }
        Command::Screenshot { path, output } => {
            let raw = path.or(output).ok_or_else(|| {
                Error::InvalidArgs("give an output path: `autolab screenshot out.png`".into())
            })?;
            let abs = output_path::resolve_png(&raw, &current_dir()?)?;
            print_screenshot(&daemon_ctl::connect()?.screenshot(&abs)?, json);
            Ok(0)
        }
        Command::Audio {
            out,
            channel,
            seconds,
        } => {
            let out = output_path::absolute(&out, &current_dir()?);
            let s = audio_cmd::record(channel, seconds, &out)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&s).expect("serializes"));
            } else if s.bytes == 0 {
                println!(
                    "No {} audio arrived in {:.0} s: the phone was silent on that channel. Wrote {}",
                    channel, s.wall_seconds, s.path
                );
            } else {
                println!(
                    "Recorded {:.1} s of {} audio in {:.0} s ({} bytes, peak {:.2}, rms {:.3}): {}",
                    s.audio_seconds, channel, s.wall_seconds, s.bytes, s.peak, s.rms, s.path
                );
            }
            Ok(0)
        }
        Command::Tap { x, y } => {
            let r = daemon_ctl::connect()?.tap(x, y)?;
            print_input(&r, json);
            Ok(status_exit_code(r.result.status))
        }
        Command::Key { key, long } => {
            let r = daemon_ctl::connect()?.key(key, long)?;
            print_input(&r, json);
            Ok(status_exit_code(r.result.status))
        }
        Command::Swipe {
            x1,
            y1,
            x2,
            y2,
            duration_ms,
        } => {
            let r = daemon_ctl::connect()?.swipe(
                (x1, y1),
                (x2, y2),
                std::time::Duration::from_millis(duration_ms),
            )?;
            print_input(&r, json);
            Ok(status_exit_code(r.result.status))
        }
        Command::Test {
            paths,
            report,
            artifacts_dir,
            skip_unsupported,
        } => {
            let cfg = load()?;
            test_cmd::run(test_cmd::Options {
                paths,
                report: if report.is_empty() {
                    cfg.report.clone()
                } else {
                    report
                },
                artifacts_dir: config::first(artifacts_dir, cfg.artifacts_dir.clone()),
                skip_unsupported: skip_unsupported || cfg.skip_unsupported,
                json,
            })
        }
        Command::Doctor { dhu_path } => {
            let checks = match load() {
                Ok(cfg) => {
                    let dhu = config::dhu_path(dhu_path, cfg.dhu_path.clone(), dhu_env_set);
                    doctor::run(dhu, cfg.source.as_deref())
                }
                Err(e) => {
                    let mut checks = vec![doctor::config_error(&e)];
                    checks.extend(doctor::run(dhu_path, None).into_iter().skip(1));
                    checks
                }
            };
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
        Command::Daemon {
            port,
            exit_when_idle,
        } => {
            let cfg = load()?;
            autolab_daemon::run(ServeOptions {
                runtime_dir: runtime::runtime_dir()?,
                port: config::first(port, cfg.port),
                exit_when_idle,
                idle_grace: None,
            })?;
            Ok(0)
        }
    }
}

fn parse_channel(s: &str) -> std::result::Result<autolab_core::AudioChannel, String> {
    s.parse()
}

fn parse_key(s: &str) -> std::result::Result<autolab_core::Key, String> {
    s.parse()
}

fn current_dir() -> Result<PathBuf> {
    std::env::current_dir().map_err(|e| Error::Other(format!("cannot read current directory: {e}")))
}

/// `None` means the daemon is up but has no session. That is the same "no session" condition
/// (exit code 3) as no daemon at all.
fn expect_session(s: Option<SessionStatus>) -> Result<SessionStatus> {
    s.ok_or_else(|| {
        Error::BackendUnavailable(
            "no AutoLab session is running; start one with `autolab start`".into(),
        )
    })
}

fn status_exit_code(s: OpStatus) -> u8 {
    match s {
        OpStatus::Accepted | OpStatus::Acknowledged | OpStatus::Observed => 0,
        OpStatus::Failed => 1,
        OpStatus::Unsupported => 5,
        OpStatus::TimedOut => 6,
    }
}

fn focus_word(s: FocusState) -> &'static str {
    match s {
        FocusState::On => "on",
        FocusState::Off => "off",
        FocusState::Unknown => "unknown",
    }
}

fn print_focus(resp: &OperationResponse, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(resp).expect("serializes")
        );
        return;
    }
    let r = &resp.result;
    match r.status {
        OpStatus::Accepted | OpStatus::Acknowledged | OpStatus::Observed => {
            let verdict = if r.status == OpStatus::Observed {
                "confirmed by the phone"
            } else {
                "command accepted"
            };
            println!("Video focus {} {verdict}", focus_word(r.requested));
            match resp.backend {
                BackendKind::Dhu => println!(
                    "  DHU reports:     {} (DHU console readback, not a phone acknowledgement)",
                    focus_word(r.backend_reported)
                ),
                BackendKind::Native => println!(
                    "  Head unit sent:  {} (our own message, not a phone acknowledgement)",
                    focus_word(r.backend_reported)
                ),
            }
            println!("  Phone confirmed: {}", focus_word(r.observed));
            if r.status == OpStatus::Observed {
                for l in &r.evidence {
                    println!("  {l}");
                }
            }
        }
        other => {
            eprintln!(
                "Video focus {} command {}",
                focus_word(r.requested),
                match other {
                    OpStatus::TimedOut => "not confirmed by the phone in time".to_string(),
                    o => format!("{o:?}"),
                }
            );
            for l in &r.evidence {
                eprintln!("  {l}");
            }
        }
    }
}

fn print_input(resp: &InputResponse, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(resp).expect("serializes")
        );
        return;
    }
    println!("Sent (the phone does not acknowledge touch or buttons)");
    for l in &resp.result.evidence {
        println!("  {l}");
    }
}

fn print_screenshot(resp: &ScreenshotResponse, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(resp).expect("serializes")
        );
    } else {
        let r = &resp.result;
        println!(
            "Screenshot saved: {} ({}x{})",
            r.path.display(),
            r.width,
            r.height
        );
    }
}

fn print_session(s: &SessionStatus, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(s).expect("serializes"));
        return;
    }
    println!("AutoLab {}\n", env!("CARGO_PKG_VERSION"));
    let Some(d) = &s.details else {
        println!("State:        {:?}", s.phase);
        println!("Session:      {}", s.session_id);
        return;
    };
    let h = &d.head_unit;
    let v = &h.video_focus;
    let serial = h.serial.as_deref().unwrap_or("unknown");
    println!(
        "Backend:      {} ({}, build {})",
        h.backend,
        h.backend_version.as_deref().unwrap_or("unknown version"),
        h.backend_build.as_deref().unwrap_or("unknown")
    );
    if h.backend == "native" {
        println!(
            "Certificate:  {}",
            h.certificate_source.as_deref().unwrap_or("unknown")
        );
    }
    let state = match h.state {
        SessionState::Running => "running",
    };
    println!("State:        {state}");
    println!("Transport:    {} (serial {serial})", d.transport);
    println!(
        "Video focus:  requested {}; {} {}; phone confirmed {}",
        focus_word(v.requested),
        if h.backend == "native" {
            "head unit sent"
        } else {
            "DHU reports"
        },
        focus_word(v.backend_reported),
        focus_word(v.observed)
    );
    for t in &d.tunnels {
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
        "Session:      {} (daemon pid {}, dhu pid {})",
        s.session_id, d.daemon_pid, d.dhu_pid
    );
}

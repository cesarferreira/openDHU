//! `aa test`: parse every file first, then run them through the session backend.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use autolab_core::{Error, HeadUnitBackend, Result};
use autolab_test::report::{
    BackendMeta, Format, Metadata, RunReport, Summary, exit_code_for, iso8601_utc, step_log,
    write_artifacts,
};
use autolab_test::runner::{
    Clock, INTERRUPTED, Progress, RunConfig, SystemClock, run_test_with_progress,
};
use autolab_test::{Outcome, TestFile};
use clap::ValueEnum;

use crate::daemon_ctl;
use crate::session_backend::DaemonBackend;

#[derive(ValueEnum, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReportFormat {
    Json,
    Junit,
}

pub struct Options {
    pub paths: Vec<PathBuf>,
    pub filter: Option<String>,
    pub output: Arc<crate::test_output::Output>,
    pub validate: bool,
    pub show_head_unit: bool,
    pub cancel: Option<Arc<AtomicBool>>,
    pub app: Option<crate::suite::Application>,
    pub runtime_dir: Option<PathBuf>,
    pub report: Vec<ReportFormat>,
    pub artifacts_dir: Option<PathBuf>,
    pub skip_unsupported: bool,
    pub json: bool,
}

/// Expand files and directories (recursively, sorted) into test files.
pub(crate) fn collect(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, out)?;
            } else if is_yaml(&p) {
                out.push(p);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    for p in paths {
        if p.is_dir() {
            let before = files.len();
            walk(p, &mut files)
                .map_err(|e| Error::InvalidArgs(format!("cannot read {}: {e}", p.display())))?;
            if files.len() == before {
                return Err(Error::InvalidArgs(format!(
                    "no .yaml or .yml test files in {}",
                    p.display()
                )));
            }
        } else if p.is_file() {
            files.push(p.clone());
        } else {
            return Err(Error::InvalidArgs(format!(
                "{} does not exist",
                p.display()
            )));
        }
    }
    Ok(files)
}

fn is_yaml(p: &Path) -> bool {
    p.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("yaml") || e.eq_ignore_ascii_case("yml"))
}

pub(crate) fn parse_all(files: &[PathBuf]) -> Result<Vec<(String, TestFile)>> {
    let mut tests = Vec::new();
    let mut errors = Vec::new();
    for f in files {
        let shown = f.display().to_string();
        match std::fs::read_to_string(f) {
            Err(e) => errors.push(format!("{shown}: cannot read: {e}")),
            Ok(text) => match autolab_test::parse(&text) {
                Ok(t) => tests.push((shown, t)),
                Err(e) => errors.push(format!("{shown}: {}", e.message())),
            },
        }
    }
    if errors.is_empty() {
        Ok(tests)
    } else {
        Err(Error::InvalidArgs(errors.join("\n")))
    }
}

fn artifacts_dir(opt: Option<PathBuf>, needs_screenshots: bool) -> Result<PathBuf> {
    let cwd = std::env::current_dir()
        .map_err(|e| Error::Other(format!("cannot read current directory: {e}")))?;
    let dir = opt.unwrap_or_else(|| PathBuf::from("test-results"));
    let dir = if dir.is_absolute() {
        dir
    } else {
        cwd.join(dir)
    };
    let text = dir.to_string_lossy();
    if needs_screenshots && text.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(Error::InvalidArgs(format!(
            "artifacts directory {text:?} must not contain whitespace: screenshot paths are passed to the DHU console, which cannot take it safely"
        )));
    }
    Ok(dir)
}

fn word(o: Outcome) -> &'static str {
    match o {
        Outcome::Passed => "PASSED",
        Outcome::Failed => "FAILED",
        Outcome::Error => "ERROR",
        Outcome::Skipped => "SKIPPED",
    }
}

/// Flush every progress line even when stdout is redirected to a CI log. JSON stays on stdout.
pub(crate) fn progress(output: &crate::test_output::Output, message: impl std::fmt::Display) {
    output.line(message);
}

fn step_description(step: &autolab_test::schema::Step) -> String {
    use autolab_test::schema::Step;
    match step {
        Step::UiTap { text, until, .. } => format!(
            " {text:?}{}",
            until
                .as_ref()
                .map(|text| format!(" until {text:?}"))
                .unwrap_or_default()
        ),
        Step::UiWaitFor { text, .. } | Step::UiExpect { text, .. } => format!(" {text:?}"),
        Step::HttpRequest { method, url, .. } => format!(" {method:?} {url}"),
        Step::Wait { duration } => format!(" ({})", autolab_test::runner::fmt_dur(duration.0)),
        Step::Screenshot { path } => format!(" {path}"),
        _ => String::new(),
    }
}

fn print_step(output: &crate::test_output::Output, s: &autolab_test::StepResult) {
    let tag = match s.outcome {
        Outcome::Passed => "PASS",
        Outcome::Failed => "FAIL",
        Outcome::Error => "ERROR",
        Outcome::Skipped => "SKIP",
    };
    let name = if s.index == 0 {
        s.action.clone()
    } else {
        format!("{}. {}", s.index, s.action)
    };
    progress(
        output,
        format!(
            "  {tag} {name}: {} ({:.1}s)",
            s.detail,
            s.duration_ms as f64 / 1000.0
        ),
    );
}

/// First Ctrl+C or SIGTERM asks the run to stop cleanly. A second one exits at once, in case a
/// backend call is stuck and the clean stop never comes.
pub(crate) fn install_interrupt_handler(flag: &Arc<AtomicBool>) -> Result<()> {
    let flag = Arc::clone(flag);
    ctrlc::set_handler(move || {
        if flag.swap(true, Ordering::SeqCst) {
            std::process::exit(i32::from(INTERRUPTED));
        }
    })
    .map_err(|e| Error::Other(format!("cannot install the interrupt handler: {e}")))
}

pub fn run(mut opts: Options) -> Result<u8> {
    opts.paths = crate::test_selection::resolve(opts.paths)?;
    let output = Arc::clone(&opts.output);
    if opts.paths.len() == 1 && crate::suite::is_suite(&opts.paths[0])? {
        return crate::suite::run(opts);
    }
    let files = collect(&opts.paths)?;
    let tests = crate::test_selection::filter(parse_all(&files)?, opts.filter.as_deref())?; // before touching the session
    if opts.validate {
        if opts.json {
            println!(
                "{}",
                serde_json::json!({"valid": true, "tests": tests.len()})
            );
        } else {
            println!("Validated {} test files", tests.len());
        }
        return Ok(0);
    }
    let needs_shots = tests.iter().any(|(_, t)| {
        t.steps
            .iter()
            .any(|s| matches!(s, autolab_test::schema::Step::Screenshot { .. }))
    });
    let dir = artifacts_dir(opts.artifacts_dir, needs_shots)?;

    progress(&output, "Connecting to the head-unit session...");
    let client = match &opts.runtime_dir {
        Some(dir) => autolab_daemon::Client::connect_in(dir)?,
        None => daemon_ctl::connect()?,
    };
    let mut backend = DaemonBackend::new(client);
    let head = backend.status()?; // no session or dead DHU: exit 3 before any test runs
    let cancel = match opts.cancel {
        Some(cancel) => cancel,
        None => {
            let cancel = Arc::new(AtomicBool::new(false));
            install_interrupt_handler(&cancel)?;
            cancel
        }
    };
    // Google's DHU owns its own window; the AutoLab viewer is for native sessions only.
    let _viewer = if opts.show_head_unit && head.backend == "native" {
        progress(&output, "Opening the head-unit viewer...");
        Some(crate::viewer::Viewer::start(
            opts.runtime_dir.as_deref(),
            &dir,
        )?)
    } else {
        None
    };
    let clock = SystemClock::with_cancel(Arc::clone(&cancel));
    let started_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    if !opts.json {
        progress(&output, "AutoLab Test Runner\n");
    }
    let cfg = RunConfig {
        application: opts
            .app
            .as_ref()
            .map(|app| app as &dyn autolab_test::runner::ApplicationControl),
        app_label: opts.app.as_ref().map(|app| app.label.as_str()),
        artifacts_dir: &dir,
        skip_unsupported: opts.skip_unsupported,
        clock: &clock,
    };
    let mut results = Vec::new();
    for (file, test) in &tests {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        progress(&output, format!("Test: {}", test.name));
        if let Some(app) = &opts.app {
            progress(&output, format!("  Resetting application {}...", app.label));
            app.reset()?;
        }
        let r = run_test_with_progress(test, file, &mut backend, &cfg, &mut |event| match event {
            Progress::Started { index, action } => {
                let description = index
                    .checked_sub(1)
                    .and_then(|n| test.steps.get(n))
                    .map(step_description)
                    .unwrap_or_default();
                let name = if index == 0 {
                    action.to_string()
                } else {
                    format!("{index}. {action}")
                };
                progress(&output, format!("  RUN {name}{description}"));
            }
            Progress::Finished(step) => print_step(&output, step),
        });
        progress(
            &output,
            format!(
                "Result: {}\nDuration: {:.1}s\n",
                word(r.outcome),
                r.duration_ms as f64 / 1000.0
            ),
        );
        results.push(r);
    }

    // An interrupt wins over every other code, even if it arrived between two tests.
    let interrupted = cancel.load(Ordering::SeqCst);
    let not_run = tests.len() - results.len();
    let exit_code = if interrupted {
        INTERRUPTED
    } else {
        exit_code_for(&results)
    };
    let summary = Summary::of(&results);
    let report = RunReport {
        metadata: Metadata {
            autolab_version: env!("CARGO_PKG_VERSION").into(),
            started_at: iso8601_utc(started_unix),
            started_at_unix: started_unix,
            duration_ms: u64::try_from(clock.now().as_millis()).unwrap_or(u64::MAX),
            backend: Some(BackendMeta {
                name: head.backend,
                version: head.backend_version,
                build: head.backend_build,
                serial: head.serial,
            }),
            // Not observable through the backend trait today, so reported as null.
            device_model: None,
            android_version: None,
            android_auto_version: None,
            display: None,
            summary: summary.clone(),
        },
        exit_code,
        interrupted,
        tests: results,
    };

    let formats: Vec<Format> = if opts.report.is_empty() {
        vec![Format::Json]
    } else {
        opts.report
            .iter()
            .map(|f| match f {
                ReportFormat::Json => Format::Json,
                ReportFormat::Junit => Format::Junit,
            })
            .collect()
    };
    let written = write_artifacts(&dir, &report, &formats)
        .map_err(|e| Error::Other(format!("cannot write artifacts to {}: {e}", dir.display())))?;
    // A report left over from an earlier run must not be mistaken for this run's.
    for (fmt, name) in [
        (Format::Json, "results.json"),
        (Format::Junit, "results.xml"),
    ] {
        if !formats.contains(&fmt) {
            let _ = std::fs::remove_file(dir.join(name));
        }
    }

    if opts.json {
        println!("{}", autolab_test::report::to_json(&report));
    } else {
        println!(
            "{} test{}: {} passed, {} failed, {} errors, {} skipped",
            summary.tests,
            if summary.tests == 1 { "" } else { "s" },
            summary.passed,
            summary.failed,
            summary.errors,
            summary.skipped
        );
        if interrupted {
            let rest = match not_run {
                0 => String::new(),
                1 => " 1 test was not run.".into(),
                n => format!(" {n} tests were not run."),
            };
            println!("Interrupted.{rest} Video focus was restored if a test had left it off.");
        }
        println!("Artifacts: {}", dir.display());
        for p in &written {
            println!("  {}", p.display());
        }
        for t in report
            .tests
            .iter()
            .filter(|t| !t.passed() && t.outcome != Outcome::Skipped)
        {
            eprintln!("\n{}", step_log(t).trim_end());
        }
    }
    Ok(exit_code)
}

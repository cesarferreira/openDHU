//! `aa test`: parse every file first, then run them through the session backend.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::ValueEnum;
use opendhu_core::{Error, HeadUnitBackend, Result};
use opendhu_test::report::{
    BackendMeta, Format, Metadata, RunReport, Summary, exit_code_for, iso8601_utc, write_artifacts,
};
use opendhu_test::runner::{
    Clock, INTERRUPTED, Progress, RunConfig, SystemClock, run_test_with_progress,
};
use opendhu_test::{Outcome, TestFile};

use crate::daemon_ctl;
use crate::session_backend::DaemonBackend;

#[derive(ValueEnum, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReportFormat {
    Json,
    Junit,
    Html,
}

#[derive(Clone)]
pub struct Options {
    pub paths: Vec<PathBuf>,
    pub list: bool,
    pub batch: bool,
    pub verbose: bool,
    pub filter: Option<String>,
    pub output: Arc<crate::test_output::Output>,
    pub validate: bool,
    pub usb: bool,
    /// Let a managed emulator run fix up Android Auto's phone-side settings.
    pub phone_setup: bool,
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
            Ok(text) => match opendhu_test::parse(&text) {
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
    let dir = opt.unwrap_or_else(|| PathBuf::from("build/test-results"));
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

fn step_description(step: &opendhu_test::schema::Step) -> String {
    use opendhu_test::schema::Step;
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
        Step::Wait { duration } => format!(" ({})", opendhu_test::runner::fmt_dur(duration.0)),
        Step::Screenshot { path } => format!(" {path}"),
        _ => String::new(),
    }
}

fn print_step(
    output: &crate::test_output::Output,
    s: &opendhu_test::StepResult,
    step: Option<&opendhu_test::schema::Step>,
) {
    use opendhu_test::schema::Step;

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
    let detail = match (s.outcome, step) {
        (Outcome::Passed, Some(Step::UiExpect { text, .. } | Step::UiWaitFor { text, .. })) => {
            format!("projected text {text:?} visible")
        }
        _ => s.detail.clone(),
    };
    progress(
        output,
        format!(
            "  {tag} {name}: {} ({:.1}s)",
            detail,
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

pub fn run(opts: Options) -> Result<u8> {
    let output = Arc::clone(&opts.output);
    let footer = !opts.list && !opts.validate;
    let verbose = opts.verbose;
    let result = run_with_emulators(opts, None);
    if footer {
        output.finish(
            match &result {
                Ok(code) => *code,
                Err(error) => error.exit_code(),
            },
            verbose,
        );
    }
    result
}

pub(crate) fn run_with_emulators(
    mut opts: Options,
    emulators: Option<&mut crate::suite::EmulatorPool>,
) -> Result<u8> {
    if opts.list {
        return list_tests(&opts);
    }
    if opts.paths.is_empty() || crate::test_selection::is_all(&opts.paths) {
        return run_all(opts);
    }
    opts.paths = crate::test_selection::resolve(opts.paths)?;
    let output = Arc::clone(&opts.output);
    if opts.paths.len() == 1 && crate::suite::is_suite(&opts.paths[0])? {
        let count_suite = !opts.batch && !opts.validate;
        let result = crate::suite::run(opts, emulators);
        if count_suite {
            output.suite_finished(match &result {
                Ok(code) => *code,
                Err(error) => error.exit_code(),
            });
        }
        return result;
    }
    if opts.usb {
        return Err(Error::InvalidArgs(
            "--usb requires a managed suite (for example `aa test sample --usb`)".into(),
        ));
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
            .any(|s| matches!(s, opendhu_test::schema::Step::Screenshot { .. }))
    });
    let dir = artifacts_dir(opts.artifacts_dir, needs_shots)?;

    output.set_viewer_runtime(opts.runtime_dir.as_deref());
    progress(&output, "Connecting to the head-unit session...");
    let client = match &opts.runtime_dir {
        Some(dir) => opendhu_daemon::Client::connect_in(dir)?,
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
    // Google's DHU owns its own window; the openDHU viewer is for native sessions only.
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
        progress(&output, "openDHU Test Runner\n");
    }
    let cfg = RunConfig {
        application: opts
            .app
            .as_ref()
            .map(|app| app as &dyn opendhu_test::runner::ApplicationControl),
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
        let mut r =
            run_test_with_progress(test, file, &mut backend, &cfg, &mut |event| match event {
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
                Progress::Finished(step) => print_step(
                    &output,
                    step,
                    step.index.checked_sub(1).and_then(|n| test.steps.get(n)),
                ),
            });
        if let Some(app) = &opts.app {
            let health = app.monitor.finish();
            let failure = match health {
                Ok(Some(detail)) => Some((Outcome::Failed, 7, detail)),
                Err(error) => Some((
                    Outcome::Error,
                    error.exit_code(),
                    error.message().to_string(),
                )),
                Ok(None) => None,
            };
            if let Some((outcome, code, detail)) = failure {
                let step = opendhu_test::model::StepResult {
                    index: r.steps.last().map_or(0, |step| step.index) + 1,
                    action: "app.health".into(),
                    outcome,
                    detail: detail.clone(),
                    duration_ms: 0,
                    exit_code: Some(code),
                };
                print_step(&output, &step, None);
                r.steps.push(step);
                r.message = Some(match r.message.take() {
                    Some(message) => format!("{message}; {detail}"),
                    None => detail,
                });
                r.outcome = outcome;
                r.exit_code = code;
            }
        }
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
            opendhu_version: env!("CARGO_PKG_VERSION").into(),
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
        vec![Format::Json, Format::Html]
    } else {
        opts.report
            .iter()
            .map(|f| match f {
                ReportFormat::Json => Format::Json,
                ReportFormat::Junit => Format::Junit,
                ReportFormat::Html => Format::Html,
            })
            .collect()
    };
    let written = write_artifacts(&dir, &report, &formats)
        .map_err(|e| Error::Other(format!("cannot write artifacts to {}: {e}", dir.display())))?;
    // A report left over from an earlier run must not be mistaken for this run's.
    for (fmt, name) in [
        (Format::Json, "results.json"),
        (Format::Junit, "results.xml"),
        (Format::Html, "results.html"),
    ] {
        if !formats.contains(&fmt) {
            let _ = std::fs::remove_file(dir.join(name));
        }
    }

    output.record_run(&report, &dir);
    if opts.json && !opts.batch {
        println!("{}", opendhu_test::report::to_json(&report));
    } else if !opts.json {
        if interrupted {
            let rest = match not_run {
                0 => String::new(),
                1 => " 1 test was not run.".into(),
                n => format!(" {n} tests were not run."),
            };
            progress(
                &output,
                format!("Interrupted.{rest} Video focus was restored if a test had left it off."),
            );
        }
        output.report(&report, &dir, written);
    }
    Ok(exit_code)
}

fn list_tests(opts: &Options) -> Result<u8> {
    if opts.paths.is_empty() && opts.filter.is_none() {
        let suites = crate::test_selection::registered()?;
        if opts.json {
            println!("{}", serde_json::json!({"suites": suites}));
        } else if suites.is_empty() {
            println!("No suites registered in opendhu.yaml");
        } else {
            for suite in suites {
                println!("{}\t{}", suite.name, suite.path.display());
            }
        }
        return Ok(0);
    }
    let paths = if opts.paths.is_empty() || crate::test_selection::is_all(&opts.paths) {
        crate::test_selection::registered()?
            .into_iter()
            .map(|suite| suite.path)
            .collect()
    } else {
        crate::test_selection::resolve(opts.paths.clone())?
    };
    let mut tests = Vec::new();
    for path in paths {
        if crate::suite::is_suite(&path)? {
            tests.extend(crate::suite::inspect(&path, None)?);
        } else {
            tests.extend(parse_all(&collect(&[path])?)?);
        }
    }
    let tests = crate::test_selection::filter(tests, opts.filter.as_deref())?;
    if opts.json {
        let tests: Vec<_> = tests
            .iter()
            .map(|(file, test)| serde_json::json!({"file": file, "name": test.name}))
            .collect();
        println!("{}", serde_json::json!({"tests": tests}));
    } else {
        for (file, test) in tests {
            println!("{}\t{file}", test.name);
        }
    }
    Ok(0)
}

/// Removes what an earlier default-folder run left behind: its reports, logs and per-suite folders,
/// plus the `run-*` and `all-*` folders older versions created. Nothing else in the folder is touched.
pub(crate) fn reset_artifacts(dir: &Path, suites: &[&str]) {
    const OWNED: [&str; 10] = [
        "results.html",
        "results.json",
        "results.xml",
        "metadata.json",
        "suite.json",
        "head-unit.log",
        "emulator.log",
        "logcat.txt",
        "logs",
        "screenshots",
    ];
    let timestamped = |name: &str| {
        ["run-", "all-"].iter().any(|prefix| {
            name.strip_prefix(prefix)
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if OWNED.contains(&&*name) || suites.contains(&&*name) || timestamped(&name) {
            let path = entry.path();
            let _ = if path.is_dir() {
                std::fs::remove_dir_all(path)
            } else {
                std::fs::remove_file(path)
            };
        }
    }
}

/// One `results.html` and `results.json` next to the per-suite folders, so a run reads as one report.
fn write_combined(
    opts: &Options,
    artifacts: &Path,
    results: &[serde_json::Value],
    interrupted: bool,
) {
    let runs = opts.output.take_runs();
    let cwd = std::env::current_dir().unwrap_or_default();
    let abs = |p: &Path| {
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            cwd.join(p)
        }
    };
    let entries: Vec<_> = results
        .iter()
        .map(|r| {
            let name = r["name"].as_str().unwrap_or_default();
            let dir = abs(&artifacts.join(name));
            let report = runs
                .iter()
                .find(|(d, _)| abs(d) == dir)
                .map(|(_, report)| report);
            (
                name,
                r["exitCode"].as_u64().unwrap_or(1) as u8,
                r["error"].as_str(),
                report,
            )
        })
        .collect();
    let suites: Vec<_> = entries
        .iter()
        .map(
            |&(name, exit_code, error, report)| opendhu_test::report::SuiteEntry {
                name,
                exit_code,
                error,
                report,
            },
        )
        .collect();
    let formats = if opts.report.is_empty() {
        vec![ReportFormat::Json, ReportFormat::Html]
    } else {
        opts.report.clone()
    };
    let abs_dir = abs(artifacts);
    let mut html = None;
    for (format, file) in [
        (ReportFormat::Html, "results.html"),
        (ReportFormat::Json, "results.json"),
    ] {
        let path = abs_dir.join(file);
        if !formats.contains(&format) {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        let body = match format {
            ReportFormat::Html => opendhu_test::report::to_combined_html(&suites, interrupted),
            _ => serde_json::to_string_pretty(&serde_json::json!({
                "suites": results,
                "reports": suites.iter().map(|s| serde_json::json!({"name": s.name, "report": s.report})).collect::<Vec<_>>(),
                "interrupted": interrupted,
            }))
            .unwrap_or_default(),
        };
        if std::fs::create_dir_all(&abs_dir).is_ok()
            && std::fs::write(&path, body).is_ok()
            && format == ReportFormat::Html
        {
            html = Some(path);
        }
    }
    if let Some(path) = html {
        opts.output.combined_report(path);
    }
}

fn run_all(opts: Options) -> Result<u8> {
    let suites = crate::test_selection::registered()?;
    if suites.is_empty() {
        return Err(Error::InvalidArgs(
            "no suites registered in opendhu.yaml".into(),
        ));
    }
    // Validate every suite before installing apps or acquiring a device.
    let mut selected = Vec::new();
    for suite in suites {
        let managed = crate::suite::is_suite(&suite.path)?;
        let tests = if managed {
            crate::suite::inspect(&suite.path, opts.filter.as_deref())?
        } else {
            crate::test_selection::matching(
                parse_all(&collect(std::slice::from_ref(&suite.path))?)?,
                opts.filter.as_deref(),
            )?
        };
        if tests.is_empty() {
            continue;
        }
        if opts.usb && !managed {
            return Err(Error::InvalidArgs(format!(
                "--usb requires a managed suite: {}",
                suite.path.display(),
            )));
        }
        selected.push((suite, tests.len()));
    }
    if selected.is_empty() {
        return Err(Error::InvalidArgs(match &opts.filter {
            Some(query) => format!("--filter {query:?} matched no tests in any registered suite"),
            None => "no tests found in registered suites".into(),
        }));
    }
    if opts.validate {
        let total: usize = selected.iter().map(|(_, count)| count).sum();
        if opts.json {
            let suites: Vec<_> = selected.iter().map(|(s, count)| serde_json::json!({"name": s.name, "path": s.path, "tests": count})).collect();
            println!(
                "{}",
                serde_json::json!({"valid": true, "suites": suites, "tests": total})
            );
        } else {
            println!("Validated {} suites and {total} test files", selected.len());
        }
        return Ok(0);
    }
    let cancel = Arc::new(AtomicBool::new(false));
    install_interrupt_handler(&cancel)?;
    // The default folder holds only the latest run; an explicit --artifacts-dir is never cleared.
    let artifacts = match opts.artifacts_dir.clone() {
        Some(dir) => dir,
        None => {
            let dir = PathBuf::from("build/test-results");
            let names: Vec<&str> = selected.iter().map(|(s, _)| s.name.as_str()).collect();
            reset_artifacts(&dir, &names);
            dir
        }
    };
    opts.output.artifacts(&artifacts);
    let mut results = Vec::new();
    let mut exit_code = 0;
    let mut emulators = crate::suite::EmulatorPool::default();
    for (suite, count) in selected {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        progress(&opts.output, format!("Running suite {}...", suite.name));
        let mut child = opts.clone();
        child.paths = vec![suite.path.clone()];
        child.batch = true;
        child.cancel = Some(Arc::clone(&cancel));
        let dir = artifacts.join(&suite.name);
        child.artifacts_dir = Some(dir.clone());
        let (code, error) = match run_with_emulators(child, Some(&mut emulators)) {
            Ok(code) => (code, None),
            Err(error) => {
                progress(
                    &opts.output,
                    format!("Suite {} failed: {error}", suite.name),
                );
                (error.exit_code(), Some(error.to_string()))
            }
        };
        if exit_code == 0 {
            exit_code = code;
        }
        opts.output.suite_finished(code);
        results.push(serde_json::json!({"name": suite.name, "path": suite.path, "tests": count, "exitCode": code, "error": error, "artifacts": dir}));
        if code == INTERRUPTED {
            break;
        }
    }
    drop(emulators);
    let interrupted = cancel.load(Ordering::SeqCst);
    if interrupted {
        exit_code = INTERRUPTED;
    }
    write_combined(&opts, &artifacts, &results, interrupted);
    if opts.json {
        println!(
            "{}",
            serde_json::json!({"suites": results, "exitCode": exit_code, "interrupted": interrupted})
        );
    } else if interrupted {
        progress(&opts.output, "Interrupted; remaining suites were not run.");
    }
    Ok(exit_code)
}

#[cfg(test)]
mod reset_tests {
    use super::reset_artifacts;

    #[test]
    fn reset_clears_only_what_runs_own() {
        let dir = std::env::temp_dir().join(format!("aa-reset-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["logs", "robotaxi", "run-123", "all-456", "keep-me"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        for file in ["results.html", "notes.txt"] {
            std::fs::write(dir.join(file), "x").unwrap();
        }
        reset_artifacts(&dir, &["robotaxi"]);
        assert!(!dir.join("logs").exists());
        assert!(!dir.join("robotaxi").exists());
        assert!(!dir.join("run-123").exists());
        assert!(!dir.join("all-456").exists());
        assert!(!dir.join("results.html").exists());
        assert!(dir.join("keep-me").exists());
        assert!(dir.join("notes.txt").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

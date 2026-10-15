//! `autolab test`: parse every file first, then run them through the session backend.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use autolab_core::{Error, HeadUnitBackend, Result};
use autolab_test::report::{
    BackendMeta, Format, Metadata, RunReport, Summary, exit_code_for, iso8601_utc, step_log,
    write_artifacts,
};
use autolab_test::runner::{Clock, RunConfig, SystemClock, run_test};
use autolab_test::{Outcome, TestFile, TestResult};
use clap::ValueEnum;

use crate::session_backend::SessionBackend;

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportFormat {
    Json,
    Junit,
}

pub struct Options {
    pub paths: Vec<PathBuf>,
    pub report: Vec<ReportFormat>,
    pub artifacts_dir: Option<PathBuf>,
    pub skip_unsupported: bool,
    pub json: bool,
}

/// Expand files and directories (recursively, sorted) into test files.
fn collect(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
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

fn parse_all(files: &[PathBuf]) -> Result<Vec<(String, TestFile)>> {
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

fn print_test(t: &TestResult) {
    println!("Test: {}", t.name);
    for s in &t.steps {
        let tag = match s.outcome {
            Outcome::Passed => "PASS ",
            Outcome::Failed => "FAIL ",
            Outcome::Error => "ERROR",
            Outcome::Skipped => "SKIP ",
        };
        let name = if s.index == 0 {
            s.action.clone()
        } else {
            format!("{}. {}", s.index, s.action)
        };
        println!("  {tag} {name}: {}", s.detail);
    }
    println!("Result: {}", word(t.outcome));
    println!("Duration: {:.1}s\n", t.duration_ms as f64 / 1000.0);
}

pub fn run(opts: Options) -> Result<u8> {
    let files = collect(&opts.paths)?;
    let tests = parse_all(&files)?; // exit 2 before the session is touched
    let needs_shots = tests.iter().any(|(_, t)| {
        t.steps
            .iter()
            .any(|s| matches!(s, autolab_test::schema::Step::Screenshot { .. }))
    });
    let dir = artifacts_dir(opts.artifacts_dir, needs_shots)?;

    let mut backend = SessionBackend;
    let head = backend.status()?; // no session or dead DHU: exit 3 before any test runs
    let clock = SystemClock::new();
    let started_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    if !opts.json {
        println!("AutoLab Test Runner\n");
    }
    let cfg = RunConfig {
        artifacts_dir: &dir,
        skip_unsupported: opts.skip_unsupported,
        clock: &clock,
    };
    let mut results = Vec::new();
    for (file, test) in &tests {
        let r = run_test(test, file, &mut backend, &cfg);
        if !opts.json {
            print_test(&r);
        }
        results.push(r);
    }

    let exit_code = exit_code_for(&results);
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

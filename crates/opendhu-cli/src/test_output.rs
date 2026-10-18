//! Terminal presentation is separate from the runner and its machine-readable reports.
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use opendhu_test::Outcome;
use opendhu_test::report::{RunReport, Summary, step_log};

#[derive(Default)]
struct Footer {
    summary: Summary,
    suites: usize,
    failed_suites: usize,
    artifacts: Option<PathBuf>,
    reports: Vec<PathBuf>,
    written: Vec<PathBuf>,
    failures: Vec<(String, PathBuf, String)>,
}

fn display_path(path: &Path) -> String {
    let cwd = std::env::current_dir().ok();
    let path = cwd
        .as_ref()
        .and_then(|cwd| path.strip_prefix(cwd).ok())
        .unwrap_or(path);
    if path.as_os_str().is_empty() {
        ".".into()
    } else {
        path.display().to_string()
    }
}

struct State {
    active: Option<(String, Instant)>,
    stopped: bool,
    frame: usize,
}
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    color: bool,
}
pub(crate) struct Output {
    json: bool,
    interactive: bool,
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    footer: Mutex<Footer>,
}
const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn styled(line: &str, color: bool) -> String {
    if !color {
        return line.to_string();
    }
    let text = line.trim_start();
    let indent = &line[..line.len() - text.len()];
    if let Some((label, path)) = text.split_once(':')
        && matches!(label, "Report" | "Artifacts" | "Log")
    {
        return format!("{indent}\x1b[36m{label}:\x1b[0m\x1b[2m{path}\x1b[0m");
    }
    let code = if text.starts_with("PASS ") || text == "Result: PASSED" {
        "32"
    } else if text.starts_with("FAIL ")
        || text.starts_with("ERROR ")
        || text == "Result: FAILED"
        || text == "Result: ERROR"
    {
        "31"
    } else if text.starts_with("SKIP ")
        || text.starts_with("INTERRUPTED ")
        || text == "Result: SKIPPED"
    {
        "33"
    } else if text.starts_with("RUN ") {
        "36"
    } else if text.starts_with("Test:")
        || text.starts_with("Suite:")
        || text == "openDHU Test Runner"
    {
        "1;36"
    } else if text.starts_with("Duration:")
        || text.starts_with("Artifacts:")
        || text.starts_with('/')
        || text.starts_with("File:")
    {
        "2"
    } else {
        return line.to_string();
    };
    // Keep details readable; status labels carry the color, while headings use the whole line.
    if let Some((tag, rest)) = text.split_once(' ')
        && matches!(
            tag,
            "PASS" | "FAIL" | "ERROR" | "SKIP" | "RUN" | "INTERRUPTED"
        )
    {
        format!("{indent}\x1b[{code}m{tag}\x1b[0m {rest}")
    } else {
        format!("{indent}\x1b[{code}m{text}\x1b[0m")
    }
}

fn draw(shared: &Shared, state: &mut State) {
    let Some((message, started)) = &state.active else {
        return;
    };
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let columns = if unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) } == 0
        && size.ws_col > 20
    {
        usize::from(size.ws_col)
    } else {
        100
    };
    // Keep the animation on one physical line, including narrow terminals and long APK paths.
    let available = columns.saturating_sub(19);
    let clean: String = message.chars().filter(|c| !c.is_control()).collect();
    let message = if clean.chars().count() > available {
        format!(
            "{}…",
            clean
                .chars()
                .take(available.saturating_sub(1))
                .collect::<String>()
        )
    } else {
        clean
    };
    let frame = FRAMES[state.frame % FRAMES.len()];
    let frame = if shared.color {
        format!("\x1b[36m{frame}\x1b[0m")
    } else {
        frame.into()
    };
    let mut out = std::io::stdout().lock();
    let _ = write!(
        out,
        "\r\x1b[2K  {frame} {message} [{:.1}s]",
        started.elapsed().as_secs_f64()
    );
    let _ = out.flush();
    state.frame += 1;
}

impl Output {
    pub(crate) fn new(json: bool, interactive: bool) -> Self {
        let terminal = !json
            && std::io::stdout().is_terminal()
            && std::env::var("TERM").as_deref() != Ok("dumb");
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                active: None,
                stopped: false,
                frame: 0,
            }),
            wake: Condvar::new(),
            color: terminal && std::env::var_os("NO_COLOR").is_none(),
        });
        let interactive = interactive && terminal;
        let worker = interactive.then(|| {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                while !state.stopped {
                    draw(&shared, &mut state);
                    state = shared
                        .wake
                        .wait_timeout(state, Duration::from_millis(80))
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
            })
        });
        Self {
            json,
            interactive,
            shared,
            worker,
            footer: Mutex::new(Footer::default()),
        }
    }

    pub(crate) fn artifacts(&self, path: &Path) {
        self.footer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .artifacts
            .get_or_insert_with(|| path.into());
    }

    pub(crate) fn suite_finished(&self, code: u8) {
        let mut footer = self.footer.lock().unwrap_or_else(|e| e.into_inner());
        footer.suites += 1;
        footer.failed_suites += usize::from(code != 0);
    }

    pub(crate) fn report(&self, report: &RunReport, dir: &Path, written: Vec<PathBuf>) {
        if self.json {
            return;
        }
        let mut footer = self.footer.lock().unwrap_or_else(|e| e.into_inner());
        let summary = &report.metadata.summary;
        footer.summary.tests += summary.tests;
        footer.summary.passed += summary.passed;
        footer.summary.failed += summary.failed;
        footer.summary.errors += summary.errors;
        footer.summary.skipped += summary.skipped;
        footer.artifacts.get_or_insert_with(|| dir.into());
        if let Some(path) = ["results.html", "results.json", "results.xml"]
            .into_iter()
            .map(|name| dir.join(name))
            .find(|path| written.contains(path))
        {
            footer.reports.push(path);
        }
        let logs = dir.join("logs");
        for (test, path) in report.tests.iter().zip(
            written
                .iter()
                .filter(|path| path.parent() == Some(logs.as_path())),
        ) {
            if matches!(test.outcome, Outcome::Failed | Outcome::Error) {
                footer
                    .failures
                    .push((test.name.clone(), path.clone(), step_log(test)));
            }
        }
        footer.written.extend(written);
    }

    /// Called once by the outer command, after suite and emulator cleanup.
    pub(crate) fn finish(&self, code: u8, verbose: bool) {
        if self.json {
            return;
        }
        let footer = std::mem::take(&mut *self.footer.lock().unwrap_or_else(|e| e.into_inner()));
        let summary = &footer.summary;
        if summary.tests == 0 && footer.suites == 0 && footer.artifacts.is_none() {
            return;
        }
        let status = if code == opendhu_test::runner::INTERRUPTED {
            "INTERRUPTED"
        } else if code != 0 {
            "FAIL"
        } else if summary.tests > 0 && summary.skipped == summary.tests {
            "SKIP"
        } else {
            "PASS"
        };
        let mut counts = vec![format!(
            "{} test{}",
            summary.tests,
            if summary.tests == 1 { "" } else { "s" },
        )];
        if footer.suites > 0 {
            counts.push(format!(
                "{} suite{}",
                footer.suites,
                if footer.suites == 1 { "" } else { "s" }
            ));
        }
        for (count, label) in [
            (summary.failed, "failed"),
            (summary.errors, "errors"),
            (summary.skipped, "skipped"),
        ] {
            if count > 0 {
                counts.push(format!("{count} {label}"));
            }
        }
        if footer.failed_suites > 0 {
            counts.push(format!(
                "{} suite{} failed",
                footer.failed_suites,
                if footer.failed_suites == 1 { "" } else { "s" }
            ));
        }
        self.line(format!("\n{status}  {}\n", counts.join(" · ")));
        for (name, log, details) in &footer.failures {
            self.line(format!("FAIL {name}\nLog:        {}", display_path(log)));
            if verbose {
                self.line(details.trim_end());
            }
        }
        for report in &footer.reports {
            self.line(format!("Report:     {}", display_path(report)));
        }
        if let Some(dir) = &footer.artifacts {
            self.line(format!("Artifacts:  {}", display_path(dir)));
        }
        if verbose {
            for path in &footer.written {
                self.line(format!("File:       {}", display_path(path)));
            }
        }
    }

    pub(crate) fn line(&self, message: impl std::fmt::Display) {
        let message = message.to_string();
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if self.interactive && state.active.take().is_some() {
            let mut out = std::io::stdout().lock();
            let _ = write!(out, "\r\x1b[2K");
            let _ = out.flush();
        }
        let busy = message.trim_start().starts_with("RUN ") || message.ends_with("...");
        if self.interactive && busy {
            state.active = Some((message.trim().to_string(), Instant::now()));
            draw(&self.shared, &mut state);
        } else {
            let mut out: Box<dyn Write> = if self.json {
                Box::new(std::io::stderr().lock())
            } else {
                Box::new(std::io::stdout().lock())
            };
            for line in message.split('\n') {
                let _ = writeln!(out, "{}", styled(line, self.shared.color));
            }
            let _ = out.flush();
        }
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        {
            let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.stopped = true;
            if self.interactive && state.active.take().is_some() {
                let mut out = std::io::stdout().lock();
                let _ = write!(out, "\r\x1b[2K");
                let _ = out.flush();
            }
            self.shared.wake.notify_one();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

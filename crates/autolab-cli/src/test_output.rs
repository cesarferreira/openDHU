//! Terminal presentation is separate from the runner and its machine-readable reports.
use std::io::{IsTerminal, Write};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

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
}
const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn styled(line: &str, color: bool) -> String {
    if !color {
        return line.to_string();
    }
    let text = line.trim_start();
    let indent = &line[..line.len() - text.len()];
    let code = if text.starts_with("PASS ") || text == "Result: PASSED" {
        "32"
    } else if text.starts_with("FAIL ")
        || text.starts_with("ERROR ")
        || text == "Result: FAILED"
        || text == "Result: ERROR"
    {
        "31"
    } else if text.starts_with("SKIP ") || text == "Result: SKIPPED" {
        "33"
    } else if text.starts_with("RUN ") {
        "36"
    } else if text.starts_with("Test:")
        || text.starts_with("Suite:")
        || text == "AutoLab Test Runner"
    {
        "1;36"
    } else if text.starts_with("Duration:")
        || text.starts_with("Artifacts:")
        || text.starts_with('/')
    {
        "2"
    } else {
        return line.to_string();
    };
    // Keep details readable; status labels carry the color, while headings use the whole line.
    if let Some((tag, rest)) = text.split_once(' ')
        && matches!(tag, "PASS" | "FAIL" | "ERROR" | "SKIP" | "RUN")
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

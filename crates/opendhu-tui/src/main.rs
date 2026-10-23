//! `opendhu-tui`: the head unit in a terminal. The picture is drawn with the kitty graphics
//! protocol; mouse and keys go to the daemon like the desktop window's. It needs a terminal that
//! speaks that protocol (kitty, Ghostty, WezTerm).

mod kitty;
mod layout;
mod session;
mod term;
mod video;

use std::io::{self, Read, Write};
use std::process::ExitCode;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use opendhu_core::{FocusState, Key};
use opendhu_daemon::Client;

use layout::Layout;
use term::{Arrow, Event, Mouse, Terminal};

/// A press that moves less than this many phone pixels before release is a tap.
const TAP_SLOP: u32 = 24;
const MIN_FRAME_GAP: Duration = Duration::from_millis(33);
const NOTE_FOR: Duration = Duration::from_secs(3);
const HINTS: &str = "click/drag: touch  wheel: scroll  esc: back  h: home  arrows/enter: dpad  \
                     s: search  space: play  n/p: track  f: video focus  S: screenshot  q: quit";

pub enum Msg {
    Input(Event),
    Frame,
    Status(Option<Status>),
    Note(String),
    /// One line of the test run's progress, and the end of the run.
    Log(String),
    LogEnd,
    /// The daemon to follow changed.
    Runtime,
}

#[derive(Clone)]
pub struct Status {
    summary: String,
    screen: (u32, u32),
    video_focus: FocusState,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> io::Result<()> {
    let terminal = Terminal::enter()?;
    let (tx, rx) = mpsc::channel();
    spawn_input(tx.clone());
    spawn_status(tx.clone());
    let log = spawn_log(tx.clone());
    let mut app = App::new(tx, &terminal, log);
    let mut out = io::stdout();
    loop {
        match rx.recv_timeout(Duration::from_millis(30)) {
            Ok(msg) => {
                if app.handle(msg) {
                    return Ok(());
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        app.render(&mut out)?;
    }
}

fn spawn_input(tx: Sender<Msg>) {
    thread::spawn(move || {
        let mut parser = term::Parser::default();
        let mut buf = [0u8; 1024];
        let mut stdin = io::stdin().lock();
        while let Ok(n) = stdin.read(&mut buf) {
            if n == 0 {
                break;
            }
            for event in parser.feed(&buf[..n]) {
                if tx.send(Msg::Input(event)).is_err() {
                    return;
                }
            }
        }
    });
}

/// `aa test --tui` hands over the run's progress as lines on the descriptor named by
/// `OPENDHU_TUI_LOG_FD`. Without it there is no log pane.
fn spawn_log(tx: Sender<Msg>) -> Option<Log> {
    use std::io::BufRead;
    use std::os::fd::FromRawFd;

    let fd: i32 = std::env::var("OPENDHU_TUI_LOG_FD").ok()?.parse().ok()?;
    // SAFETY: the parent passed this descriptor to this process alone, for this purpose.
    let pipe = unsafe { std::fs::File::from_raw_fd(fd) };
    thread::spawn(move || {
        for line in io::BufReader::new(pipe).lines().map_while(Result::ok) {
            // `aa` marks control lines with a leading record separator: `\x1eruntime\t<dir>`.
            if let Some(dir) = line.strip_prefix("\x1eruntime\t") {
                let dir = (!dir.is_empty()).then(|| dir.into());
                if session::set(dir) && tx.send(Msg::Runtime).is_err() {
                    return;
                }
                continue;
            }
            let clean = line
                .replace('\t', "    ")
                .chars()
                .filter(|c| !c.is_control())
                .collect();
            if tx.send(Msg::Log(clean)).is_err() {
                return;
            }
        }
        let _ = tx.send(Msg::LogEnd);
    });
    Some(Log::default())
}

/// The run's progress so far.
#[derive(Default)]
struct Log {
    lines: Vec<String>,
    ended: bool,
}

/// Lines kept; older ones are dropped, they are still in the shell's scrollback afterwards.
const LOG_KEEP: usize = 2000;

/// Colours for the labels the test runner prints, as in its own terminal output.
fn styled(line: &str, cols: u32) -> String {
    let text = fit(line, cols);
    let body = text.trim_start();
    let code = if body.starts_with("PASS ") || body == "Result: PASSED" {
        "32"
    } else if body.starts_with("FAIL ")
        || body.starts_with("ERROR ")
        || body == "Result: FAILED"
        || body == "Result: ERROR"
    {
        "31"
    } else if body.starts_with("SKIP ") || body.starts_with("INTERRUPTED ") {
        "33"
    } else if body.starts_with("RUN ") {
        "36"
    } else if body.starts_with("Test:") || body.starts_with("Suite:") {
        "1;36"
    } else {
        return text;
    };
    format!("\x1b[{code}m{text}\x1b[0m")
}

fn spawn_status(tx: Sender<Msg>) {
    thread::spawn(move || {
        loop {
            let status = session::client()
                .and_then(|c| c.status())
                .ok()
                .and_then(|s| {
                    let head = s.session?.details?.head_unit;
                    let screen = match (head.screen_width, head.screen_height) {
                        (Some(w), Some(h)) => (w.into(), h.into()),
                        _ => (
                            opendhu_native::protocol::SCREEN_WIDTH.into(),
                            opendhu_native::protocol::SCREEN_HEIGHT.into(),
                        ),
                    };
                    Some(Status {
                        summary: format!(
                            "{} {:?} {}x{} focus {:?}",
                            head.backend, head.state, screen.0, screen.1, head.video_focus.observed
                        ),
                        screen,
                        video_focus: head.video_focus.observed,
                    })
                });
            if tx.send(Msg::Status(status)).is_err() {
                return;
            }
            thread::sleep(Duration::from_secs(1));
        }
    });
}

struct App<'a> {
    tx: Sender<Msg>,
    terminal: &'a Terminal,
    video: video::Video,
    status: Option<Status>,
    note: Option<(String, Instant)>,
    pixel_mouse: bool,
    press: Option<(Instant, (u32, u32))>,
    /// What is on screen now, so a change redraws and an unchanged tick draws nothing.
    drawn: Option<(term::Size, Layout, Chrome)>,
    log: Option<Log>,
    canvas: kitty::Canvas,
    frame: Option<video::Frame>,
    frame_dirty: bool,
    last_draw: Option<Instant>,
    frames: u64,
    started: Instant,
}

impl<'a> App<'a> {
    fn new(tx: Sender<Msg>, terminal: &'a Terminal, log: Option<Log>) -> Self {
        Self {
            video: video::Video::new(tx.clone()),
            tx,
            terminal,
            status: None,
            note: None,
            pixel_mouse: false,
            press: None,
            drawn: None,
            log,
            canvas: kitty::Canvas::default(),
            frame: None,
            frame_dirty: false,
            last_draw: None,
            frames: 0,
            started: Instant::now(),
        }
    }

    fn screen(&self) -> (u32, u32) {
        self.status.as_ref().map_or(
            (
                opendhu_native::protocol::SCREEN_WIDTH.into(),
                opendhu_native::protocol::SCREEN_HEIGHT.into(),
            ),
            |s| s.screen,
        )
    }

    fn layout(&self) -> Layout {
        self.layout_in(term::size())
    }

    fn log_rows(&self, size: term::Size) -> u32 {
        if self.log.is_some() {
            layout::log_rows(size)
        } else {
            0
        }
    }

    fn layout_in(&self, size: term::Size) -> Layout {
        Layout::new(size, self.screen(), self.log_rows(size))
    }

    /// Returns true to quit.
    fn handle(&mut self, msg: Msg) -> bool {
        match msg {
            Msg::Frame => {
                if let Some(frame) = self.video.take() {
                    self.frames += 1;
                    self.frame = Some(frame);
                    self.frame_dirty = true;
                }
            }
            Msg::Status(status) => self.status = status,
            Msg::Runtime => {
                // The old session's picture and status are someone else's now.
                self.frame = None;
                self.frames = 0;
                self.status = None;
                self.started = Instant::now();
            }
            Msg::Log(line) => {
                if let Some(log) = &mut self.log {
                    log.lines.push(line);
                    if log.lines.len() > LOG_KEEP {
                        log.lines.drain(..LOG_KEEP / 2);
                    }
                }
            }
            Msg::LogEnd => {
                if let Some(log) = &mut self.log {
                    log.ended = true;
                }
            }
            Msg::Note(text) => self.note = Some((text, Instant::now())),
            Msg::Input(event) => return self.input(event),
        }
        false
    }

    fn input(&mut self, event: Event) -> bool {
        match event {
            Event::PixelMouse(true) => {
                if self.terminal.use_pixel_mouse().is_ok() {
                    self.pixel_mouse = true;
                }
            }
            Event::PixelMouse(false) => {}
            Event::Char('q') => return true,
            Event::Char('f') => {
                let on = self.status.as_ref().map(|s| s.video_focus) == Some(FocusState::On);
                self.act("video focus", move |c| c.focus_video(!on).map(drop));
            }
            Event::Char('S') => {
                let path = format!("aa-screenshot-{}.png", unix_seconds());
                let note = path.clone();
                self.act(&format!("saved {note}"), move |c| {
                    c.screenshot(std::path::Path::new(&path)).map(drop)
                });
            }
            Event::Char(c) => {
                if let Some(key) = key_for_char(c) {
                    self.key(key);
                }
            }
            Event::Escape => self.key(Key::Back),
            Event::Enter => self.key(Key::Enter),
            Event::Arrow(a) => self.key(match a {
                Arrow::Up => Key::Up,
                Arrow::Down => Key::Down,
                Arrow::Left => Key::Left,
                Arrow::Right => Key::Right,
            }),
            Event::Mouse { kind, x, y } => self.mouse(kind, (x, y)),
        }
        false
    }

    fn key(&self, key: Key) {
        self.act(&format!("{key:?}"), move |c| c.key(key, false).map(drop));
    }

    fn mouse(&mut self, kind: Mouse, at: (u32, u32)) {
        let layout = self.layout();
        let px = layout.mouse_pixel(at, self.pixel_mouse);
        match kind {
            Mouse::Press => {
                self.press = layout.screen_at(px, false).map(|p| (Instant::now(), p));
            }
            Mouse::Drag => {}
            Mouse::Release => {
                let Some((since, from)) = self.press.take() else {
                    return;
                };
                let Some(to) = layout.screen_at(px, true) else {
                    return;
                };
                if from.0.abs_diff(to.0).max(from.1.abs_diff(to.1)) < TAP_SLOP {
                    self.act("tap", move |c| c.tap(from.0, from.1).map(drop));
                } else {
                    let held = since
                        .elapsed()
                        .clamp(Duration::from_millis(100), Duration::from_millis(1500));
                    self.act("swipe", move |c| c.swipe(from, to, held).map(drop));
                }
            }
            Mouse::WheelUp | Mouse::WheelDown => {
                let Some(at) = layout.screen_at(px, false) else {
                    return;
                };
                let reach = layout.screen.1 / 3;
                // Wheel up shows what is above: drag the content down.
                let (from_y, to_y) = if kind == Mouse::WheelUp {
                    (
                        at.1.saturating_sub(reach / 2),
                        (at.1 + reach / 2).min(layout.screen.1 - 1),
                    )
                } else {
                    (
                        (at.1 + reach / 2).min(layout.screen.1 - 1),
                        at.1.saturating_sub(reach / 2),
                    )
                };
                self.act("scroll", move |c| {
                    c.swipe((at.0, from_y), (at.0, to_y), Duration::from_millis(250))
                        .map(drop)
                });
            }
        }
    }

    /// Run a daemon call off the UI thread; say `label` in the footer on success, or the error.
    fn act(
        &self,
        label: &str,
        call: impl FnOnce(Client) -> opendhu_core::Result<()> + Send + 'static,
    ) {
        let (tx, label) = (self.tx.clone(), label.to_string());
        thread::spawn(move || {
            let note = match session::client().and_then(call) {
                Ok(()) => label,
                Err(e) => format!("{label}: {e}"),
            };
            let _ = tx.send(Msg::Note(note));
        });
    }

    fn render(&mut self, out: &mut impl Write) -> io::Result<()> {
        if self
            .note
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() > NOTE_FOR)
        {
            self.note = None;
        }
        let size = term::size();
        let layout = self.layout_in(size);
        self.video.tick(layout.decode_size());

        let chrome = self.chrome(size);
        let changed = self.drawn.as_ref().map(|(s, l, _)| (*s, *l)) != Some((size, layout));
        if changed {
            out.write_all(b"\x1b[2J")?;
            self.canvas.clear(out)?;
        }
        if changed || self.drawn.as_ref().is_none_or(|(_, _, c)| *c != chrome) {
            let rows = size.rows.max(1);
            write!(out, "\x1b[1;1H\x1b[2K{}", fit(&chrome.header, size.cols))?;
            write!(
                out,
                "\x1b[{rows};1H\x1b[2K{}",
                fit(&chrome.footer, size.cols)
            )?;
            let pane = self.log_rows(size) as usize;
            for (i, line) in chrome.log.iter().enumerate() {
                let row = rows as usize - pane + i;
                write!(out, "\x1b[{row};1H\x1b[2K{}", styled(line, size.cols))?;
            }
            out.flush()?;
        }
        // At most ~30 pictures a second: more only queues bytes the terminal must parse.
        let due = self.last_draw.is_none_or(|t| t.elapsed() >= MIN_FRAME_GAP);
        if (changed || (self.frame_dirty && due))
            && let Some(frame) = self
                .frame
                .as_ref()
                .filter(|f| f.size == layout.decode_size())
        {
            self.canvas
                .draw(out, &frame.rgb, frame.size, layout.placement())?;
            self.last_draw = Some(Instant::now());
            self.frame_dirty = false;
        }
        self.drawn = Some((size, layout, chrome));
        Ok(())
    }

    /// Everything drawn around the picture, so one comparison says whether it needs redrawing.
    fn chrome(&self, size: term::Size) -> Chrome {
        let link = self.status.as_ref().map_or_else(
            || "no session; start one with `aa start`".to_string(),
            |s| s.summary.clone(),
        );
        let video = match (self.video.problem(), self.frames) {
            (Some(p), _) => p,
            (None, 0) => "waiting for video".to_string(),
            (None, n) => format!(
                "{:.0} fps",
                n as f64 / self.started.elapsed().as_secs_f64().max(1.0)
            ),
        };
        let pointer = if self.pixel_mouse {
            "pixel mouse"
        } else {
            "cell mouse"
        };
        let finished = self.log.as_ref().is_some_and(|l| l.ended);
        let footer = match (&self.note, finished) {
            (Some((n, _)), _) => n.as_str(),
            (None, true) => "test run finished; press q to close",
            (None, false) => HINTS,
        };
        let pane = self.log_rows(size) as usize;
        let log = self.log.as_ref().map_or_else(Vec::new, |l| {
            let mut tail = l.lines.iter().rev().take(pane).cloned().collect::<Vec<_>>();
            tail.reverse();
            // Pad so the pane always rewrites every row it owns.
            tail.resize(pane, String::new());
            tail
        });
        Chrome {
            header: format!(" openDHU · {link} · {video} · {pointer}"),
            footer: format!(" {footer}"),
            log,
        }
    }
}

#[derive(PartialEq)]
struct Chrome {
    header: String,
    footer: String,
    log: Vec<String>,
}

/// `text` cut to `cols` characters.
fn fit(text: &str, cols: u32) -> String {
    text.chars().take(cols as usize).collect()
}

fn key_for_char(c: char) -> Option<Key> {
    Some(match c {
        'b' => Key::Back,
        'h' => Key::Home,
        's' => Key::Search,
        ' ' => Key::PlayPause,
        'n' => Key::Next,
        'p' => Key::Previous,
        'c' => Key::Call,
        'e' => Key::EndCall,
        _ => return None,
    })
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_map_to_car_buttons() {
        assert_eq!(key_for_char('h'), Some(Key::Home));
        assert_eq!(key_for_char(' '), Some(Key::PlayPause));
        assert_eq!(key_for_char('z'), None);
    }

    #[test]
    fn header_and_footer_are_cut_to_the_terminal_width() {
        assert_eq!(fit("héllo wörld", 5), "héllo");
    }

    #[test]
    fn runner_labels_are_coloured_and_cut_to_width() {
        assert_eq!(styled("  PASS tap", 80), "\x1b[32m  PASS tap\x1b[0m");
        assert_eq!(styled("FAIL xyz", 6), "\x1b[31mFAIL x\x1b[0m");
        assert_eq!(styled("plain text", 5), "plain");
    }
}

//! The test run's progress. `aa test --web` writes it as lines to the descriptor named by
//! `OPENDHU_WEB_LOG_FD`, in the same format it gives `opendhu-tui`; every page sees the whole run
//! so far and then each new line.

use std::io::BufRead;
use std::sync::Mutex;

use serde::Serialize;
use tokio::sync::broadcast;

/// Lines kept for a page opened late; older ones are dropped, they are still in the shell.
const KEEP: usize = 5000;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    Line {
        text: String,
        kind: Kind,
    },
    /// The run is over; no more lines follow.
    End,
}

/// How the page colours a line, after the labels the test runner prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Pass,
    Fail,
    Skip,
    Run,
    Heading,
    Plain,
}

pub fn kind(line: &str) -> Kind {
    let body = line.trim_start();
    if body.starts_with("PASS ") || body == "Result: PASSED" {
        Kind::Pass
    } else if body.starts_with("FAIL ")
        || body.starts_with("ERROR ")
        || body == "Result: FAILED"
        || body == "Result: ERROR"
    {
        Kind::Fail
    } else if body.starts_with("SKIP ")
        || body.starts_with("INTERRUPTED ")
        || body == "Result: SKIPPED"
    {
        Kind::Skip
    } else if body.starts_with("RUN ") {
        Kind::Run
    } else if body.starts_with("Test:") || body.starts_with("Suite:") {
        Kind::Heading
    } else {
        Kind::Plain
    }
}

/// What a line of the feed means.
#[derive(Debug, PartialEq)]
pub enum Feed {
    /// `aa` marks control lines with a leading record separator: `\x1eruntime\t<dir>` names the
    /// runtime directory of the daemon the current test uses (empty: the default one).
    Runtime(Option<std::path::PathBuf>),
    Line(String),
}

pub fn parse(line: &str) -> Feed {
    if let Some(dir) = line.strip_prefix("\x1eruntime\t") {
        return Feed::Runtime((!dir.is_empty()).then(|| dir.into()));
    }
    Feed::Line(
        line.replace('\t', "    ")
            .chars()
            .filter(|c| !c.is_control())
            .collect(),
    )
}

pub struct Log {
    /// `None` without a test run: there is no log pane.
    state: Mutex<Option<State>>,
    tx: broadcast::Sender<Event>,
}

#[derive(Default)]
struct State {
    events: Vec<Event>,
    ended: bool,
}

impl Log {
    pub fn new(attached: bool) -> Self {
        Self {
            state: Mutex::new(attached.then(State::default)),
            tx: broadcast::channel(1024).0,
        }
    }

    pub fn attached(&self) -> bool {
        self.lock().is_some()
    }

    pub fn ended(&self) -> bool {
        self.lock().as_ref().is_some_and(|s| s.ended)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<State>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn push(&self, event: Event) {
        let mut guard = self.lock();
        let Some(state) = guard.as_mut() else {
            return;
        };
        if event == Event::End {
            state.ended = true;
        }
        state.events.push(event.clone());
        if state.events.len() > KEEP {
            state.events.drain(..KEEP / 2);
        }
        // Sent under the lock, so a page that subscribes in `follow` sees each event exactly once.
        let _ = self.tx.send(event);
    }

    /// Everything so far, and a receiver for what comes next.
    pub fn follow(&self) -> (Vec<Event>, broadcast::Receiver<Event>) {
        let guard = self.lock();
        let past = guard.as_ref().map_or_else(Vec::new, |s| s.events.clone());
        (past, self.tx.subscribe())
    }

    /// Read the feed until `aa` closes it, which is the end of the run.
    pub fn read(&self, feed: impl BufRead, mut on_runtime: impl FnMut(Option<std::path::PathBuf>)) {
        for line in feed.lines().map_while(Result::ok) {
            match parse(&line) {
                Feed::Runtime(dir) => on_runtime(dir),
                Feed::Line(text) => self.push(Event::Line {
                    kind: kind(&text),
                    text,
                }),
            }
        }
        self.push(Event::End);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_labels_get_their_colour() {
        assert_eq!(kind("  PASS 1. tap (2.0s)"), Kind::Pass);
        assert_eq!(kind("FAIL 2. expect"), Kind::Fail);
        assert_eq!(kind("Result: ERROR"), Kind::Fail);
        assert_eq!(kind("  SKIP needs touch"), Kind::Skip);
        assert_eq!(kind("RUN 3. swipe"), Kind::Run);
        assert_eq!(kind("Suite: sample (3 tests)"), Kind::Heading);
        assert_eq!(kind("PASSING remark"), Kind::Plain);
    }

    #[test]
    fn control_lines_name_the_runtime_and_text_is_cleaned() {
        assert_eq!(
            parse("\x1eruntime\t/tmp/suite"),
            Feed::Runtime(Some("/tmp/suite".into()))
        );
        assert_eq!(parse("\x1eruntime\t"), Feed::Runtime(None));
        assert_eq!(
            parse("a\tb\x1b[31mc"),
            Feed::Line("a    b[31mc".to_string())
        );
    }

    #[test]
    fn a_late_page_gets_the_whole_run_then_the_end() {
        let log = Log::new(true);
        let mut runtimes = Vec::new();
        log.read("Suite: s\n\x1eruntime\t/r\n  PASS one\n".as_bytes(), |d| {
            runtimes.push(d)
        });
        assert_eq!(runtimes, vec![Some("/r".into())]);
        assert!(log.ended());
        let (past, _) = log.follow();
        assert_eq!(
            past,
            vec![
                Event::Line {
                    text: "Suite: s".into(),
                    kind: Kind::Heading
                },
                Event::Line {
                    text: "  PASS one".into(),
                    kind: Kind::Pass
                },
                Event::End,
            ]
        );
    }

    #[test]
    fn without_a_run_nothing_is_kept() {
        let log = Log::new(false);
        log.push(Event::End);
        assert!(!log.attached() && !log.ended());
        assert!(log.follow().0.is_empty());
    }

    #[test]
    fn events_serialize_for_the_page() {
        let line = Event::Line {
            text: "RUN x".into(),
            kind: Kind::Run,
        };
        assert_eq!(
            serde_json::to_string(&line).unwrap(),
            r#"{"type":"line","text":"RUN x","kind":"run"}"#
        );
        assert_eq!(
            serde_json::to_string(&Event::End).unwrap(),
            r#"{"type":"end"}"#
        );
    }
}

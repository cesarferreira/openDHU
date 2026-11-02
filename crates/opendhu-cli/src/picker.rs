//! `aa test -i`: a terminal checklist of suites and their tests, with no UI dependency.
//! The checklist state and key decoding are pure so they can be tested without a terminal.
use std::collections::BTreeSet;
use std::io::{Read, Write};

use opendhu_core::{Error, Result};

/// One suite (or test directory) and the tests it contains, as `(file, name)`.
pub(crate) struct Group {
    pub name: String,
    pub tests: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Up,
    Down,
    Toggle,
    All,
    Confirm,
    Cancel,
}

pub(crate) fn decode(bytes: &[u8]) -> Option<Key> {
    match bytes {
        b"\x1b[A" | b"\x1bOA" | b"k" => Some(Key::Up),
        b"\x1b[B" | b"\x1bOB" | b"j" => Some(Key::Down),
        b" " => Some(Key::Toggle),
        b"a" => Some(Key::All),
        b"\r" | b"\n" => Some(Key::Confirm),
        b"\x1b" | b"\x03" | b"q" => Some(Key::Cancel),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Row {
    Suite(usize),
    Test(usize, usize),
}

pub(crate) struct Checklist {
    groups: Vec<Group>,
    rows: Vec<Row>,
    checked: Vec<Vec<bool>>,
    cursor: usize,
}

impl Checklist {
    /// Starts with everything selected, so Enter alone runs what a plain `aa test` would.
    pub(crate) fn new(groups: Vec<Group>) -> Self {
        let mut rows = Vec::new();
        for (g, group) in groups.iter().enumerate() {
            rows.push(Row::Suite(g));
            rows.extend((0..group.tests.len()).map(|t| Row::Test(g, t)));
        }
        let checked = groups.iter().map(|g| vec![true; g.tests.len()]).collect();
        Self {
            groups,
            rows,
            checked,
            cursor: 0,
        }
    }

    pub(crate) fn press(&mut self, key: Key) {
        match key {
            Key::Up => self.cursor = self.cursor.saturating_sub(1),
            Key::Down => self.cursor = (self.cursor + 1).min(self.rows.len().saturating_sub(1)),
            Key::Toggle => match self.rows[self.cursor] {
                Row::Suite(g) => {
                    let all = self.checked[g].iter().all(|c| *c);
                    self.checked[g].iter_mut().for_each(|c| *c = !all);
                }
                Row::Test(g, t) => self.checked[g][t] = !self.checked[g][t],
            },
            Key::All => {
                let all = self.checked.iter().flatten().all(|c| *c);
                self.checked.iter_mut().flatten().for_each(|c| *c = !all);
            }
            Key::Confirm | Key::Cancel => {}
        }
    }

    pub(crate) fn selected(&self) -> BTreeSet<String> {
        let mut files = BTreeSet::new();
        for (g, group) in self.groups.iter().enumerate() {
            for (t, (file, _)) in group.tests.iter().enumerate() {
                if self.checked[g][t] {
                    files.insert(file.clone());
                }
            }
        }
        files
    }

    fn render(&self, height: usize) -> Vec<String> {
        // Keep the cursor inside a window that fits above the two help lines.
        let window = height.saturating_sub(3).max(3).min(self.rows.len());
        let start = self
            .cursor
            .saturating_sub(window - 1)
            .min(self.rows.len() - window);
        let mut lines = Vec::new();
        for (i, row) in self.rows.iter().enumerate().skip(start).take(window) {
            let (indent, name, on, some) = match *row {
                Row::Suite(g) => {
                    let n = self.checked[g].iter().filter(|c| **c).count();
                    let name = format!("{} ({}/{})", self.groups[g].name, n, self.checked[g].len());
                    ("", name, n == self.checked[g].len(), n > 0)
                }
                Row::Test(g, t) => {
                    let on = self.checked[g][t];
                    ("  ", self.groups[g].tests[t].1.clone(), on, on)
                }
            };
            let mark = if on {
                "x"
            } else if some {
                "-"
            } else {
                " "
            };
            let pointer = if i == self.cursor { ">" } else { " " };
            let bold = if matches!(row, Row::Suite(_)) {
                "\x1b[1m"
            } else {
                ""
            };
            let reverse = if i == self.cursor { "\x1b[7m" } else { "" };
            lines.push(format!(
                "{pointer} {indent}{reverse}{bold}[{mark}] {name}\x1b[0m"
            ));
        }
        let total: usize = self.checked.iter().flatten().filter(|c| **c).count();
        lines.push(String::new());
        lines.push(format!(
            "\x1b[2m{total} selected  ↑/↓ move  space toggle  a all/none  enter run  q cancel\x1b[0m"
        ));
        lines
    }
}

struct RawMode(libc::termios);

impl RawMode {
    fn enter() -> Result<Self> {
        // SAFETY: plain termios calls on stdin with a zeroed, then kernel-filled, struct.
        unsafe {
            let mut term: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut term) != 0 {
                return Err(Error::InvalidArgs(
                    "-i needs an interactive terminal".into(),
                ));
            }
            let saved = term;
            libc::cfmakeraw(&mut term);
            // Keep Ctrl-C delivering a key (handled as cancel) and output post-processing on.
            term.c_oflag |= libc::OPOST;
            libc::tcsetattr(0, libc::TCSANOW, &term);
            Ok(Self(saved))
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        // SAFETY: restores the attributes captured in `enter`.
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &self.0) };
    }
}

fn terminal_height() -> usize {
    // SAFETY: TIOCGWINSZ fills a winsize; on failure it stays zeroed and we use a default.
    let rows = unsafe {
        let mut size: libc::winsize = std::mem::zeroed();
        libc::ioctl(2, libc::TIOCGWINSZ, &mut size);
        size.ws_row
    };
    if rows == 0 { 24 } else { rows as usize }
}

/// Returns the chosen test files, or `None` when the user cancelled.
pub(crate) fn pick(groups: Vec<Group>) -> Result<Option<BTreeSet<String>>> {
    let _raw = RawMode::enter()?;
    interact(Checklist::new(groups)).map_err(|e| Error::Other(format!("terminal error: {e}")))
}

fn interact(mut list: Checklist) -> std::io::Result<Option<BTreeSet<String>>> {
    let mut err = std::io::stderr();
    let mut drawn = 0;
    let result = loop {
        let lines = list.render(terminal_height());
        if drawn > 0 {
            write!(err, "\x1b[{drawn}A")?;
        }
        for line in &lines {
            write!(err, "\r\x1b[2K{line}\r\n")?;
        }
        err.flush()?;
        drawn = lines.len();
        let mut buf = [0u8; 8];
        let n = std::io::stdin().read(&mut buf)?;
        if n == 0 {
            break None;
        }
        match decode(&buf[..n]) {
            Some(Key::Confirm) => break Some(list.selected()),
            Some(Key::Cancel) => break None,
            Some(key) => list.press(key),
            None => {}
        }
    };
    // Erase the checklist so the run's own output starts on a clean screen.
    write!(err, "\x1b[{drawn}A\x1b[J")?;
    err.flush()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups() -> Vec<Group> {
        let test = |file: &str| (file.to_string(), file.to_string());
        vec![
            Group {
                name: "a".into(),
                tests: vec![test("a/1.yaml"), test("a/2.yaml")],
            },
            Group {
                name: "b".into(),
                tests: vec![test("b/1.yaml")],
            },
        ]
    }

    #[test]
    fn starts_with_everything_selected() {
        assert_eq!(Checklist::new(groups()).selected().len(), 3);
    }

    #[test]
    fn toggling_a_suite_row_clears_then_restores_its_tests() {
        let mut list = Checklist::new(groups());
        list.press(Key::Toggle);
        assert_eq!(
            list.selected().into_iter().collect::<Vec<_>>(),
            ["b/1.yaml"]
        );
        list.press(Key::Toggle);
        assert_eq!(list.selected().len(), 3);
    }

    #[test]
    fn toggling_a_test_row_changes_only_that_test() {
        let mut list = Checklist::new(groups());
        list.press(Key::Down);
        list.press(Key::Toggle);
        let selected = list.selected();
        assert!(!selected.contains("a/1.yaml"));
        assert!(selected.contains("a/2.yaml") && selected.contains("b/1.yaml"));
    }

    #[test]
    fn all_key_clears_then_selects_everything() {
        let mut list = Checklist::new(groups());
        list.press(Key::All);
        assert!(list.selected().is_empty());
        list.press(Key::All);
        assert_eq!(list.selected().len(), 3);
    }

    #[test]
    fn cursor_stays_within_the_rows() {
        let mut list = Checklist::new(groups());
        list.press(Key::Up);
        assert_eq!(list.cursor, 0);
        for _ in 0..10 {
            list.press(Key::Down);
        }
        assert_eq!(list.cursor, list.rows.len() - 1);
    }

    #[test]
    fn decodes_arrows_and_vi_keys() {
        assert_eq!(decode(b"\x1b[A"), Some(Key::Up));
        assert_eq!(decode(b"j"), Some(Key::Down));
        assert_eq!(decode(b" "), Some(Key::Toggle));
        assert_eq!(decode(b"\r"), Some(Key::Confirm));
        assert_eq!(decode(b"\x1b"), Some(Key::Cancel));
        assert_eq!(decode(b"z"), None);
    }

    #[test]
    fn render_fits_the_requested_height() {
        let mut list = Checklist::new(groups());
        list.press(Key::Down);
        assert!(list.render(6).len() <= 6);
    }
}

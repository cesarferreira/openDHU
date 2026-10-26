//! Durations from earlier runs, kept under the user's config directory, so a run can say roughly
//! how long it has left. Purely advisory: a missing, unreadable or stale file is never an error.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Recent durations kept per test or suite; the median is the estimate.
const KEEP: usize = 5;

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Timings {
    #[serde(default)]
    tests: BTreeMap<String, Vec<u64>>,
    #[serde(default)]
    setup: BTreeMap<String, Vec<u64>>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

fn median(values: &[u64]) -> Option<u64> {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted.get(sorted.len() / 2).copied()
}

fn push(map: &mut BTreeMap<String, Vec<u64>>, key: String, ms: u64) {
    let runs = map.entry(key).or_default();
    runs.push(ms);
    if runs.len() > KEEP {
        runs.remove(0);
    }
}

fn key(file: &str, name: &str) -> String {
    let file = std::fs::canonicalize(file).unwrap_or_else(|_| PathBuf::from(file));
    format!("{}::{name}", file.display())
}

fn suite_key(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

impl Timings {
    /// `opendhu/timings.json` next to the config file: `$XDG_CONFIG_HOME` or `~/.config`.
    fn location() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("opendhu").join("timings.json"))
    }

    pub(crate) fn load() -> Self {
        Self::load_from(Self::location())
    }

    fn load_from(path: Option<PathBuf>) -> Self {
        let mut timings = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str::<Self>(&text).ok())
            .unwrap_or_default();
        timings.path = path;
        timings
    }

    /// Best effort and atomic, so an interrupted run never leaves a half-written file.
    pub(crate) fn save(&self) {
        let Some(path) = &self.path else { return };
        let Ok(text) = serde_json::to_string_pretty(self) else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let temp = path.with_extension(format!("json.{}", std::process::id()));
        if std::fs::write(&temp, text).is_ok() && std::fs::rename(&temp, path).is_err() {
            let _ = std::fs::remove_file(&temp);
        }
    }

    pub(crate) fn record_test(&mut self, file: &str, name: &str, ms: u64) {
        push(&mut self.tests, key(file, name), ms);
    }

    pub(crate) fn record_setup(&mut self, suite: &Path, ms: u64) {
        push(&mut self.setup, suite_key(suite), ms);
    }

    pub(crate) fn expected_test(&self, file: &str, name: &str) -> Option<u64> {
        self.tests.get(&key(file, name)).and_then(|v| median(v))
    }

    pub(crate) fn expected_setup(&self, suite: &Path) -> Option<u64> {
        self.setup.get(&suite_key(suite)).and_then(|v| median(v))
    }
}

/// Expected total for tests whose estimates are `known`, filling unknown ones with their average.
/// `None` when nothing is known at all, so no guess is shown.
pub(crate) fn total(known: &[Option<u64>]) -> Option<u64> {
    let seen: Vec<u64> = known.iter().flatten().copied().collect();
    if seen.is_empty() {
        return None;
    }
    let average = seen.iter().sum::<u64>() / seen.len() as u64;
    Some(known.iter().map(|k| k.unwrap_or(average)).sum())
}

pub(crate) fn left(ms: u64) -> String {
    let secs = (ms + 500) / 1000;
    match secs {
        0..=44 => "under a minute".into(),
        45..=3599 => format!("~{}m", (secs + 30) / 60),
        _ => format!("~{}h{:02}m", secs / 3600, (secs % 3600 + 30) / 60 % 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_recent_runs_and_estimates_with_the_median() {
        let mut t = Timings::default();
        for ms in [10, 20, 30, 40, 50, 60, 1000] {
            push(&mut t.tests, "a".into(), ms);
        }
        assert_eq!(t.tests["a"], vec![30, 40, 50, 60, 1000]);
        assert_eq!(median(&t.tests["a"]), Some(50));
        assert_eq!(median(&[]), None);
    }

    #[test]
    fn unknown_tests_take_the_average_and_no_data_gives_no_guess() {
        assert_eq!(total(&[Some(100), None, Some(300)]), Some(600));
        assert_eq!(total(&[None, None]), None);
        assert_eq!(total(&[]), None);
    }

    #[test]
    fn saves_and_reloads_atomically() {
        let dir = std::env::temp_dir().join(format!("aa-timings-{}", std::process::id()));
        let path = dir.join("opendhu").join("timings.json");
        let mut t = Timings::load_from(Some(path.clone()));
        t.record_setup(Path::new("/nonexistent/suite.yaml"), 1234);
        t.save();
        let again = Timings::load_from(Some(path));
        assert_eq!(
            again.expected_setup(Path::new("/nonexistent/suite.yaml")),
            Some(1234)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_corrupt_file_is_ignored() {
        let dir = std::env::temp_dir().join(format!("aa-timings-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("timings.json");
        std::fs::write(&path, "not json").unwrap();
        let t = Timings::load_from(Some(path));
        assert_eq!(t.expected_setup(Path::new("/x")), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn formats_time_left() {
        assert_eq!(left(20_000), "under a minute");
        assert_eq!(left(5 * 60_000), "~5m");
        assert_eq!(left(3_900_000), "~1h05m");
    }
}

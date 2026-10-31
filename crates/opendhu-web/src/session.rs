//! Which daemon the viewer talks to. A managed suite runs each test against its own daemon in its
//! own runtime directory, so `aa test --web` tells the viewer where the current one is.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use opendhu_core::Result;
use opendhu_daemon::Client;

static RUNTIME: Mutex<Option<PathBuf>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Point at another runtime directory (`None`: the default one). Returns whether it changed.
pub fn set(dir: Option<PathBuf>) -> bool {
    let mut current = RUNTIME.lock().unwrap_or_else(|e| e.into_inner());
    if *current == dir {
        return false;
    }
    *current = dir;
    GENERATION.fetch_add(1, Ordering::SeqCst);
    true
}

/// Changes whenever `set` changes the directory, so a stream can tell it has gone stale.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

pub fn client() -> Result<Client> {
    let dir = RUNTIME.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match dir {
        Some(dir) => Client::connect_in(&dir),
        None => Client::connect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_different_directory_is_a_change() {
        let start = generation();
        assert!(set(Some("/tmp/a-web-suite-runtime".into())));
        assert!(!set(Some("/tmp/a-web-suite-runtime".into())));
        assert_eq!(generation(), start + 1);
        assert!(set(None));
    }
}

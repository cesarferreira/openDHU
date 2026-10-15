#![allow(dead_code)]
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static N: AtomicUsize = AtomicUsize::new(0);

pub fn fake_dhu() -> String {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../autolab-dhu/tests/fixtures/fake-dhu.sh"
    )
    .to_string()
}

/// An isolated runtime dir plus helpers. Stops any session it started, even on test failure.
pub struct Sandbox {
    pub dir: PathBuf,
}

impl Sandbox {
    pub fn new() -> Self {
        let dir = PathBuf::from(format!(
            "/tmp/al-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        Self { dir }
    }

    pub fn autolab(&self, args: &[&str]) -> Output {
        self.autolab_env(args, &[])
    }

    pub fn autolab_env(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_autolab"));
        c.args(args)
            .env("AUTOLAB_RUNTIME_DIR", &self.dir)
            .env_remove("AUTOLAB_DHU_PATH");
        for (k, v) in env {
            c.env(k, v);
        }
        c.output().expect("run autolab")
    }

    pub fn start_fake(&self) -> Output {
        self.autolab(&["start", "--dhu-path", &fake_dhu(), "--serial", "FAKE0001"])
    }

    pub fn status_json(&self) -> serde_json::Value {
        let o = self.autolab(&["status", "--json"]);
        assert_eq!(code(&o), 0, "{}", text(&o));
        serde_json::from_slice(&o.stdout).unwrap()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = Command::new(env!("CARGO_BIN_EXE_autolab"))
            .arg("stop")
            .env("AUTOLAB_RUNTIME_DIR", &self.dir)
            .output();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub fn code(o: &Output) -> i32 {
    o.status.code().unwrap_or(-1)
}

pub fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

pub fn alive(pid: u64) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .unwrap()
        .status
        .success()
}

pub fn wait_until(limit: Duration, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    f()
}

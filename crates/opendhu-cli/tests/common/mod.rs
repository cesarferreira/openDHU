#![allow(dead_code)]
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static N: AtomicUsize = AtomicUsize::new(0);

pub fn fake_adb_dir() -> String {
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/bin").to_string()
}

pub fn fake_dhu() -> String {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../opendhu-dhu/tests/fixtures/fake-dhu.sh"
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

    pub fn opendhu(&self, args: &[&str]) -> Output {
        self.opendhu_env(args, &[])
    }

    pub fn opendhu_env(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_aa"));
        c.args(args)
            .env("OPENDHU_RUNTIME_DIR", &self.dir)
            // Never read the developer's real config file.
            .env("XDG_CONFIG_HOME", self.dir.join("no-xdg"))
            .env_remove("OPENDHU_CONFIG")
            .env_remove("OPENDHU_DHU_PATH");
        for (k, v) in env {
            c.env(k, v);
        }
        c.output().expect("run opendhu")
    }

    pub fn start_fake(&self) -> Output {
        self.opendhu(&[
            "session",
            "start",
            "--dhu",
            "--dhu-path",
            &fake_dhu(),
            "--serial",
            "FAKE0001",
        ])
    }

    /// Env that puts the fake adb first on PATH, with its state kept next to the sandbox.
    pub fn adb_env(&self, mode: &str) -> Vec<(String, String)> {
        let state = self.adb_state();
        std::fs::create_dir_all(&state).unwrap();
        vec![
            ("FAKE_ADB_STATE".into(), state.display().to_string()),
            ("FAKE_ADB_MODE".into(), mode.into()),
            (
                "PATH".into(),
                format!(
                    "{}:{}",
                    fake_adb_dir(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            ),
        ]
    }

    pub fn adb_state(&self) -> PathBuf {
        PathBuf::from(format!("{}-adb", self.dir.display()))
    }

    pub fn adb_calls(&self) -> String {
        std::fs::read_to_string(self.adb_state().join("calls.log")).unwrap_or_default()
    }

    pub fn opendhu_with(&self, args: &[&str], env: &[(String, String)]) -> Output {
        let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        self.opendhu_env(args, &pairs)
    }

    /// A scratch path next to the runtime dir, removed on drop.
    pub fn scratch(&self, name: &str) -> PathBuf {
        PathBuf::from(format!("{}-{name}", self.dir.display()))
    }

    pub fn status_json(&self) -> serde_json::Value {
        let o = self.opendhu(&["session", "status", "--json"]);
        assert_eq!(code(&o), 0, "{}", text(&o));
        serde_json::from_slice(&o.stdout).unwrap()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = Command::new(env!("CARGO_BIN_EXE_aa"))
            .args(["session", "stop"])
            .env("OPENDHU_RUNTIME_DIR", &self.dir)
            .output();
        let _ = std::fs::remove_dir_all(&self.dir);
        // Everything a test put next to the runtime dir: `-adb`, `-shot.png`, `-art`, `-tests`.
        if let (Some(parent), Some(name)) = (self.dir.parent(), self.dir.file_name()) {
            let prefix = format!("{}-", name.to_string_lossy());
            for e in std::fs::read_dir(parent).into_iter().flatten().flatten() {
                if e.file_name().to_string_lossy().starts_with(&prefix) {
                    let p = e.path();
                    let _ = if p.is_dir() {
                        std::fs::remove_dir_all(&p)
                    } else {
                        std::fs::remove_file(&p)
                    };
                }
            }
        }
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

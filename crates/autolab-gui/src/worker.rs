//! Background threads. The window never blocks: it sends a `Cmd` and later receives an `Update`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use autolab_daemon::Client;
use eframe::egui;

use crate::model::{Conn, Update, log_line};

const POLL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub enum Cmd {
    Start {
        native: bool,
        serial: Option<String>,
        reverse: Vec<u16>,
        headless: bool,
    },
    Stop,
    Focus(bool),
    Screenshot(PathBuf),
    Tap(u32, u32),
    Key(autolab_core::Key, bool),
    Swipe {
        from: (u32, u32),
        to: (u32, u32),
        duration_ms: u64,
    },
}

fn send(tx: &mpsc::Sender<Update>, ctx: &egui::Context, u: Update) -> bool {
    let ok = tx.send(u).is_ok();
    ctx.request_repaint();
    ok
}

/// Poll the daemon's status once a second, and keep an event stream open while it is reachable.
pub fn spawn_poller(ctx: egui::Context, tx: mpsc::Sender<Update>) {
    thread::spawn(move || {
        let stream_alive = Arc::new(AtomicBool::new(false));
        let mut client: Option<Client> = None;
        loop {
            if client.is_none() {
                client = Client::connect().ok();
            }
            let conn = match client.as_ref().map(Client::status) {
                Some(Ok(status)) => Conn::Connected(Box::new(status)),
                _ => {
                    client = None;
                    Conn::Disconnected
                }
            };
            if let (Some(c), false) = (&client, stream_alive.load(Ordering::SeqCst)) {
                spawn_stream(
                    c.clone(),
                    ctx.clone(),
                    tx.clone(),
                    Arc::clone(&stream_alive),
                );
            }
            if !send(&tx, &ctx, Update::Conn(conn)) {
                return; // the window is gone
            }
            thread::sleep(POLL);
        }
    });
}

fn spawn_stream(
    client: Client,
    ctx: egui::Context,
    tx: mpsc::Sender<Update>,
    alive: Arc<AtomicBool>,
) {
    alive.store(true, Ordering::SeqCst);
    thread::spawn(move || {
        if let Ok(stream) = client.events() {
            for message in stream {
                let Ok(m) = message else { break };
                // `stream.opened` has no timestamp or payload worth showing.
                if m.event == "stream.opened" {
                    continue;
                }
                let ts = m.data["timestamp"].as_str().unwrap_or("");
                let line = log_line(ts, &m.event, &m.data["data"]);
                if !send(&tx, &ctx, Update::Event(line)) {
                    break;
                }
            }
        }
        alive.store(false, Ordering::SeqCst);
    });
}

/// Run one command on its own thread so a slow start never freezes the window or the polling.
// Tests record commands instead of running them (see `ui::dispatch`), so only the real build calls this.
#[cfg_attr(test, allow(dead_code))]
pub fn run(cmd: Cmd, ctx: egui::Context, tx: mpsc::Sender<Update>) {
    thread::spawn(move || {
        let result = execute(&cmd, &ctx, &tx);
        send(&tx, &ctx, Update::Done(result));
    });
}

/// A touch from the live picture. Unlike a button press it does not hold the buttons disabled, so
/// several can follow each other, and it reports only through the notice line.
#[cfg_attr(test, allow(dead_code))]
pub fn run_touch(cmd: Cmd, ctx: egui::Context, tx: mpsc::Sender<Update>) {
    thread::spawn(move || {
        let result = execute(&cmd, &ctx, &tx);
        let (text, is_error) = match result {
            Ok(t) => (t, false),
            Err(t) => (t, true),
        };
        send(
            &tx,
            &ctx,
            Update::Notice(crate::model::Notice { text, is_error }),
        );
    });
}

fn execute(cmd: &Cmd, ctx: &egui::Context, tx: &mpsc::Sender<Update>) -> Result<String, String> {
    match cmd {
        Cmd::Start {
            native,
            serial,
            reverse,
            headless,
        } => start_with_cli(*native, serial.as_deref(), reverse, *headless),
        Cmd::Stop => {
            Client::connect()
                .and_then(|c| c.stop())
                .map_err(|e| e.to_string())?;
            Ok("Session stopped".into())
        }
        Cmd::Focus(on) => {
            use autolab_core::OpStatus;
            use autolab_daemon::api::BackendKind;
            let client = Client::connect().map_err(|e| e.to_string())?;
            let r = client.focus_video(*on).map_err(|e| e.to_string())?;
            let word = if *on { "on" } else { "off" };
            match (r.backend, r.result.status) {
                (_, OpStatus::Failed | OpStatus::Unsupported) => {
                    Err(format!("Video focus {word} failed ({})", r.operation_id))
                }
                (_, OpStatus::TimedOut) => Err(format!(
                    "Video focus {word}: the phone did not confirm in time ({})",
                    r.operation_id
                )),
                (_, OpStatus::Observed) => Ok(format!(
                    "Video focus {word}: confirmed by the phone ({})",
                    r.operation_id
                )),
                // Accepted means the head unit took it. The DHU cannot hear from the phone.
                (BackendKind::Dhu, _) => Ok(format!(
                    "Video focus {word}: accepted by the DHU, not confirmed by the phone ({})",
                    r.operation_id
                )),
                (BackendKind::Native, _) => {
                    Ok(format!("Video focus {word}: sent ({})", r.operation_id))
                }
            }
        }
        Cmd::Tap(x, y) => {
            let client = Client::connect().map_err(|e| e.to_string())?;
            client.tap(*x, *y).map_err(|e| e.to_string())?;
            // The phone never acknowledges touch, so say only that it was sent.
            Ok(format!("Tap sent at ({x}, {y})"))
        }
        Cmd::Key(key, long) => {
            let client = Client::connect().map_err(|e| e.to_string())?;
            client.key(*key, *long).map_err(|e| e.to_string())?;
            Ok(format!("{} pressed", key.name()))
        }
        Cmd::Swipe {
            from,
            to,
            duration_ms,
        } => {
            let client = Client::connect().map_err(|e| e.to_string())?;
            client
                .swipe(*from, *to, Duration::from_millis(*duration_ms))
                .map_err(|e| e.to_string())?;
            Ok(format!(
                "Swipe sent ({}, {}) to ({}, {})",
                from.0, from.1, to.0, to.1
            ))
        }
        Cmd::Screenshot(path) => {
            let client = Client::connect().map_err(|e| e.to_string())?;
            let r = client.screenshot(path).map_err(|e| e.to_string())?;
            let (w, h) = (r.result.width as usize, r.result.height as usize);
            match load_texture(ctx, &r.result.path) {
                Ok(texture) => {
                    send(
                        tx,
                        ctx,
                        Update::Screenshot {
                            path: r.result.path.clone(),
                            width: w,
                            height: h,
                            texture,
                        },
                    );
                }
                Err(e) => return Err(format!("Saved, but cannot preview it: {e}")),
            }
            Ok(format!(
                "Screenshot saved: {} ({w}x{h})",
                r.result.path.display()
            ))
        }
    }
}

fn load_texture(ctx: &egui::Context, path: &Path) -> Result<egui::TextureHandle, String> {
    let img = image::open(path).map_err(|e| e.to_string())?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Ok(ctx.load_texture("screenshot", color, egui::TextureOptions::LINEAR))
}

/// `aa` next to this executable, or on `PATH`. `aa start` owns daemon start-up and the
/// config file, so the window does not repeat that logic.
fn cli_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("aa")))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("aa"))
}

fn start_with_cli(
    native: bool,
    serial: Option<&str>,
    reverse: &[u16],
    headless: bool,
) -> Result<String, String> {
    let mut cmd = Command::new(cli_path());
    cmd.arg("start");
    if native {
        cmd.args(["--backend", "native"]);
    }
    if let Some(s) = serial.filter(|s| !s.is_empty()) {
        cmd.args(["--serial", s]);
    }
    for p in reverse {
        cmd.args(["--reverse", &p.to_string()]);
    }
    if headless {
        cmd.arg("--headless");
    }
    let out = cmd
        .output()
        .map_err(|e| format!("cannot run `aa start`: {e}"))?;
    if out.status.success() {
        Ok("Session started".into())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("aa start failed");
        Err(line.trim_start_matches("error: ").to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autolab_daemon::ServeOptions;
    use autolab_daemon::api::StartRequest;

    /// The window's commands against a real in-process daemon and the scripted fake DHU. The runtime
    /// directory comes from the environment, so this is one test: nothing else may change it.
    #[test]
    fn commands_drive_a_real_daemon_and_report_the_dhus_refusals() {
        let dir = PathBuf::from(format!("/tmp/al-gui-t-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: this is the only test in the crate that touches the environment.
        unsafe {
            std::env::set_var("AUTOLAB_RUNTIME_DIR", &dir);
            // The fake DHU then writes a complete PNG, so the preview can decode it.
            std::env::set_var("FAKE_DHU_MODE", "screenshot-valid");
        }
        let d = dir.clone();
        let daemon = thread::spawn(move || {
            autolab_daemon::run(ServeOptions {
                runtime_dir: d,
                port: Some(0),
                exit_when_idle: false,
                idle_grace: None,
            })
        });
        let end = std::time::Instant::now() + Duration::from_secs(10);
        while Client::connect_in(&dir).is_err() {
            assert!(std::time::Instant::now() < end, "daemon never came up");
            thread::sleep(Duration::from_millis(25));
        }
        let client = Client::connect_in(&dir).unwrap();
        client
            .start(&StartRequest {
                dhu_path: Some(
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("../autolab-dhu/tests/fixtures/fake-dhu.sh"),
                ),
                serial: Some("FAKE0001".into()),
                ..Default::default()
            })
            .unwrap();

        let ctx = egui::Context::default();
        let (tx, rx) = mpsc::channel();

        let off = execute(&Cmd::Focus(false), &ctx, &tx).unwrap();
        assert!(
            off.contains("off: accepted by the DHU, not confirmed by the phone"),
            "{off}"
        );

        let shot = dir
            .parent()
            .unwrap()
            .join(format!("al-gui-t-{}.png", std::process::id()));
        let on = execute(&Cmd::Focus(true), &ctx, &tx).unwrap();
        assert!(on.contains("on: accepted by the DHU"), "{on}");
        let saved = execute(&Cmd::Screenshot(shot.clone()), &ctx, &tx).unwrap();
        assert!(saved.contains("1280x720"), "{saved}");
        match rx.try_recv().unwrap() {
            Update::Screenshot {
                width,
                height,
                path,
                ..
            } => {
                assert_eq!((width, height), (1280, 720));
                assert_eq!(path, shot);
            }
            _ => panic!("expected a screenshot update"),
        }

        // The DHU refuses screenshots without focus; the window must show its words.
        execute(&Cmd::Focus(false), &ctx, &tx).unwrap();
        let refused = execute(&Cmd::Screenshot(shot.clone()), &ctx, &tx).unwrap_err();
        assert!(refused.contains("nothing to screenshot"), "{refused}");

        assert_eq!(execute(&Cmd::Stop, &ctx, &tx).unwrap(), "Session stopped");
        assert!(
            execute(&Cmd::Focus(true), &ctx, &tx).is_err(),
            "no session after stop"
        );
        client.shutdown().unwrap();
        daemon.join().unwrap().unwrap();
        let _ = std::fs::remove_file(&shot);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

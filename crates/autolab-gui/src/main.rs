//! `autolab-gui`: a desktop window onto the AutoLab daemon. It uses the same HTTP API as the CLI.

mod live;
mod model;
mod sound;
mod ui;
mod worker;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui;
use model::State;

/// Set to a `.png` path to save an image of the window after a short wait, then quit. Used to check
/// the rendered window without a screen recorder, so nothing else on the desktop is captured.
const SNAPSHOT_ENV: &str = "AUTOLAB_GUI_SNAPSHOT";
const SNAPSHOT_AFTER: Duration = Duration::from_millis(2500);
/// Overrides the wait, for a window that needs longer to fill, such as the live picture.
const SNAPSHOT_AFTER_ENV: &str = "AUTOLAB_GUI_SNAPSHOT_AFTER_MS";

struct App {
    state: State,
    form: ui::Form,
    tx: mpsc::Sender<model::Update>,
    rx: mpsc::Receiver<model::Update>,
    live: live::Live,
    sound: sound::Sound,
    started: Instant,
    snapshot_to: Option<std::path::PathBuf>,
    snapshot_requested: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = mpsc::channel();
        worker::spawn_poller(cc.egui_ctx.clone(), tx.clone());
        Self {
            state: State::new(),
            form: ui::Form::new(),
            tx,
            rx,
            live: live::Live::new(),
            sound: sound::Sound::new(),
            started: Instant::now(),
            snapshot_to: std::env::var_os(SNAPSHOT_ENV).map(Into::into),
            snapshot_requested: false,
        }
    }

    fn snapshot_step(&mut self, ctx: &egui::Context) {
        let Some(path) = self.snapshot_to.clone() else {
            return;
        };
        let wait = std::env::var(SNAPSHOT_AFTER_ENV)
            .ok()
            .and_then(|v| v.parse().ok())
            .map_or(SNAPSHOT_AFTER, Duration::from_millis);
        if !self.snapshot_requested && self.started.elapsed() >= wait {
            self.snapshot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = image {
            let [w, h] = img.size;
            let saved = image::save_buffer(
                &path,
                img.as_raw(),
                w as u32,
                h as u32,
                image::ColorType::Rgba8,
            );
            if let Err(e) = saved {
                eprintln!("cannot save the snapshot: {e}");
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint_after(Duration::from_millis(200));
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(update) = self.rx.try_recv() {
            self.state.apply(update);
        }
        // The live picture runs only while a native session does and the box is ticked.
        let want = self.state.native_running();
        self.live.tick(ctx, want && self.form.live);
        if let Some(img) = self.live.take_frame() {
            match &mut self.state.live_texture {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => {
                    self.state.live_texture =
                        Some(ctx.load_texture("live", img, egui::TextureOptions::LINEAR));
                }
            }
        }
        if !want {
            self.state.live_texture = None;
        }
        // Audio is opt-in: it plays sound, and the phone may be playing something private.
        self.sound.tick(want && self.form.audio);
        self.state.audio_summary = self.form.audio.then(|| self.sound.summary());
        self.state.live_summary = self.live.summary();
        self.state.live_problem = self
            .live
            .problem
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        ui::draw(ctx, &mut self.state, &mut self.form, &self.tx);
        self.snapshot_step(ctx);
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("AutoLab")
            .with_inner_size([1000.0, 640.0])
            .with_min_inner_size([720.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "AutoLab",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

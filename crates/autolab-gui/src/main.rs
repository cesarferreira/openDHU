//! `autolab-gui`: a desktop window onto the AutoLab daemon. It uses the same HTTP API as the CLI.

mod model;
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

struct App {
    state: State,
    form: ui::Form,
    tx: mpsc::Sender<model::Update>,
    rx: mpsc::Receiver<model::Update>,
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
            started: Instant::now(),
            snapshot_to: std::env::var_os(SNAPSHOT_ENV).map(Into::into),
            snapshot_requested: false,
        }
    }

    fn snapshot_step(&mut self, ctx: &egui::Context) {
        let Some(path) = self.snapshot_to.clone() else {
            return;
        };
        if !self.snapshot_requested && self.started.elapsed() >= SNAPSHOT_AFTER {
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

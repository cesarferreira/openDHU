//! Drawing. All decisions come from `State`; this file only lays them out.

use std::sync::mpsc;

use autolab_core::FocusState;
use autolab_daemon::api::{Phase, SessionStatus, TunnelState};
use eframe::egui::{self, Color32, RichText};

use crate::model::{Conn, State};
use crate::worker::{self, Cmd};

/// What the user typed. Kept apart from `State` because only the window edits it.
pub struct Form {
    pub serial: String,
    pub reverse: String,
    pub headless: bool,
    pub shot_path: String,
}

impl Form {
    pub fn new() -> Self {
        let home = std::env::var("HOME")
            .ok()
            .filter(|h| !h.contains(char::is_whitespace));
        let dir = home.unwrap_or_else(|| std::env::temp_dir().display().to_string());
        Self {
            serial: String::new(),
            reverse: String::new(),
            headless: false,
            shot_path: format!("{}/autolab-screenshot.png", dir.trim_end_matches('/')),
        }
    }
}

impl Default for Form {
    fn default() -> Self {
        Self::new()
    }
}

const OK: Color32 = Color32::from_rgb(70, 170, 100);
const WARN: Color32 = Color32::from_rgb(210, 160, 50);
const BAD: Color32 = Color32::from_rgb(210, 80, 80);
const DIM: Color32 = Color32::from_gray(140);

fn focus_word(f: FocusState) -> &'static str {
    match f {
        FocusState::On => "on",
        FocusState::Off => "off",
        FocusState::Unknown => "unknown",
    }
}

fn pill(ui: &mut egui::Ui, text: &str, color: Color32) {
    // The default font has no "●" glyph, so the dot is drawn.
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 5.0, color);
        ui.label(RichText::new(text).color(color).strong());
    });
}

fn card(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong().size(15.0));
            ui.add_space(4.0);
            body(ui);
        });
    ui.add_space(6.0);
}

fn row(ui: &mut egui::Ui, key: &str, value: impl Into<String>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{key:<15}")).color(DIM).monospace());
        ui.label(value.into());
    });
}

pub fn draw(
    ctx: &egui::Context,
    state: &mut State,
    form: &mut Form,
    tx: &mpsc::Sender<crate::model::Update>,
) {
    let controls = state.controls();

    egui::TopBottomPanel::top("top").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading("AutoLab");
            ui.add_space(12.0);
            match (&state.conn, state.phase()) {
                (Conn::Disconnected, _) => pill(ui, "daemon not running", DIM),
                (_, None) => pill(ui, "daemon up, no session", WARN),
                (_, Some(Phase::Starting)) => pill(ui, "starting", WARN),
                (_, Some(Phase::Running)) => pill(ui, "session running", OK),
                (_, Some(Phase::Stopping)) => pill(ui, "stopping", WARN),
            }
        });
    });

    egui::TopBottomPanel::bottom("notice").show(ctx, |ui| match &state.notice {
        Some(n) => {
            let color = if n.is_error {
                BAD
            } else {
                ui.visuals().text_color()
            };
            ui.label(RichText::new(&n.text).color(color));
        }
        None => {
            ui.label(RichText::new("Ready").color(DIM));
        }
    });

    egui::SidePanel::right("log")
        .default_width(340.0)
        .show(ctx, |ui| {
            ui.label(RichText::new("Events").strong().size(15.0));
            ui.separator();
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if state.log.is_empty() {
                        ui.label(
                            RichText::new("Nothing yet. Events appear while a daemon is up.")
                                .color(DIM),
                        );
                    }
                    for l in &state.log {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&l.time).color(DIM).monospace());
                            let color = if l.kind.ends_with("failed") || l.kind.ends_with("ended") {
                                BAD
                            } else {
                                ui.visuals().text_color()
                            };
                            ui.label(RichText::new(&l.kind).color(color).monospace());
                            ui.label(RichText::new(&l.detail).color(DIM).monospace());
                        });
                    }
                });
        });

    egui::CentralPanel::default().show(ctx, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            let session: Option<SessionStatus> = match &state.conn {
                Conn::Connected(s) => s.session.clone(),
                Conn::Disconnected => None,
            };
            session_card(ui, session.as_ref(), controls.can_stop, tx, ctx, state);
            if session.is_none() {
                start_card(ui, form, controls.can_start, tx, ctx, state);
            }
            focus_card(ui, session.as_ref(), controls.can_focus, tx, ctx, state);
            screenshot_card(ui, form, controls.can_focus, tx, ctx, state);
        });
    });
}

fn dispatch(
    cmd: Cmd,
    what: &str,
    tx: &mpsc::Sender<crate::model::Update>,
    ctx: &egui::Context,
    state: &mut State,
) {
    state.begin_command(what);
    worker::run(cmd, ctx.clone(), tx.clone());
}

fn session_card(
    ui: &mut egui::Ui,
    session: Option<&SessionStatus>,
    can_stop: bool,
    tx: &mpsc::Sender<crate::model::Update>,
    ctx: &egui::Context,
    state: &mut State,
) {
    card(ui, "Session", |ui| {
        let Some(s) = session else {
            ui.label(RichText::new("No session.").color(DIM));
            return;
        };
        row(ui, "session", s.session_id.as_str());
        let Some(d) = &s.details else {
            row(ui, "phase", format!("{:?}", s.phase).to_lowercase());
            return;
        };
        let h = &d.head_unit;
        row(
            ui,
            "backend",
            format!(
                "{} {} (build {})",
                h.backend,
                h.backend_version.as_deref().unwrap_or("?"),
                h.backend_build.as_deref().unwrap_or("?")
            ),
        );
        row(
            ui,
            "phone",
            format!(
                "{} via {}",
                h.serial.as_deref().unwrap_or("unknown"),
                d.transport
            ),
        );
        row(
            ui,
            "processes",
            format!("daemon {}, dhu {}", d.daemon_pid, d.dhu_pid),
        );
        for t in &d.tunnels {
            let color = match t.state {
                TunnelState::Active => OK,
                TunnelState::Missing => BAD,
                _ => WARN,
            };
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{:<15}", "tunnel"))
                        .color(DIM)
                        .monospace(),
                );
                ui.label(
                    RichText::new(format!("tcp:{} {:?}", t.port, t.state).to_lowercase())
                        .color(color),
                );
                if t.reapplied > 0 {
                    ui.label(RichText::new(format!("(re-applied {}x)", t.reapplied)).color(DIM));
                }
            });
        }
        ui.add_space(6.0);
        if ui
            .add_enabled(can_stop, egui::Button::new("Stop session"))
            .clicked()
        {
            dispatch(Cmd::Stop, "Stopping the session", tx, ctx, state);
        }
    });
}

fn start_card(
    ui: &mut egui::Ui,
    form: &mut Form,
    can_start: bool,
    tx: &mpsc::Sender<crate::model::Update>,
    ctx: &egui::Context,
    state: &mut State,
) {
    card(ui, "Start a session", |ui| {
        ui.horizontal(|ui| {
            ui.label("Serial");
            ui.add(
                egui::TextEdit::singleline(&mut form.serial).hint_text("only authorized device"),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Reverse ports");
            ui.add(egui::TextEdit::singleline(&mut form.reverse).hint_text("e.g. 10101, 8080"));
        });
        ui.checkbox(&mut form.headless, "Run the DHU without its window");
        ui.add_space(4.0);
        if ui
            .add_enabled(can_start, egui::Button::new("Start session"))
            .clicked()
        {
            match crate::model::parse_ports(&form.reverse) {
                Ok(reverse) => {
                    let serial = Some(form.serial.trim().to_string()).filter(|s| !s.is_empty());
                    dispatch(
                        Cmd::Start {
                            serial,
                            reverse,
                            headless: form.headless,
                        },
                        "Starting the session (about 5 s)",
                        tx,
                        ctx,
                        state,
                    );
                }
                Err(e) => {
                    state.notice = Some(crate::model::Notice {
                        text: e,
                        is_error: true,
                    });
                }
            }
        }
    });
}

fn focus_card(
    ui: &mut egui::Ui,
    session: Option<&SessionStatus>,
    can_focus: bool,
    tx: &mpsc::Sender<crate::model::Update>,
    ctx: &egui::Context,
    state: &mut State,
) {
    card(ui, "Video focus", |ui| {
        let v = session
            .and_then(|s| s.details.as_ref())
            .map(|d| &d.head_unit.video_focus);
        match v {
            Some(v) => {
                let native = session
                    .and_then(|s| s.details.as_ref())
                    .is_some_and(|d| d.head_unit.backend == "native");
                row(ui, "requested", focus_word(v.requested));
                row(
                    ui,
                    if native {
                        "head unit sent"
                    } else {
                        "DHU reports"
                    },
                    focus_word(v.backend_reported),
                );
                // Only the native backend hears from the phone; the DHU cannot.
                let phone = match (native, v.observed) {
                    (false, _) => "unknown (the DHU gives no phone-side signal)".to_string(),
                    (true, o) => focus_word(o).to_string(),
                };
                row(ui, "phone says", phone);
            }
            None => {
                ui.label(RichText::new("Needs a running session.").color(DIM));
            }
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(can_focus, egui::Button::new("Focus on"))
                .clicked()
            {
                dispatch(Cmd::Focus(true), "Taking video focus", tx, ctx, state);
            }
            if ui
                .add_enabled(can_focus, egui::Button::new("Focus off"))
                .clicked()
            {
                dispatch(Cmd::Focus(false), "Releasing video focus", tx, ctx, state);
            }
        });
    });
}

fn screenshot_card(
    ui: &mut egui::Ui,
    form: &mut Form,
    can_shoot: bool,
    tx: &mpsc::Sender<crate::model::Update>,
    ctx: &egui::Context,
    state: &mut State,
) {
    card(ui, "Screenshot", |ui| {
        ui.horizontal(|ui| {
            ui.label("Save to");
            ui.add(egui::TextEdit::singleline(&mut form.shot_path).desired_width(f32::INFINITY));
        });
        if ui
            .add_enabled(can_shoot, egui::Button::new("Take screenshot"))
            .clicked()
        {
            dispatch(
                Cmd::Screenshot(form.shot_path.trim().into()),
                "Taking a screenshot",
                tx,
                ctx,
                state,
            );
        }
        if let Some(shot) = &state.shot {
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!(
                    "{} ({}x{})",
                    shot.path.display(),
                    shot.width,
                    shot.height
                ))
                .color(DIM),
            );
            let width = ui.available_width().min(shot.width as f32);
            let height = width * shot.height as f32 / shot.width.max(1) as f32;
            ui.add(egui::Image::new((
                shot.texture.id(),
                egui::vec2(width, height),
            )));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LogLine, Notice, Update};
    use autolab_daemon::api::{DaemonInfo, StatusResponse};

    fn frame(state: &mut State, form: &mut Form) {
        let ctx = egui::Context::default();
        let (tx, _rx) = mpsc::channel();
        // Two frames: the first lays widgets out, the second uses those sizes.
        for _ in 0..2 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| draw(ctx, state, form, &tx));
        }
    }

    #[test]
    fn every_connection_state_draws_without_panicking() {
        let mut form = Form::new();
        let mut s = State::new();
        frame(&mut s, &mut form); // no daemon
        s.conn = Conn::Connected(Box::new(StatusResponse {
            daemon: DaemonInfo {
                version: "0".into(),
                pid: 1,
                port: 1,
                exit_when_idle: false,
            },
            session: None,
        }));
        frame(&mut s, &mut form); // daemon, no session
        s.notice = Some(Notice {
            text: "boom".into(),
            is_error: true,
        });
        s.apply(Update::Event(LogLine {
            time: "09:00:00".into(),
            kind: "session.ended".into(),
            detail: "reason x".into(),
        }));
        frame(&mut s, &mut form);
    }

    #[test]
    fn a_screenshot_preview_draws() {
        let ctx = egui::Context::default();
        let texture = ctx.load_texture(
            "t",
            egui::ColorImage::filled([4, 2], Color32::RED),
            egui::TextureOptions::LINEAR,
        );
        let mut s = State::new();
        s.apply(Update::Screenshot {
            path: "/x.png".into(),
            width: 4,
            height: 2,
            texture,
        });
        frame(&mut s, &mut Form::new());
    }

    #[test]
    fn the_default_screenshot_path_is_absolute_png_without_whitespace() {
        let f = Form::new();
        assert!(f.shot_path.starts_with('/') && f.shot_path.ends_with(".png"));
        assert!(!f.shot_path.contains(char::is_whitespace));
    }
}

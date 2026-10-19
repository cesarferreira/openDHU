//! The native viewer: connection bar, projection, controls, and optional diagnostics.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Color32, RichText};
use egui_phosphor::regular as icon;
use opendhu_core::FocusState;
use opendhu_daemon::api::{Phase, SessionStatus, TunnelState};

use crate::model::{Conn, State};
use crate::worker::Cmd;

pub struct Form {
    pub serial: String,
    pub reverse: String,
    pub native: bool,
    pub headless: bool,
    pub live: bool,
    pub long_press: bool,
    pub audio: bool,
    pub drag: Option<((u32, u32), std::time::Instant)>,
    pub screenshot_dir: String,
    pub diagnostics: bool,
    pub settings: bool,
}

impl Form {
    pub fn new() -> Self {
        let home = std::env::var("HOME")
            .ok()
            .filter(|h| !h.contains(char::is_whitespace));
        let dir = home.unwrap_or_else(|| std::env::temp_dir().display().to_string());
        let mut form = Self {
            serial: String::new(),
            reverse: String::new(),
            native: true,
            headless: false,
            live: true,
            long_press: false,
            audio: false,
            drag: None,
            screenshot_dir: dir,
            diagnostics: false,
            settings: false,
        };
        if let Some(path) = preferences_path()
            && let Ok(bytes) = std::fs::read(path)
            && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
            && let Some(dir) = value["screenshotDirectory"].as_str()
        {
            form.screenshot_dir = dir.to_owned();
        }
        form
    }

    pub fn screenshot_path(&self) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Path::new(self.screenshot_dir.trim()).join(format!("opendhu-{stamp}.png"))
    }

    pub fn save_preferences(&self) -> Result<(), String> {
        let path = preferences_path().ok_or("Cannot locate the settings folder")?;
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        let value = serde_json::json!({"screenshotDirectory": self.screenshot_dir});
        std::fs::write(path, value.to_string()).map_err(|e| e.to_string())
    }
}

impl Default for Form {
    fn default() -> Self {
        Self::new()
    }
}

fn preferences_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|p| p.join("opendhu/gui.json"))
}

const BG: Color32 = Color32::from_rgb(23, 25, 28);
const BAR: Color32 = Color32::from_rgb(29, 32, 36);
const LINE: Color32 = Color32::from_rgb(49, 53, 59);
const TEXT: Color32 = Color32::from_rgb(235, 238, 243);
const DIM: Color32 = Color32::from_rgb(158, 166, 178);
const BLUE: Color32 = Color32::from_rgb(64, 136, 244);
const OK: Color32 = Color32::from_rgb(51, 200, 111);
const WARN: Color32 = Color32::from_rgb(239, 188, 92);
const BAD: Color32 = Color32::from_rgb(248, 116, 116);

pub fn configure(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    if let Ok(bytes) = std::fs::read("/System/Library/Fonts/SFNS.ttf") {
        fonts
            .font_data
            .insert("system".into(), egui::FontData::from_owned(bytes).into());
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "system".into());
    }
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = BAR;
    style.visuals.extreme_bg_color = Color32::from_rgb(16, 18, 21);
    style.visuals.faint_bg_color = BAR;
    style.visuals.window_stroke = egui::Stroke::new(1.0_f32, LINE);
    style.visuals.window_corner_radius = 12.into();
    style.visuals.selection.bg_fill = BLUE;
    style.visuals.selection.stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.corner_radius = 7.into();
        widget.bg_stroke = egui::Stroke::new(1.0_f32, LINE);
        widget.fg_stroke = egui::Stroke::new(1.0_f32, TEXT);
    }
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(38, 42, 48);
    style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(38, 42, 48);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(49, 55, 65);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(55, 66, 83);
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);
    style.spacing.interact_size = egui::vec2(36.0, 34.0);
    for kind in [egui::TextStyle::Body, egui::TextStyle::Button] {
        style
            .text_styles
            .insert(kind, egui::FontId::proportional(14.0));
    }
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(22.0));
    ctx.set_style(style);
}

fn bar_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(BAR)
        .inner_margin(egui::Margin::symmetric(18, 8))
}

fn button(ui: &mut egui::Ui, glyph: &str, hint: &str) -> egui::Response {
    ui.add_sized(
        [36.0, 36.0],
        egui::Button::new(RichText::new(glyph).size(21.0)).frame(false),
    )
    .on_hover_text(hint)
}

fn status(ui: &mut egui::Ui, text: &str, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
    ui.label(RichText::new(text).color(DIM).size(13.0));
}

fn focus_word(f: FocusState) -> &'static str {
    match f {
        FocusState::On => "on",
        FocusState::Off => "off",
        FocusState::Unknown => "unknown",
    }
}

fn card(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.label(RichText::new(title).strong());
    body(ui);
    ui.add_space(10.0);
}

fn row(ui: &mut egui::Ui, key: &str, value: impl Into<String>) {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = 18.0;
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("{key:<15}")).color(DIM).monospace());
            ui.label(value.into());
        });
    });
}

fn session(state: &State) -> Option<&SessionStatus> {
    match &state.conn {
        Conn::Connected(s) => s.session.as_ref(),
        Conn::Disconnected => None,
    }
}

fn device_name(state: &State, serial: &str) -> String {
    state
        .devices
        .iter()
        .find(|d| d.serial == serial)
        .and_then(|d| d.model.as_deref())
        .map(|s| s.replace('_', " "))
        .unwrap_or_else(|| serial.to_owned())
}

pub fn draw(
    ctx: &egui::Context,
    state: &mut State,
    form: &mut Form,
    tx: &mpsc::Sender<crate::model::Update>,
) {
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
    }
    connection_bar(ctx, state, form, tx);
    diagnostics(ctx, state, form, tx);
    if state.phase() == Some(Phase::Running) {
        actions(ctx, state, form, tx);
    }
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(BG)
                .inner_margin(egui::Margin::same(16)),
        )
        .show(ctx, |ui| {
            if state.phase().is_none() && state.busy == 0 {
                connect_view(ui, ctx, state, form, tx);
            } else {
                projection(ui, ctx, state, form, tx);
            }
        });
    settings(ctx, state, form);
}

fn connection_bar(
    ctx: &egui::Context,
    state: &mut State,
    form: &mut Form,
    tx: &mpsc::Sender<crate::model::Update>,
) {
    let controls = state.controls();
    egui::TopBottomPanel::top("connection")
        .frame(bar_frame())
        .exact_height(60.0)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                let running_serial = session(state)
                    .and_then(|s| s.details.as_ref())
                    .and_then(|d| d.head_unit.serial.as_deref());
                let selected = running_serial.unwrap_or(&form.serial);
                let name = if selected.is_empty() {
                    let ready: Vec<_> = state
                        .devices
                        .iter()
                        .filter(|d| d.state == "device")
                        .collect();
                    match ready.as_slice() {
                        [one] => device_name(state, &one.serial),
                        _ => "Choose device".into(),
                    }
                } else {
                    device_name(state, selected)
                };
                egui::ComboBox::from_id_salt("device")
                    .width(185.0)
                    .truncate()
                    .selected_text(format!("{}   {}", icon::DEVICE_MOBILE, name))
                    .show_ui(ui, |ui| {
                        if !controls.can_start {
                            ui.label(
                                RichText::new("Stop the session to change devices.").color(DIM),
                            );
                            if let Some(serial) = running_serial {
                                ui.monospace(serial);
                            }
                        }
                        ui.add_enabled_ui(controls.can_start, |ui| {
                            ui.selectable_value(
                                &mut form.serial,
                                String::new(),
                                "Automatic device",
                            );
                            for d in &state.devices {
                                let name = device_name(state, &d.serial);
                                ui.add_enabled_ui(d.state == "device", |ui| {
                                    ui.selectable_value(
                                        &mut form.serial,
                                        d.serial.clone(),
                                        format!("{name} · {}", d.state),
                                    );
                                });
                            }
                            if let Some(problem) = &state.device_problem {
                                ui.label(RichText::new(problem).color(WARN));
                            } else if state.devices.is_empty() {
                                ui.label(
                                    RichText::new("Connect a phone with USB debugging enabled.")
                                        .color(DIM),
                                );
                            }
                            ui.separator();
                            if ui.button("Enter serial manually…").clicked() {
                                form.settings = true;
                                ui.close();
                            }
                        });
                    });
                ui.add_space(4.0);
                let (label, color) = match state.phase() {
                    Some(Phase::Running) if state.phone_stopped_projecting() => {
                        ("Projection paused", WARN)
                    }
                    Some(Phase::Running) => ("Connected", OK),
                    Some(Phase::Starting) => ("Connecting…", WARN),
                    Some(Phase::Stopping) => ("Stopping…", WARN),
                    None if state.busy > 0 => ("Connecting…", WARN),
                    None => ("Not connected", DIM),
                };
                status(ui, label, color);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if state.phase().is_some() {
                        if ui
                            .add_enabled(
                                controls.can_stop,
                                egui::Button::new("Stop").min_size(egui::vec2(82.0, 36.0)),
                            )
                            .clicked()
                        {
                            dispatch(Cmd::Stop, "Stopping the session", tx, ctx, state);
                        }
                    } else if ui
                        .add_enabled(
                            controls.can_start,
                            egui::Button::new("Start")
                                .fill(BLUE)
                                .min_size(egui::vec2(82.0, 36.0)),
                        )
                        .clicked()
                    {
                        start(ctx, state, form, tx);
                    }
                    if button(ui, icon::GEAR_SIX, "Settings").clicked() {
                        form.settings = true;
                    }
                });
            });
        });
}

fn start(
    ctx: &egui::Context,
    state: &mut State,
    form: &Form,
    tx: &mpsc::Sender<crate::model::Update>,
) {
    match crate::model::parse_ports(&form.reverse) {
        Ok(reverse) => {
            let serial = Some(form.serial.trim().to_owned()).filter(|s| !s.is_empty());
            dispatch(
                Cmd::Start {
                    native: form.native,
                    serial,
                    reverse,
                    headless: form.headless && !form.native,
                },
                "Starting the session",
                tx,
                ctx,
                state,
            );
        }
        Err(text) => {
            state.notice = Some(crate::model::Notice {
                text,
                is_error: true,
            })
        }
    }
}

fn connect_view(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    state: &mut State,
    form: &Form,
    tx: &mpsc::Sender<crate::model::Update>,
) {
    let center = ui.max_rect().center();
    let rect = egui::Rect::from_center_size(
        center,
        egui::vec2(380.0_f32.min(ui.available_width()), 225.0),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(icon::MONITOR_PLAY).size(44.0).color(DIM));
            ui.add_space(14.0);
            ui.heading("Your head unit, on your desktop.");
            ui.add_space(4.0);
            ui.label(RichText::new("Connect your Android phone to get started.").color(DIM));
            ui.add_space(16.0);
            if ui
                .add_enabled(
                    state.controls().can_start,
                    egui::Button::new("Start session")
                        .fill(BLUE)
                        .min_size(egui::vec2(144.0, 40.0)),
                )
                .clicked()
            {
                start(ctx, state, form, tx);
            }
        });
    });
}

fn fitted_rect(available: egui::Rect, screen: (u16, u16)) -> egui::Rect {
    let aspect = f32::from(screen.0.max(1)) / f32::from(screen.1.max(1));
    let width = available.width().min(available.height() * aspect).max(0.0);
    egui::Rect::from_center_size(available.center(), egui::vec2(width, width / aspect))
}

fn projection(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    state: &mut State,
    form: &mut Form,
    tx: &mpsc::Sender<crate::model::Update>,
) {
    let available = ui.max_rect();
    ui.painter()
        .rect_filled(available, 10.0, Color32::from_rgb(10, 12, 14));
    ui.painter().rect_stroke(
        available,
        10.0,
        egui::Stroke::new(1.0_f32, LINE),
        egui::StrokeKind::Inside,
    );
    let rect = fitted_rect(available.shrink(8.0), state.screen());
    let stopped = state.phone_stopped_projecting();
    let native = state.native_running();
    let mut overlay = None;
    let texture = state.live_texture.as_ref().filter(|_| native && form.live);
    if let Some(texture) = texture {
        let mut image = egui::Image::new((texture.id(), rect.size()));
        if stopped || state.live_problem.is_some() {
            image = image.tint(Color32::from_gray(65));
        } else {
            image = image.sense(egui::Sense::click_and_drag());
        }
        let response = ui.put(rect, image);
        let at = |p: egui::Pos2| to_pixel(p - rect.min, rect.size(), state.screen());
        if response.drag_started()
            && let Some(p) = response.interact_pointer_pos()
        {
            form.drag = Some((at(p), std::time::Instant::now()));
        }
        if response.drag_stopped() {
            if let (Some((from, began)), Some(p)) =
                (form.drag.take(), ctx.input(|i| i.pointer.latest_pos()))
            {
                touch(gesture(from, at(p), began.elapsed()), tx, ctx);
            }
        } else if response.clicked()
            && let Some(p) = response.interact_pointer_pos()
        {
            let (x, y) = at(p);
            touch(Cmd::Tap(x, y), tx, ctx);
        }
    } else {
        form.drag = None;
        overlay = Some(
            if state.busy > 0 || matches!(state.phase(), Some(Phase::Starting | Phase::Stopping)) {
                (
                    "Connecting to your phone",
                    "The display will appear when the session is ready.",
                )
            } else if !native {
                (
                    "Google DHU is running",
                    "Your head-unit display is open in its own window.",
                )
            } else if !form.live {
                (
                    "Display paused",
                    "Resume to show the live head-unit display.",
                )
            } else {
                (
                    "Waiting for your display",
                    "Android Auto will appear here when projection starts.",
                )
            },
        );
    }
    if stopped {
        overlay = Some(("Projection paused", "Your phone stopped sending video."));
    }
    if let Some(problem) = state
        .live_problem
        .as_deref()
        .filter(|_| native && form.live)
    {
        overlay = Some(("Display unavailable", problem));
    }
    if let Some((title, description)) = overlay {
        let description = description.to_owned();
        let text_rect = egui::Rect::from_center_size(
            available.center(),
            egui::vec2(420.0_f32.min(available.width() - 24.0), 150.0),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(icon::MONITOR).size(32.0).color(DIM));
                ui.add_space(8.0);
                ui.heading(title);
                ui.label(RichText::new(description).color(DIM));
                if stopped
                    && ui
                        .add_enabled(
                            state.controls().can_focus,
                            egui::Button::new("Resume projection").fill(BLUE),
                        )
                        .clicked()
                {
                    dispatch(Cmd::Focus(true), "Resuming projection", tx, ctx, state);
                } else if native && !form.live && ui.button("Resume display").clicked() {
                    form.live = true;
                }
            });
        });
    }
}

fn actions(
    ctx: &egui::Context,
    state: &mut State,
    form: &mut Form,
    tx: &mpsc::Sender<crate::model::Update>,
) {
    egui::TopBottomPanel::bottom("actions").frame(bar_frame()).exact_height(56.0).show(ctx, |ui| {
        ui.horizontal_centered(|ui| {
            ui.add_enabled_ui(state.native_running() && !state.phone_stopped_projecting(), |ui| {
                if button(ui, icon::HOUSE, "Home").clicked() { touch(Cmd::Key(opendhu_core::Key::Home, form.long_press), tx, ctx); }
                if button(ui, icon::ARROW_LEFT, "Back").clicked() { touch(Cmd::Key(opendhu_core::Key::Back, form.long_press), tx, ctx); }
            });
            ui.separator();
            ui.add_enabled_ui(state.native_running(), |ui| {
                let glyph = if form.audio { icon::SPEAKER_HIGH } else { icon::SPEAKER_SLASH };
                if ui.selectable_label(form.audio, format!("{glyph}   Audio")).on_hover_text("Play the phone’s audio through this computer").clicked() { form.audio = !form.audio; }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::containers::menu::MenuButton::from_button(egui::Button::new(RichText::new(icon::DOTS_THREE).size(22.0)).frame(false)).ui(ui, |ui| {
                    ui.set_min_width(200.0);
                    ui.checkbox(&mut form.live, "Show display");
                    ui.checkbox(&mut form.long_press, "Long button presses");
                    ui.separator();
                    ui.add_enabled_ui(state.controls().can_focus, |ui| {
                        if ui.button("Take video focus").clicked() { dispatch(Cmd::Focus(true), "Taking video focus", tx, ctx, state); ui.close(); }
                        if ui.button("Release video focus").clicked() { dispatch(Cmd::Focus(false), "Releasing video focus", tx, ctx, state); ui.close(); }
                    });
                    ui.separator();
                    ui.add_enabled_ui(state.native_running() && !state.phone_stopped_projecting(), |ui| {
                        for (label, key) in BUTTONS {
                            if ui.button(label).clicked() { touch(Cmd::Key(key, form.long_press), tx, ctx); ui.close(); }
                        }
                    });
                }).0.on_hover_text("More controls");
                let full = ctx.input(|i| i.viewport().fullscreen == Some(true));
                if button(ui, if full { icon::CORNERS_IN } else { icon::CORNERS_OUT }, "Toggle fullscreen (Esc to exit)").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
                }
                if ui.add_enabled(state.controls().can_focus, egui::Button::new(format!("{}   Screenshot", icon::CAMERA)).frame(false).min_size(egui::vec2(124.0, 36.0))).clicked() {
                    let path = form.screenshot_path();
                    if !path.is_absolute() || path.to_string_lossy().contains(char::is_whitespace) || !path.parent().is_some_and(Path::is_dir) {
                        state.notice = Some(crate::model::Notice { text: "Choose an existing absolute screenshot folder without spaces in Settings.".into(), is_error: true });
                        form.settings = true;
                    } else { dispatch(Cmd::Screenshot(path), "Taking a screenshot", tx, ctx, state); }
                }
            });
        });
    });
}

fn diagnostics(
    ctx: &egui::Context,
    state: &mut State,
    form: &mut Form,
    tx: &mpsc::Sender<crate::model::Update>,
) {
    egui::TopBottomPanel::bottom("diagnostics-toggle")
        .frame(bar_frame().inner_margin(egui::Margin::symmetric(18, 2)))
        .exact_height(38.0)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                let caret = if form.diagnostics {
                    icon::CARET_DOWN
                } else {
                    icon::CARET_RIGHT
                };
                if ui
                    .add(egui::Button::new(format!("{caret}   Diagnostics")).frame(false))
                    .clicked()
                {
                    form.diagnostics = !form.diagnostics;
                }
                if let Some(shot) = &state.shot
                    && state
                        .notice
                        .as_ref()
                        .is_some_and(|n| !n.is_error && n.text.starts_with("Screenshot saved"))
                    && ui.small_button("Open screenshot").clicked()
                {
                    crate::worker::open_file(shot.path.clone(), ctx.clone(), tx.clone());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let notice = state.notice.as_ref();
                    if let Some(notice) = notice.filter(|n| n.is_error) {
                        ui.add(
                            egui::Label::new(RichText::new(&notice.text).color(BAD).size(12.0))
                                .truncate(),
                        )
                        .on_hover_text(&notice.text);
                    } else if let Some(problem) = &state.live_problem {
                        ui.label(RichText::new("Stream issue").color(WARN).size(12.0))
                            .on_hover_text(problem);
                    } else if let Some(tunnel) = session(state)
                        .and_then(|s| s.details.as_ref())
                        .and_then(|d| d.tunnels.iter().find(|t| t.state != TunnelState::Active))
                    {
                        let label = if tunnel.state == TunnelState::Missing {
                            "Tunnel disconnected"
                        } else {
                            "Tunnel status unknown"
                        };
                        ui.label(RichText::new(label).color(WARN).size(12.0))
                            .on_hover_text(format!(
                                "Port {} · open Diagnostics for details",
                                tunnel.port
                            ));
                    } else if state.phone_stopped_projecting() {
                        status(ui, "Projection paused", WARN);
                    } else if let Some(notice) =
                        notice.filter(|n| state.busy > 0 || n.text.starts_with("Screenshot saved"))
                    {
                        let text = if notice.text.starts_with("Screenshot saved") {
                            "Screenshot saved"
                        } else {
                            &notice.text
                        };
                        ui.add(
                            egui::Label::new(RichText::new(text).color(DIM).size(12.0)).truncate(),
                        )
                        .on_hover_text(&notice.text);
                    } else if state.native_running() && form.live && state.live_texture.is_none() {
                        status(ui, "Waiting for video", DIM);
                    } else if state.phase() == Some(Phase::Running) {
                        status(ui, "No issues", OK);
                    }
                });
            });
        });
    if form.diagnostics {
        let max_height = ctx.available_rect().height() * 0.55;
        let height = 320.0_f32.min(max_height);
        egui::TopBottomPanel::bottom("diagnostics")
            .frame(bar_frame())
            .resizable(true)
            .default_height(height)
            .min_height(90.0)
            .max_height(max_height)
            .show(ctx, |ui| {
                ui.spacing_mut().interact_size.y = 18.0;
                ui.spacing_mut().item_spacing.y = 4.0;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if let Some(notice) = &state.notice
                            && notice.is_error
                        {
                            ui.label(RichText::new(&notice.text).color(BAD));
                            ui.separator();
                        }
                        let s = session(state).cloned();
                        ui.columns(2, |columns| {
                            session_card(&mut columns[0], s.as_ref());
                            focus_card(
                                &mut columns[0],
                                s.as_ref(),
                                state.controls().can_focus,
                                tx,
                                ctx,
                                state,
                            );
                            if !state.live_summary.is_empty() {
                                row(&mut columns[0], "stream", &state.live_summary);
                            }
                            if let Some(summary) = &state.audio_summary {
                                row(&mut columns[0], "audio", summary);
                            }
                            if let Some(problem) = &state.device_problem {
                                row(&mut columns[0], "devices", problem);
                            }
                            let ui = &mut columns[1];
                            ui.label(RichText::new("Events").strong());
                            if state.log.is_empty() {
                                ui.label(RichText::new("No events yet.").color(DIM));
                            }
                            for l in &state.log {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(RichText::new(&l.time).color(DIM).monospace());
                                    ui.label(
                                        RichText::new(&l.kind)
                                            .color(if l.kind.ends_with("failed") {
                                                BAD
                                            } else {
                                                TEXT
                                            })
                                            .monospace(),
                                    );
                                    ui.label(RichText::new(&l.detail).color(DIM).monospace());
                                });
                            }
                            if let Some(shot) = &state.shot {
                                ui.collapsing("Last screenshot", |ui| {
                                    ui.label(
                                        RichText::new(shot.path.display().to_string()).color(DIM),
                                    );
                                    let width = ui.available_width().min(240.0);
                                    ui.image((
                                        shot.texture.id(),
                                        egui::vec2(
                                            width,
                                            width * shot.height as f32 / shot.width.max(1) as f32,
                                        ),
                                    ));
                                });
                            }
                        });
                    });
            });
    }
}

fn settings(ctx: &egui::Context, state: &mut State, form: &mut Form) {
    let mut open = form.settings;
    let mut save = false;
    egui::Window::new("Settings")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(390.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label(RichText::new("Session").strong());
            ui.add_enabled_ui(state.controls().can_start, |ui| {
                ui.label(RichText::new("Head unit").color(DIM));
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut form.native, true, "openDHU native");
                    ui.selectable_value(&mut form.native, false, "Google DHU");
                });
                ui.label(RichText::new("Device serial (optional)").color(DIM));
                ui.add(
                    egui::TextEdit::singleline(&mut form.serial)
                        .hint_text("Automatic device")
                        .desired_width(f32::INFINITY),
                );
                ui.label(RichText::new("Reverse ports (optional)").color(DIM));
                ui.add(
                    egui::TextEdit::singleline(&mut form.reverse)
                        .hint_text("10101, 8080")
                        .desired_width(f32::INFINITY),
                );
                if !form.native {
                    ui.checkbox(&mut form.headless, "Run Google DHU without its window");
                }
            });
            ui.add_space(8.0);
            ui.separator();
            ui.label(RichText::new("Screenshots").strong());
            ui.label(RichText::new("Save folder").color(DIM));
            ui.add(
                egui::TextEdit::singleline(&mut form.screenshot_dir).desired_width(f32::INFINITY),
            );
            ui.label(
                RichText::new("Use an existing absolute folder without spaces.")
                    .small()
                    .color(DIM),
            );
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Done").clicked() {
                    save = true;
                }
            });
        });
    if save || (form.settings && !open) {
        if let Err(text) = form.save_preferences() {
            state.notice = Some(crate::model::Notice {
                text,
                is_error: true,
            });
        }
        open = false;
    }
    form.settings = open;
}

/// What a gesture on the live picture means: a short or still press is a tap at where it began, and
/// anything longer is a swipe that takes as long as the drag did (50 ms to 2 s).
pub fn gesture(start: (u32, u32), end: (u32, u32), held: std::time::Duration) -> Cmd {
    let (dx, dy) = (
        i64::from(end.0) - i64::from(start.0),
        i64::from(end.1) - i64::from(start.1),
    );
    if dx * dx + dy * dy < 12 * 12 {
        return Cmd::Tap(start.0, start.1);
    }
    let ms = u64::try_from(held.as_millis())
        .unwrap_or(u64::MAX)
        .clamp(50, 2000);
    Cmd::Swipe {
        from: start,
        to: end,
        duration_ms: ms,
    }
}

const BUTTONS: [(&str, opendhu_core::Key); 8] = [
    ("Up", opendhu_core::Key::Up),
    ("Down", opendhu_core::Key::Down),
    ("Left", opendhu_core::Key::Left),
    ("Right", opendhu_core::Key::Right),
    ("Enter", opendhu_core::Key::Enter),
    ("Play / pause", opendhu_core::Key::PlayPause),
    ("Next", opendhu_core::Key::Next),
    ("Previous", opendhu_core::Key::Previous),
];

fn touch(cmd: Cmd, tx: &mpsc::Sender<crate::model::Update>, ctx: &egui::Context) {
    #[cfg(test)]
    {
        let _ = (tx, ctx);
        tests::SENT.with(|s| s.borrow_mut().push(cmd));
    }
    #[cfg(not(test))]
    crate::worker::run_touch(cmd, ctx.clone(), tx.clone());
}

fn dispatch(
    cmd: Cmd,
    what: &str,
    tx: &mpsc::Sender<crate::model::Update>,
    ctx: &egui::Context,
    state: &mut State,
) {
    state.begin_command(what);
    // Tests only record what the window would send. They must never reach a real daemon: a click
    // test could otherwise touch a phone that happens to be running a session.
    #[cfg(test)]
    {
        let _ = (tx, ctx);
        tests::SENT.with(|s| s.borrow_mut().push(cmd));
    }
    #[cfg(not(test))]
    crate::worker::run(cmd, ctx.clone(), tx.clone());
}

fn session_card(ui: &mut egui::Ui, session: Option<&SessionStatus>) {
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
        if h.backend == "native" {
            row(
                ui,
                "certificate",
                h.certificate_source.as_deref().unwrap_or("unknown"),
            );
        }
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

/// The screenshot pixel under a click. `offset` is from the picture's top left, `shown` is the size
/// it is drawn at, and `screen` is the session's touch screen, so scaling is undone and the result is
/// clamped to it.
pub fn to_pixel(offset: egui::Vec2, shown: egui::Vec2, screen: (u16, u16)) -> (u32, u32) {
    let (w, h) = (f32::from(screen.0), f32::from(screen.1));
    let x = (offset.x / shown.x.max(1.0) * w).clamp(0.0, w - 1.0);
    let y = (offset.y / shown.y.max(1.0) * h).clamp(0.0, h - 1.0);
    (x as u32, y as u32)
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;

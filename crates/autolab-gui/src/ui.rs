//! Drawing. All decisions come from `State`; this file only lays them out.

use std::sync::mpsc;

use autolab_core::FocusState;
use autolab_daemon::api::{Phase, SessionStatus, TunnelState};
use eframe::egui::{self, Color32, RichText};

use crate::model::{Conn, State};
use crate::worker::Cmd;

/// What the user typed. Kept apart from `State` because only the window edits it.
pub struct Form {
    pub serial: String,
    pub reverse: String,
    pub native: bool,
    pub headless: bool,
    /// Show the live picture while a native session runs.
    pub live: bool,
    /// A drag in progress on the live picture: where it began, and when.
    pub drag: Option<((u32, u32), std::time::Instant)>,
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
            native: false,
            headless: false,
            live: true,
            drag: None,
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
            live_card(ui, form, tx, ctx, state);
            if session.is_none() {
                start_card(ui, form, controls.can_start, tx, ctx, state);
            }
            focus_card(ui, session.as_ref(), controls.can_focus, tx, ctx, state);
            let native = state.native_running();
            screenshot_card(
                ui,
                form,
                controls.can_focus,
                native && controls.can_focus,
                tx,
                ctx,
                state,
            );
        });
    });
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

/// A touch from the live picture: no disabled buttons, no "busy" state.
fn touch(cmd: Cmd, tx: &mpsc::Sender<crate::model::Update>, ctx: &egui::Context) {
    #[cfg(test)]
    {
        let _ = (tx, ctx);
        tests::SENT.with(|s| s.borrow_mut().push(cmd));
    }
    #[cfg(not(test))]
    crate::worker::run_touch(cmd, ctx.clone(), tx.clone());
}

fn live_card(
    ui: &mut egui::Ui,
    form: &mut Form,
    tx: &mpsc::Sender<crate::model::Update>,
    ctx: &egui::Context,
    state: &State,
) {
    card(ui, "Live view", |ui| {
        if !state.native_running() {
            ui.label(
                RichText::new("Needs a running native session. The DHU draws its own window.")
                    .color(DIM),
            );
            return;
        }
        ui.checkbox(&mut form.live, "Show the head unit display");
        if !form.live {
            return;
        }
        let Some(texture) = &state.live_texture else {
            match &state.live_problem {
                Some(p) => ui.label(RichText::new(format!("No picture: {p}")).color(BAD)),
                None => ui.label(RichText::new("Waiting for the first picture…").color(DIM)),
            };
            return;
        };
        let width = ui.available_width().min(800.0);
        let size = egui::vec2(width, width * 480.0 / 800.0);
        let response =
            ui.add(egui::Image::new((texture.id(), size)).sense(egui::Sense::click_and_drag()));
        ui.label(
            RichText::new("This is the head unit display, as a car would show it. Click to tap, drag to swipe: touches go to the phone.")
                .color(DIM),
        );
        let at = |p: egui::Pos2| to_pixel(p - response.rect.min, response.rect.size());
        if response.drag_started()
            && let Some(p) = response.interact_pointer_pos()
        {
            form.drag = Some((at(p), std::time::Instant::now()));
        }
        if response.drag_stopped() {
            if let (Some((start, began)), Some(p)) =
                (form.drag.take(), ctx.input(|i| i.pointer.latest_pos()))
            {
                touch(gesture(start, at(p), began.elapsed()), tx, ctx);
            }
        } else if response.clicked()
            && let Some(p) = response.interact_pointer_pos()
        {
            let (x, y) = at(p);
            touch(Cmd::Tap(x, y), tx, ctx);
        }
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
        ui.horizontal(|ui| {
            ui.label("Head unit");
            ui.selectable_value(&mut form.native, false, "Google DHU");
            ui.selectable_value(&mut form.native, true, "AutoLab native");
        });
        ui.add_enabled(
            !form.native,
            egui::Checkbox::new(&mut form.headless, "Run the DHU without its window"),
        );
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
                            native: form.native,
                            serial,
                            reverse,
                            headless: form.headless && !form.native,
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

/// The screenshot pixel under a click. `offset` is from the picture's top left and `shown` is the
/// size it is drawn at, so scaling is undone and the result is clamped to the 800x480 touch screen.
pub fn to_pixel(offset: egui::Vec2, shown: egui::Vec2) -> (u32, u32) {
    let (w, h) = (
        f32::from(autolab_native::protocol::SCREEN_WIDTH),
        f32::from(autolab_native::protocol::SCREEN_HEIGHT),
    );
    let x = (offset.x / shown.x.max(1.0) * w).clamp(0.0, w - 1.0);
    let y = (offset.y / shown.y.max(1.0) * h).clamp(0.0, h - 1.0);
    (x as u32, y as u32)
}

fn screenshot_card(
    ui: &mut egui::Ui,
    form: &mut Form,
    can_shoot: bool,
    can_tap: bool,
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
            // Clicking the picture taps the phone at the same spot, but only on the native backend and
            // only when no command is running. It is a real touch on the real phone.
            let tappable = can_tap;
            let image = egui::Image::new((shot.texture.id(), egui::vec2(width, height))).sense(
                if tappable {
                    egui::Sense::click()
                } else {
                    egui::Sense::hover()
                },
            );
            let response = ui.add(image);
            if can_tap {
                ui.label(
                    RichText::new(
                        "Click the picture to tap the phone at that spot (native backend).",
                    )
                    .color(DIM),
                );
            }
            if tappable
                && response.clicked()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let (x, y) = to_pixel(pos - response.rect.min, response.rect.size());
                dispatch(Cmd::Tap(x, y), "Sending a tap", tx, ctx, state);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LogLine, Notice, Update};

    thread_local! {
        /// Every command the window would have sent on this test's thread.
        pub static SENT: std::cell::RefCell<Vec<Cmd>> = const { std::cell::RefCell::new(Vec::new()) };
    }
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

    /// A running native session with a screenshot on show, as the window sees it.
    fn native_state_with_a_shot(ctx: &egui::Context) -> State {
        use autolab_core::{FocusState, SessionState, VideoFocusStatus};
        use autolab_daemon::api::{Phase, RunningDetails, SessionStatus};
        let mut s = State::new();
        s.conn = Conn::Connected(Box::new(StatusResponse {
            daemon: DaemonInfo {
                version: "0".into(),
                pid: 1,
                port: 1,
                exit_when_idle: false,
            },
            session: Some(SessionStatus {
                session_id: "s".into(),
                phase: Phase::Running,
                details: Some(RunningDetails {
                    head_unit: autolab_core::HeadUnitStatus {
                        backend: "native".into(),
                        state: SessionState::Running,
                        serial: None,
                        backend_version: None,
                        backend_build: None,
                        video_focus: VideoFocusStatus {
                            requested: FocusState::On,
                            backend_reported: FocusState::On,
                            observed: FocusState::On,
                        },
                    },
                    transport: "usb".into(),
                    daemon_pid: 1,
                    dhu_pid: 2,
                    tunnels: vec![],
                }),
            }),
        }));
        let texture = ctx.load_texture(
            "t",
            egui::ColorImage::filled([800, 480], Color32::RED),
            egui::TextureOptions::LINEAR,
        );
        s.apply(Update::Screenshot {
            path: "/x.png".into(),
            width: 800,
            height: 480,
            texture,
        });
        s
    }

    #[test]
    fn clicking_the_preview_of_a_native_session_sends_a_tap_and_a_dhu_session_does_not() {
        for native in [true, false] {
            let ctx = egui::Context::default();
            let mut state = native_state_with_a_shot(&ctx);
            if !native {
                if let Conn::Connected(s) = &mut state.conn {
                    s.session
                        .as_mut()
                        .unwrap()
                        .details
                        .as_mut()
                        .unwrap()
                        .head_unit
                        .backend = "dhu".into();
                }
            }
            let mut form = Form::new();
            let (tx, _rx) = mpsc::channel();
            let raw = |events: Vec<egui::Event>| egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(1000.0, 1400.0),
                )),
                events,
                ..Default::default()
            };
            // Two quiet frames so every widget has a size, then click a grid of points.
            for _ in 0..2 {
                let _ = ctx.run(raw(vec![]), |c| draw(c, &mut state, &mut form, &tx));
            }
            let mut tapped = false;
            'grid: for y in (100..1300).step_by(40) {
                for x in (20..600).step_by(40) {
                    let at = egui::pos2(x as f32, y as f32);
                    let button = |pressed| egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    };
                    for events in [
                        vec![egui::Event::PointerMoved(at)],
                        vec![button(true)],
                        vec![button(false)],
                        vec![],
                    ] {
                        let _ = ctx.run(raw(events), |c| draw(c, &mut state, &mut form, &tx));
                    }
                    let sent = SENT.with(|s| s.borrow_mut().drain(..).collect::<Vec<_>>());
                    if let Some(Cmd::Tap(px, py)) =
                        sent.into_iter().find(|c| matches!(c, Cmd::Tap(..)))
                    {
                        assert!(
                            px < 800 && py < 480,
                            "({px}, {py}) is on the phone's screen"
                        );
                        tapped = true;
                        break 'grid;
                    }
                    // Ignore anything else a click may have started (focus, screenshot...).
                    state.busy = 0;
                    state.notice = None;
                }
            }
            assert_eq!(tapped, native, "native={native}");
        }
    }

    #[test]
    fn choosing_native_and_pressing_start_sends_a_native_start() {
        let ctx = egui::Context::default();
        let mut state = State::new(); // no daemon: the Start card is shown
        let mut form = Form::new();
        let (tx, _rx) = mpsc::channel();
        let raw = |events: Vec<egui::Event>| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1000.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let click = |at: egui::Pos2, state: &mut State, form: &mut Form| {
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            for events in [
                vec![egui::Event::PointerMoved(at)],
                vec![button(true)],
                vec![button(false)],
                vec![],
            ] {
                let _ = ctx.run(raw(events), |c| draw(c, state, form, &tx));
            }
        };
        for _ in 0..2 {
            let _ = ctx.run(raw(vec![]), |c| draw(c, &mut state, &mut form, &tx));
        }
        let grid = || {
            (60..700).step_by(8).flat_map(|y| {
                (20..600)
                    .step_by(12)
                    .map(move |x| egui::pos2(x as f32, y as f32))
            })
        };
        // Click around until the "AutoLab native" choice takes.
        let mut chose = false;
        for at in grid() {
            click(at, &mut state, &mut form);
            if form.native {
                chose = true;
                break;
            }
            form.headless = false; // a stray click may have ticked the checkbox
        }
        assert!(chose, "no click selected AutoLab native");
        // Then click around until something sends a start.
        SENT.with(|s| s.borrow_mut().clear());
        let mut started = None;
        for at in grid() {
            click(at, &mut state, &mut form);
            let sent = SENT.with(|s| s.borrow_mut().drain(..).collect::<Vec<_>>());
            if let Some(Cmd::Start { native, .. }) =
                sent.into_iter().find(|c| matches!(c, Cmd::Start { .. }))
            {
                started = Some(native);
                break;
            }
            state.busy = 0;
        }
        assert_eq!(started, Some(true), "Start must carry the native choice");
    }

    #[test]
    fn a_still_press_is_a_tap_and_a_drag_is_a_swipe_as_long_as_it_took() {
        use std::time::Duration;
        let tap = gesture((10, 20), (14, 23), Duration::from_millis(400));
        assert!(matches!(tap, Cmd::Tap(10, 20)), "{tap:?}");
        let swipe = gesture((10, 20), (300, 20), Duration::from_millis(400));
        assert!(
            matches!(
                swipe,
                Cmd::Swipe {
                    from: (10, 20),
                    to: (300, 20),
                    duration_ms: 400
                }
            ),
            "{swipe:?}"
        );
        // Very quick and very slow drags are held inside what the head unit accepts.
        let quick = gesture((0, 0), (100, 0), Duration::from_millis(5));
        assert!(
            matches!(
                quick,
                Cmd::Swipe {
                    duration_ms: 50,
                    ..
                }
            ),
            "{quick:?}"
        );
        let slow = gesture((0, 0), (100, 0), Duration::from_secs(30));
        assert!(
            matches!(
                slow,
                Cmd::Swipe {
                    duration_ms: 2000,
                    ..
                }
            ),
            "{slow:?}"
        );
    }

    #[test]
    fn the_live_card_draws_waiting_with_a_problem_and_with_a_picture() {
        let ctx = egui::Context::default();
        let mut form = Form::new();
        let mut state = native_state_with_a_shot(&ctx);
        frame(&mut state, &mut form); // native running, no picture yet
        state.live_problem = Some("ffmpeg not found".into());
        frame(&mut state, &mut form);
        state.live_texture = Some(ctx.load_texture(
            "live",
            egui::ColorImage::filled([800, 480], Color32::BLUE),
            egui::TextureOptions::LINEAR,
        ));
        frame(&mut state, &mut form);
        form.live = false;
        frame(&mut state, &mut form);
    }

    #[test]
    fn a_click_on_the_picture_maps_back_to_screen_pixels_and_stays_on_the_screen() {
        // Drawn at 400x240, half size: the middle is (400, 240) on the real screen.
        let shown = egui::vec2(400.0, 240.0);
        assert_eq!(to_pixel(egui::vec2(200.0, 120.0), shown), (400, 240));
        assert_eq!(to_pixel(egui::vec2(0.0, 0.0), shown), (0, 0));
        assert_eq!(to_pixel(egui::vec2(400.0, 240.0), shown), (799, 479));
        assert_eq!(to_pixel(egui::vec2(-5.0, 9999.0), shown), (0, 479));
        // Full size: one to one.
        assert_eq!(
            to_pixel(egui::vec2(764.0, 348.0), egui::vec2(800.0, 480.0)),
            (764, 348)
        );
    }

    #[test]
    fn the_default_screenshot_path_is_absolute_png_without_whitespace() {
        let f = Form::new();
        assert!(f.shot_path.starts_with('/') && f.shot_path.ends_with(".png"));
        assert!(!f.shot_path.contains(char::is_whitespace));
    }
}

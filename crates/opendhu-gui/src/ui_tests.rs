use super::*;
use crate::model::{LogLine, Notice, Update};

thread_local! {
    /// Every command the window would have sent on this test's thread.
    pub static SENT: std::cell::RefCell<Vec<Cmd>> = const { std::cell::RefCell::new(Vec::new()) };
}
use opendhu_daemon::api::{DaemonInfo, StatusResponse};

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
    use opendhu_core::{FocusState, SessionState, VideoFocusStatus};
    use opendhu_daemon::api::{Phase, RunningDetails, SessionStatus};
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
                head_unit: opendhu_core::HeadUnitStatus {
                    backend: "native".into(),
                    state: SessionState::Running,
                    serial: None,
                    backend_version: None,
                    backend_build: None,
                    certificate_source: None,
                    screen_width: Some(800),
                    screen_height: Some(480),
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
fn start_uses_the_selected_backend_serial_and_ports() {
    for native in [true, false] {
        let ctx = egui::Context::default();
        let mut state = State::new();
        let mut form = Form::new();
        form.native = native;
        form.serial = "PHONE123".into();
        form.reverse = "10101,8080".into();
        form.headless = true;
        let size = egui::vec2(1000.0, 640.0);
        render(&ctx, &mut state, &mut form, size, vec![]);
        let output = render(&ctx, &mut state, &mut form, size, vec![]);
        let start = text_rect(&output, "Start session").unwrap();
        SENT.with(|s| s.borrow_mut().clear());
        click_at(&ctx, &mut state, &mut form, size, start.center());
        let sent = SENT.with(|s| s.borrow_mut().drain(..).collect::<Vec<_>>());
        assert!(sent.iter().any(|c| matches!(c, Cmd::Start { native: n, serial: Some(s), reverse, headless } if *n == native && s == "PHONE123" && reverse == &[10101, 8080] && *headless != native)), "{sent:?}");
    }
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
fn a_session_whose_phone_stopped_projecting_is_recognised_and_draws_dimmed() {
    let ctx = egui::Context::default();
    let mut state = native_state_with_a_shot(&ctx);
    assert!(!state.phone_stopped_projecting());
    if let Conn::Connected(s) = &mut state.conn {
        s.session
            .as_mut()
            .unwrap()
            .details
            .as_mut()
            .unwrap()
            .head_unit
            .video_focus
            .observed = opendhu_core::FocusState::Off;
    }
    assert!(state.phone_stopped_projecting());
    state.live_texture = Some(ctx.load_texture(
        "live",
        egui::ColorImage::filled([800, 480], Color32::BLUE),
        egui::TextureOptions::LINEAR,
    ));
    frame(&mut state, &mut Form::new());
}

#[test]
fn a_click_on_the_picture_maps_back_to_screen_pixels_and_stays_on_the_screen() {
    // Drawn at 400x240, half size: the middle is (400, 240) on the real screen.
    let shown = egui::vec2(400.0, 240.0);
    let screen = (800, 480);
    assert_eq!(
        to_pixel(egui::vec2(200.0, 120.0), shown, screen),
        (400, 240)
    );
    assert_eq!(to_pixel(egui::vec2(0.0, 0.0), shown, screen), (0, 0));
    assert_eq!(
        to_pixel(egui::vec2(400.0, 240.0), shown, screen),
        (799, 479)
    );
    assert_eq!(to_pixel(egui::vec2(-5.0, 9999.0), shown, screen), (0, 479));
    // Full size: one to one.
    assert_eq!(
        to_pixel(egui::vec2(764.0, 348.0), egui::vec2(800.0, 480.0), screen),
        (764, 348)
    );
    // A different screen (720p): scaling and clamping both follow it.
    assert_eq!(
        to_pixel(egui::vec2(400.0, 240.0), shown, (1280, 720)),
        (1279, 719)
    );
}

#[test]
fn the_default_screenshot_path_is_absolute_png_without_whitespace() {
    let f = Form::new();
    assert!(f.screenshot_path().is_absolute() && f.screenshot_path().extension().unwrap() == "png");
    assert!(
        !f.screenshot_path()
            .to_string_lossy()
            .contains(char::is_whitespace)
    );
}

fn render(
    ctx: &egui::Context,
    state: &mut State,
    form: &mut Form,
    size: egui::Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let (tx, _rx) = mpsc::channel();
    ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ctx| draw(ctx, state, form, &tx),
    )
}

fn text_rect(output: &egui::FullOutput, needle: &str) -> Option<egui::Rect> {
    output.shapes.iter().find_map(|s| match &s.shape {
        egui::epaint::Shape::Text(t) if t.galley.job.text.contains(needle) => {
            Some(egui::Rect::from_min_size(t.pos, t.galley.size()))
        }
        _ => None,
    })
}

fn click_at(
    ctx: &egui::Context,
    state: &mut State,
    form: &mut Form,
    size: egui::Vec2,
    at: egui::Pos2,
) {
    for events in [
        vec![egui::Event::PointerMoved(at)],
        vec![egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
        vec![egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
        vec![],
    ] {
        render(ctx, state, form, size, events);
    }
}

#[test]
fn projection_fits_both_window_dimensions_and_scaled_taps_hit_the_phone() {
    for size in [
        egui::vec2(1000.0, 640.0),
        egui::vec2(720.0, 480.0),
        egui::vec2(1400.0, 600.0),
    ] {
        let ctx = egui::Context::default();
        let mut state = native_state_with_a_shot(&ctx);
        let live = ctx.load_texture(
            "live-fit",
            egui::ColorImage::filled([800, 480], Color32::BLUE),
            egui::TextureOptions::LINEAR,
        );
        let id = live.id();
        state.live_texture = Some(live);
        let mut form = Form::new();
        render(&ctx, &mut state, &mut form, size, vec![]);
        let output = render(&ctx, &mut state, &mut form, size, vec![]);
        let rect = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::epaint::Shape::Rect(r) if r.fill_texture_id() == id => Some(r.rect),
                egui::epaint::Shape::Mesh(m) if m.texture_id == id => Some(m.calc_bounds()),
                _ => None,
            })
            .expect("live projection must be visible");
        assert!(
            rect.min.y >= 48.0 && rect.max.y <= size.y - 72.0,
            "projection must fit above persistent controls: {rect:?}, {size:?}"
        );
        assert!(rect.min.x >= 0.0 && rect.max.x <= size.x);
        assert!((rect.width() / rect.height() - 800.0 / 480.0).abs() < 0.01);
        assert!(
            rect.height() > size.y * 0.55,
            "display must dominate the window: {rect:?}"
        );
        SENT.with(|s| s.borrow_mut().clear());
        click_at(&ctx, &mut state, &mut form, size, rect.center());
        let sent = SENT.with(|s| s.borrow_mut().drain(..).collect::<Vec<_>>());
        assert!(sent.iter().any(|c| matches!(c, Cmd::Tap(x, y) if (399..=400).contains(x) && (239..=240).contains(y))), "{sent:?}");
    }
}

#[test]
fn diagnostics_are_hidden_until_opened_and_do_not_hide_the_controls() {
    let ctx = egui::Context::default();
    let size = egui::vec2(1000.0, 640.0);
    let mut state = native_state_with_a_shot(&ctx);
    state.log.push_back(LogLine {
        time: "10:24:00".into(),
        kind: "test.event".into(),
        detail: "diagnostic detail".into(),
    });
    let mut form = Form::new();
    render(&ctx, &mut state, &mut form, size, vec![]);
    let output = render(&ctx, &mut state, &mut form, size, vec![]);
    assert!(
        text_rect(&output, "test.event").is_none(),
        "events should be tucked away by default"
    );
    let toggle = text_rect(&output, "Diagnostics").expect("diagnostics disclosure");
    click_at(&ctx, &mut state, &mut form, size, toggle.center());
    let output = render(&ctx, &mut state, &mut form, size, vec![]);
    assert!(text_rect(&output, "test.event").is_some());
    let screenshot = text_rect(&output, "Screenshot").expect("screenshot stays available");
    assert!(screenshot.max.y < size.y);
}

#[test]
fn screenshot_button_dispatches_unique_png_paths_without_overwriting_the_previous_capture() {
    let ctx = egui::Context::default();
    let mut state = native_state_with_a_shot(&ctx);
    let mut form = Form::new();
    form.screenshot_dir = "/tmp".into();
    let size = egui::vec2(1000.0, 640.0);
    let mut paths = Vec::new();
    for _ in 0..2 {
        render(&ctx, &mut state, &mut form, size, vec![]);
        let output = render(&ctx, &mut state, &mut form, size, vec![]);
        let at = text_rect(&output, "   Screenshot").unwrap().center();
        SENT.with(|s| s.borrow_mut().clear());
        click_at(&ctx, &mut state, &mut form, size, at);
        let sent = SENT.with(|s| s.borrow_mut().drain(..).collect::<Vec<_>>());
        let path = sent
            .into_iter()
            .find_map(|c| match c {
                Cmd::Screenshot(p) => Some(p),
                _ => None,
            })
            .expect("screenshot command");
        assert_eq!(path.parent().unwrap(), Path::new("/tmp"));
        assert_eq!(path.extension().unwrap(), "png");
        paths.push(path);
        state.apply(Update::Done(Ok("Screenshot saved".into())));
    }
    assert_ne!(paths[0], paths[1]);
}

#[test]
fn a_paused_or_broken_projection_never_sends_touches_to_a_stale_frame() {
    for broken in [true, false] {
        let ctx = egui::Context::default();
        let mut state = native_state_with_a_shot(&ctx);
        state.live_texture = Some(ctx.load_texture(
            "stale",
            egui::ColorImage::filled([800, 480], Color32::BLUE),
            egui::TextureOptions::LINEAR,
        ));
        if broken {
            state.live_problem = Some("decoder stopped".into());
        } else if let Conn::Connected(s) = &mut state.conn {
            s.session
                .as_mut()
                .unwrap()
                .details
                .as_mut()
                .unwrap()
                .head_unit
                .video_focus
                .observed = FocusState::Off;
        }
        let mut form = Form::new();
        let size = egui::vec2(1000.0, 640.0);
        render(&ctx, &mut state, &mut form, size, vec![]);
        render(&ctx, &mut state, &mut form, size, vec![]);
        SENT.with(|s| s.borrow_mut().clear());
        click_at(&ctx, &mut state, &mut form, size, egui::pos2(350.0, 200.0));
        let sent = SENT.with(|s| s.borrow_mut().drain(..).collect::<Vec<_>>());
        assert!(
            !sent
                .iter()
                .any(|c| matches!(c, Cmd::Tap(..) | Cmd::Swipe { .. })),
            "{sent:?}"
        );
    }
}

#[test]
fn a_missing_tunnel_is_visible_after_a_successful_start_while_diagnostics_are_collapsed() {
    let ctx = egui::Context::default();
    let mut state = native_state_with_a_shot(&ctx);
    if let Conn::Connected(s) = &mut state.conn {
        s.session
            .as_mut()
            .unwrap()
            .details
            .as_mut()
            .unwrap()
            .tunnels
            .push(opendhu_daemon::api::TunnelStatus {
                port: 10101,
                state: TunnelState::Missing,
                reapplied: 0,
            });
    }
    state.apply(Update::Done(Ok("Session started".into())));
    let mut form = Form::new();
    let size = egui::vec2(1000.0, 640.0);
    render(&ctx, &mut state, &mut form, size, vec![]);
    let output = render(&ctx, &mut state, &mut form, size, vec![]);
    assert!(text_rect(&output, "Tunnel disconnected").is_some());
    assert!(text_rect(&output, "No issues").is_none());
}

#[test]
fn audio_can_be_muted_after_the_phone_releases_video_focus() {
    let ctx = egui::Context::default();
    let mut state = native_state_with_a_shot(&ctx);
    if let Conn::Connected(s) = &mut state.conn {
        s.session
            .as_mut()
            .unwrap()
            .details
            .as_mut()
            .unwrap()
            .head_unit
            .video_focus
            .observed = FocusState::Off;
    }
    let mut form = Form::new();
    form.audio = true;
    let size = egui::vec2(1000.0, 640.0);
    render(&ctx, &mut state, &mut form, size, vec![]);
    let output = render(&ctx, &mut state, &mut form, size, vec![]);
    click_at(
        &ctx,
        &mut state,
        &mut form,
        size,
        text_rect(&output, "Audio").unwrap().center(),
    );
    assert!(!form.audio, "video focus must not prevent muting audio");
}

#[test]
fn diagnostics_label_and_hit_target_fit_inside_the_footer() {
    for size in [egui::vec2(720.0, 480.0), egui::vec2(1152.0, 768.0)] {
        for expanded in [false, true] {
            let ctx = egui::Context::default();
            configure(&ctx);
            let mut state = native_state_with_a_shot(&ctx);
            let mut form = Form::new();
            form.diagnostics = expanded;
            render(&ctx, &mut state, &mut form, size, vec![]);
            let output = render(&ctx, &mut state, &mut form, size, vec![]);
            let label = text_rect(&output, "Diagnostics").unwrap();
            let clip = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::epaint::Shape::Text(t) if t.galley.job.text.contains("Diagnostics") => {
                        Some(s.clip_rect)
                    }
                    _ => None,
                })
                .unwrap();
            assert!(
                clip.contains_rect(label),
                "footer text must fit its actual clip: {label:?}, {clip:?}"
            );
            assert!(
                label.max.y <= size.y - 4.0,
                "footer label needs bottom breathing room: {label:?}, {size:?}"
            );
            click_at(&ctx, &mut state, &mut form, size, label.center());
            assert_eq!(form.diagnostics, !expanded);
        }
    }
}

#[test]
fn consecutive_events_use_compact_text_rows_with_the_real_gui_theme() {
    let ctx = egui::Context::default();
    configure(&ctx);
    let mut state = native_state_with_a_shot(&ctx);
    for i in 0..4 {
        state.log.push_back(LogLine {
            time: "12:27:27".into(),
            kind: format!("event-{i}"),
            detail: "op-1".into(),
        });
    }
    let mut form = Form::new();
    form.diagnostics = true;
    let size = egui::vec2(1152.0, 768.0);
    render(&ctx, &mut state, &mut form, size, vec![]);
    let output = render(&ctx, &mut state, &mut form, size, vec![]);
    let first = text_rect(&output, "event-0").unwrap();
    let second = text_rect(&output, "event-1").unwrap();
    assert!(
        second.min.y - first.max.y <= 8.0,
        "event text should have a small gap, not a toolbar-sized row: {first:?}, {second:?}"
    );
}

#[test]
fn the_default_drawer_shows_session_details_and_focus_controls_without_scrolling() {
    let ctx = egui::Context::default();
    configure(&ctx);
    let mut state = native_state_with_a_shot(&ctx);
    state.live_summary = "120 frames · 800×480".into();
    let mut form = Form::new();
    form.diagnostics = true;
    let size = egui::vec2(1152.0, 768.0);
    render(&ctx, &mut state, &mut form, size, vec![]);
    let output = render(&ctx, &mut state, &mut form, size, vec![]);
    let control = text_rect(&output, "Focus off")
        .expect("focus controls should be visible when opening diagnostics");
    let clip = output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::epaint::Shape::Text(t) if t.galley.job.text.contains("Focus off") => {
                Some(s.clip_rect)
            }
            _ => None,
        })
        .unwrap();
    assert!(
        clip.contains_rect(control),
        "focus controls must not be cut off: {control:?}, {clip:?}"
    );
}

#[test]
fn gpu_video_stays_fitted_and_uses_the_existing_scaled_touch_controls() {
    for stopped in [false, true] {
        let ctx = egui::Context::default();
        let mut state = native_state_with_a_shot(&ctx);
        state.gpu_video = Some(egui::PaintCallback {
            rect: egui::Rect::NOTHING,
            callback: std::sync::Arc::new(()),
        });
        if stopped {
            if let Conn::Connected(status) = &mut state.conn {
                status
                    .session
                    .as_mut()
                    .unwrap()
                    .details
                    .as_mut()
                    .unwrap()
                    .head_unit
                    .video_focus
                    .observed = opendhu_core::FocusState::Off;
            }
        }
        let mut form = Form::new();
        let size = egui::vec2(1000.0, 640.0);
        render(&ctx, &mut state, &mut form, size, vec![]);
        let output = render(&ctx, &mut state, &mut form, size, vec![]);
        let rect = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Callback(c) => Some(c.rect),
                _ => None,
            })
            .expect("GPU video must be visible");
        assert!(rect.min.y >= 48.0 && rect.max.y <= size.y - 72.0);
        assert!((rect.width() / rect.height() - 800.0 / 480.0).abs() < 0.01);
        SENT.with(|s| s.borrow_mut().clear());
        click_at(&ctx, &mut state, &mut form, size, rect.center());
        let sent = SENT.with(|s| s.borrow_mut().drain(..).collect::<Vec<_>>());
        assert_eq!(
            sent.iter()
                .any(|c| matches!(c, Cmd::Tap(399..=400, 239..=240))),
            !stopped,
            "{sent:?}"
        );
    }
}

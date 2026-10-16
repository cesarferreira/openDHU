//! The head unit proper: answers the phone, tracks what the phone says about video, and serves
//! the daemon's requests.

use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use android_auto::{
    AndroidAutoAudioInputTrait, AndroidAutoAudioOutputTrait, AndroidAutoConfiguration,
    AndroidAutoInputChannelTrait, AndroidAutoMainTrait, AndroidAutoMessage, AndroidAutoSensorTrait,
    AndroidAutoSetup, AndroidAutoVideoChannelTrait, AndroidAutoWiredTrait, AudioChannelType,
    HeadUnitInfo, InputConfiguration, SendableAndroidAutoMessage, SensorInformation,
    VideoConfiguration, Wifi,
};
use autolab_core::{
    AudioChannel, FocusState, InputResult, Key, OpStatus, OperationResult, VideoFocusStatus,
};
use autolab_native::protocol::{
    AudioBytes, Command, ErrorInfo, ErrorKind, Event, Line, Outcome, Reply, SCREEN_HEIGHT,
    SCREEN_WIDTH, SWIPE_MAX_MS, SWIPE_MIN_MS, SWIPE_STEP_MS, StatusReport,
};
use std::sync::Arc;

use crate::screenshot;

/// How long to wait for the phone to confirm a focus change. Measured on a Pixel 6: about 0.5 s
/// for a release and a few ms for a take-back.
const CONFIRM_WAIT: Duration = Duration::from_secs(4);
/// Frames since the last keyframe are kept for screenshots. A phone that sends this much without a
/// keyframe is not behaving, so the buffer is dropped and a new keyframe awaited.
const GOP_CAP: usize = 16 * 1024 * 1024;

/// How long a tap holds the finger down.
const TAP_HOLD: Duration = Duration::from_millis(60);
/// How long a long press holds the button down.
const LONG_PRESS_HOLD: Duration = Duration::from_millis(1000);
/// How long to watch the video after a touch.
const WATCH_AFTER: Duration = Duration::from_millis(150);

const BACKEND_VERSION: &str = "android-auto 0.3.8";
const BACKEND_BUILD: &str = "vendored 06c68d5 + AutoLab patches";

struct Inner {
    requested: FocusState,
    reported: FocusState,
    /// What the phone itself last said: its start and stop indications.
    observed: FocusState,
    /// Counts the phone's start and stop indications, so a wait can tell a new one from an old one.
    phone_events: u64,
    frames: u64,
    /// SPS and PPS, needed to decode.
    sps: Vec<u8>,
    pps: Vec<u8>,
    /// Every frame since the last keyframe, including it.
    gop: Vec<u8>,
    have_keyframe: bool,
    /// A viewer is watching: every frame is also sent to the daemon.
    streaming: bool,
    /// Bytes of audio received, and whether a listener wants each channel, in `AudioChannel::ALL` order.
    audio_bytes: [u64; 3],
    audio_streaming: [bool; 3],
    /// The viewer asked before a keyframe had arrived: send the burst as soon as one does.
    burst_pending: bool,
}

pub struct Shared {
    inner: Mutex<Inner>,
    ready: AtomicBool,
    tx: tokio::sync::mpsc::Sender<SendableAndroidAutoMessage>,
    rx: tokio::sync::Mutex<Option<tokio::sync::mpsc::Receiver<SendableAndroidAutoMessage>>>,
    video: VideoConfiguration,
    sensors: SensorInformation,
    input: InputConfiguration,
}

impl Shared {
    pub fn new() -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        Self {
            inner: Mutex::new(Inner {
                requested: FocusState::Unknown,
                reported: FocusState::Unknown,
                observed: FocusState::Unknown,
                phone_events: 0,
                frames: 0,
                sps: Vec::new(),
                pps: Vec::new(),
                gop: Vec::new(),
                have_keyframe: false,
                streaming: false,
                burst_pending: false,
                audio_bytes: [0; 3],
                audio_streaming: [false; 3],
            }),
            ready: AtomicBool::new(false),
            tx,
            rx: tokio::sync::Mutex::new(Some(rx)),
            video: VideoConfiguration {
                resolution: Wifi::video_resolution::Enum::_480p,
                fps: Wifi::video_fps::Enum::_30,
                dpi: 111,
            },
            sensors: SensorInformation {
                sensors: HashSet::from([
                    Wifi::sensor_type::Enum::DRIVING_STATUS,
                    Wifi::sensor_type::Enum::NIGHT_DATA,
                ]),
            },
            input: InputConfiguration {
                // The buttons we can press. The phone asks to bind the ones it wants and is refused any
                // that are not listed here.
                keycodes: Key::ALL.map(Key::android_code).to_vec(),
                touchscreen: Some((SCREEN_WIDTH, SCREEN_HEIGHT)),
            },
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub async fn handle(&self, command: Command) -> Outcome {
        match command {
            Command::Status => {
                let i = self.lock();
                Outcome::Ok(Reply::Status(StatusReport {
                    video_focus: VideoFocusStatus {
                        requested: i.requested,
                        backend_reported: i.reported,
                        observed: i.observed,
                    },
                    frames: i.frames,
                    audio_bytes: AudioBytes {
                        media: i.audio_bytes[0],
                        system: i.audio_bytes[1],
                        speech: i.audio_bytes[2],
                    },
                }))
            }
            Command::Focus { enabled } => self.focus(enabled).await,
            Command::Screenshot { path } => self.screenshot(&path).await,
            Command::Tap { x, y } => self.touch((x, y), (x, y), 0).await,
            Command::Swipe {
                x1,
                y1,
                x2,
                y2,
                duration_ms,
            } => {
                if !(SWIPE_MIN_MS..=SWIPE_MAX_MS).contains(&duration_ms) {
                    return fail(
                        ErrorKind::InvalidArgs,
                        format!("swipe duration must be {SWIPE_MIN_MS}-{SWIPE_MAX_MS} ms"),
                    );
                }
                self.touch((x1, y1), (x2, y2), duration_ms).await
            }
            Command::AudioStream { channel, enabled } => {
                self.lock().audio_streaming[audio_slot(channel)] = enabled;
                Outcome::Ok(Reply::Done)
            }
            Command::Key { key, long_press } => self.press(key, long_press).await,
            Command::VideoStream { enabled } => {
                let mut i = self.lock();
                i.streaming = enabled;
                i.burst_pending = enabled;
                if enabled {
                    Self::send_burst(&mut i);
                }
                Outcome::Ok(Reply::Done)
            }
            Command::Quit => Outcome::Ok(Reply::Done),
        }
    }

    /// One send of a touch event. The unit and the action names follow the protocol crate's own
    /// example app: microseconds since the epoch, `POINTER_DOWN`, `DRAG`, `POINTER_UP`.
    async fn send_touch(
        &self,
        action: Wifi::touch_action::Enum,
        at: (u32, u32),
    ) -> Result<(), ErrorInfo> {
        let mut location = Wifi::TouchLocation::new();
        location.set_x(at.0);
        location.set_y(at.1);
        location.set_pointer_id(0);
        let mut event = Wifi::TouchEvent::new();
        event.touch_location = vec![location];
        event.set_touch_action(action);
        let mut m = Wifi::InputEventIndication::new();
        m.set_timestamp(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_micros() as u64),
        );
        m.touch_event = android_auto::protobuf::MessageField::some(event);
        self.tx
            .send(AndroidAutoMessage::Input(m).sendable())
            .await
            .map_err(|_| ErrorInfo {
                kind: ErrorKind::Unavailable,
                message: "the Android Auto session is not running".into(),
            })
    }

    /// One button press: down, hold, up. Android Auto reads a long press from how long it is held.
    async fn press(&self, key: Key, long_press: bool) -> Outcome {
        if self.lock().observed != FocusState::On {
            return fail(
                ErrorKind::Other,
                "The phone is not projecting - nothing to press.".to_string(),
            );
        }
        let send = |pressed: bool| async move {
            let mut event = Wifi::ButtonEvent::new();
            event.set_scan_code(key.android_code());
            event.set_is_pressed(pressed);
            if long_press {
                event.set_long_press(true);
            }
            let mut events = Wifi::ButtonEvents::new();
            events.button_events.push(event);
            let mut m = Wifi::InputEventIndication::new();
            m.set_timestamp(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_micros() as u64),
            );
            m.button_event = android_auto::protobuf::MessageField::some(events);
            self.tx
                .send(AndroidAutoMessage::Input(m).sendable())
                .await
                .map_err(|_| ErrorInfo {
                    kind: ErrorKind::Unavailable,
                    message: "the Android Auto session is not running".into(),
                })
        };
        if let Err(e) = send(true).await {
            return Outcome::Err(e);
        }
        tokio::time::sleep(if long_press {
            LONG_PRESS_HOLD
        } else {
            TAP_HOLD
        })
        .await;
        if let Err(e) = send(false).await {
            return Outcome::Err(e);
        }
        Outcome::Ok(Reply::Input(InputResult {
            status: OpStatus::Accepted,
            evidence: vec![format!(
                "sent {}{} (Android key code {}); the phone does not acknowledge buttons",
                if long_press { "a long press of " } else { "" },
                key.name(),
                key.android_code()
            )],
        }))
    }

    /// A tap is a swipe of zero length and duration. Touch gets no acknowledgement from the phone,
    /// so the best status is `Accepted`: it was sent. The frames that follow are context, not proof.
    async fn touch(&self, from: (u32, u32), to: (u32, u32), duration_ms: u64) -> Outcome {
        for (x, y) in [from, to] {
            if x >= u32::from(SCREEN_WIDTH) || y >= u32::from(SCREEN_HEIGHT) {
                return fail(
                    ErrorKind::InvalidArgs,
                    format!(
                        "({x}, {y}) is outside the {SCREEN_WIDTH}x{SCREEN_HEIGHT} screen (x 0-{}, y 0-{})",
                        SCREEN_WIDTH - 1,
                        SCREEN_HEIGHT - 1
                    ),
                );
            }
        }
        let frames_before = {
            let i = self.lock();
            if i.observed != FocusState::On {
                return fail(
                    ErrorKind::Other,
                    "The phone is not projecting - nothing to touch.".to_string(),
                );
            }
            i.frames
        };
        use Wifi::touch_action::Enum::{DRAG, POINTER_DOWN, POINTER_UP};
        let sent = async {
            self.send_touch(POINTER_DOWN, from).await?;
            if duration_ms == 0 {
                tokio::time::sleep(TAP_HOLD).await;
            } else {
                let steps = (duration_ms / SWIPE_STEP_MS).max(1);
                for n in 1..=steps {
                    tokio::time::sleep(Duration::from_millis(duration_ms / steps)).await;
                    let at = (lerp(from.0, to.0, n, steps), lerp(from.1, to.1, n, steps));
                    if n < steps {
                        self.send_touch(DRAG, at).await?;
                    }
                }
            }
            self.send_touch(POINTER_UP, to).await
        }
        .await;
        if let Err(e) = sent {
            return Outcome::Err(e);
        }
        tokio::time::sleep(WATCH_AFTER).await;
        let frames = self.lock().frames - frames_before;
        let what = if from == to && duration_ms == 0 {
            format!("tap at ({}, {})", from.0, from.1)
        } else {
            format!(
                "swipe ({}, {}) to ({}, {}) over {duration_ms} ms",
                from.0, from.1, to.0, to.1
            )
        };
        Outcome::Ok(Reply::Input(InputResult {
            status: OpStatus::Accepted,
            evidence: vec![
                format!("sent {what}"),
                format!(
                    "{frames} video frames arrived during and after it (context only: the phone does not acknowledge touch)"
                ),
            ],
        }))
    }

    async fn focus(&self, enabled: bool) -> Outcome {
        let want = FocusState::from_enabled(enabled);
        let (before, already) = {
            let i = self.lock();
            (i.phone_events, i.observed == want)
        };
        let mode = if enabled {
            Wifi::video_focus_mode::Enum::FOCUSED
        } else {
            Wifi::video_focus_mode::Enum::UNFOCUSED
        };
        let mut m = Wifi::VideoFocusIndication::new();
        m.set_focus_mode(mode);
        m.set_unrequested(true);
        if self
            .tx
            .send(AndroidAutoMessage::VideoFocus(m).sendable())
            .await
            .is_err()
        {
            return Outcome::Err(ErrorInfo {
                kind: ErrorKind::Unavailable,
                message: "the Android Auto session is not running".into(),
            });
        }
        {
            let mut i = self.lock();
            i.requested = want;
            i.reported = want;
        }
        let mut evidence = vec![format!("sent VideoFocusIndication {mode:?} (unrequested)")];

        let deadline = Instant::now() + CONFIRM_WAIT;
        let confirmed = loop {
            {
                let i = self.lock();
                if i.phone_events != before && i.observed == want {
                    break true;
                }
            }
            if Instant::now() >= deadline {
                break false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let observed = self.lock().observed;
        let status = if confirmed {
            evidence.push(format!("the phone then reported video {}", word(observed)));
            OpStatus::Observed
        } else if already {
            // Nothing changes when the phone is already in the requested state, so no message is
            // expected. Its last report still matches.
            evidence.push(format!(
                "the phone had already reported video {}, so no new message was expected",
                word(observed)
            ));
            OpStatus::Observed
        } else {
            evidence.push(format!(
                "the phone did not confirm within {}s; it still reports video {}",
                CONFIRM_WAIT.as_secs(),
                word(observed)
            ));
            OpStatus::TimedOut
        };
        Outcome::Ok(Reply::Focus(OperationResult {
            status,
            requested: want,
            backend_reported: want,
            observed,
            evidence,
        }))
    }

    async fn screenshot(&self, path: &std::path::Path) -> Outcome {
        let err = fail;
        if !path.is_absolute() {
            return err(ErrorKind::InvalidArgs, "screenshot path must be absolute");
        }
        let stream = {
            let i = self.lock();
            if i.observed == FocusState::Off || i.requested == FocusState::Off {
                return err(
                    ErrorKind::Other,
                    "Don't have video focus - nothing to screenshot.",
                );
            }
            if !i.have_keyframe || i.sps.is_empty() || i.pps.is_empty() {
                return err(
                    ErrorKind::Unavailable,
                    "no complete picture has arrived from the phone yet",
                );
            }
            let mut s = i.sps.clone();
            s.extend_from_slice(&i.pps);
            s.extend_from_slice(&i.gop);
            s
        };
        match screenshot::decode_to_png(stream, path).await {
            Ok(r) => Outcome::Ok(Reply::Screenshot(r)),
            Err(e) => Outcome::Err(e),
        }
    }

    fn on_frame(&self, data: &[u8]) {
        let picture_ready = {
            let mut i = self.lock();
            Self::take_frame(&mut i, data);
            // Sent under the same lock that updated the buffers, so a viewer that joins gets every
            // frame exactly once: either in its first burst or live.
            if i.streaming {
                if i.burst_pending {
                    // A viewer is waiting for its start: this frame is in the burst, not sent twice.
                    Self::send_burst(&mut i);
                } else {
                    crate::emit_video(data, false);
                }
            }
            i.have_keyframe && !i.sps.is_empty() && !i.pps.is_empty()
        };
        // "Ready" means a screenshot is possible: the phone said it is projecting AND a decodable
        // picture has arrived. The picture comes about 3 s after the phone's start message, and a
        // screenshot taken in between would fail.
        if picture_ready && !self.ready.swap(true, Ordering::SeqCst) {
            crate::emit(&Line::Event(Event::Ready {
                backend_version: BACKEND_VERSION.into(),
                backend_build: BACKEND_BUILD.into(),
            }));
        }
    }

    /// Start a viewer: parameter sets first, then the last keyframe and what followed. Without a
    /// keyframe yet it does nothing and tries again at the next frame.
    fn send_burst(i: &mut Inner) {
        if !i.have_keyframe || i.sps.is_empty() || i.pps.is_empty() {
            return;
        }
        let mut burst = i.sps.clone();
        burst.extend_from_slice(&i.pps);
        burst.extend_from_slice(&i.gop);
        crate::emit_video(&burst, true);
        i.burst_pending = false;
    }

    fn take_frame(i: &mut Inner, data: &[u8]) {
        i.frames += 1;
        // A buffer may hold one NAL unit or several (the phone can send SPS, PPS and the keyframe
        // together), so each is handled on its own. Judging by the first byte alone missed keyframes
        // that arrived behind a parameter set.
        for nal in nal_units(data) {
            match nal.kind {
                7 => i.sps = nal.bytes.to_vec(),
                8 => i.pps = nal.bytes.to_vec(),
                5 => {
                    i.gop.clear();
                    i.gop.extend_from_slice(nal.bytes);
                    i.have_keyframe = true;
                }
                _ if i.have_keyframe => {
                    if i.gop.len() + nal.bytes.len() > GOP_CAP {
                        i.gop.clear();
                        i.have_keyframe = false;
                    } else {
                        i.gop.extend_from_slice(nal.bytes);
                    }
                }
                _ => {}
            }
        }
    }
}

fn audio_slot(c: AudioChannel) -> usize {
    AudioChannel::ALL.iter().position(|a| *a == c).unwrap_or(0)
}

/// One NAL unit of an Annex-B stream, with its start code.
struct Nal<'a> {
    kind: u8,
    bytes: &'a [u8],
}

/// Split an Annex-B buffer at its start codes (`00 00 01` or `00 00 00 01`).
fn nal_units(data: &[u8]) -> Vec<Nal<'_>> {
    let mut starts = Vec::new();
    let mut n = 0;
    while n + 3 <= data.len() {
        if data[n..n + 3] == [0, 0, 1] {
            // A four-byte start code begins one byte earlier.
            starts.push(if n > 0 && data[n - 1] == 0 { n - 1 } else { n });
            n += 3;
        } else {
            n += 1;
        }
    }
    let mut out = Vec::new();
    for (idx, &start) in starts.iter().enumerate() {
        let end = starts.get(idx + 1).copied().unwrap_or(data.len());
        // The NAL header is the byte after the `01` that ends the start code.
        let header = data[start..end]
            .iter()
            .position(|&b| b == 1)
            .map(|p| start + p + 1);
        if let Some(h) = header.filter(|&h| h < end) {
            out.push(Nal {
                kind: data[h] & 0x1f,
                bytes: &data[start..end],
            });
        }
    }
    out
}

/// The point `n` of `steps` of the way from `a` to `b`.
fn lerp(a: u32, b: u32, n: u64, steps: u64) -> u32 {
    let (a, b) = (i64::from(a), i64::from(b));
    (a + (b - a) * n as i64 / steps as i64) as u32
}

fn fail(kind: ErrorKind, message: impl Into<String>) -> Outcome {
    Outcome::Err(ErrorInfo {
        kind,
        message: message.into(),
    })
}

fn word(f: FocusState) -> &'static str {
    match f {
        FocusState::On => "on",
        FocusState::Off => "off",
        FocusState::Unknown => "unknown",
    }
}

#[derive(Clone)]
pub struct Hu(pub Arc<Shared>);

impl std::ops::Deref for Hu {
    type Target = Shared;
    fn deref(&self) -> &Shared {
        &self.0
    }
}

const CERT_ENV: &str = "AUTOLAB_NATIVE_CERT";
const KEY_ENV: &str = "AUTOLAB_NATIVE_KEY";

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The head unit's own TLS credential, when both paths are set. Neither path means the vendored
/// certificate. One path without the other is a configuration error.
fn present(path: Option<&str>) -> Option<&str> {
    path.map(str::trim).filter(|path| !path.is_empty())
}

fn load_custom_certificate(
    cert_path: Option<&str>,
    key_path: Option<&str>,
) -> Result<Option<(Vec<u8>, Vec<u8>)>, String> {
    match (present(cert_path), present(key_path)) {
        (None, None) => Ok(None),
        (Some(cert), Some(key)) => Ok(Some((read_pem(CERT_ENV, cert)?, read_pem(KEY_ENV, key)?))),
        (Some(_), None) => Err(format!(
            "{CERT_ENV} is set but {KEY_ENV} is not; set both paths, or neither"
        )),
        (None, Some(_)) => Err(format!(
            "{KEY_ENV} is set but {CERT_ENV} is not; set both paths, or neither"
        )),
    }
}

fn read_pem(name: &str, path: &str) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {name} ({path}): {e}"))?;
    if !String::from_utf8_lossy(&bytes).contains("-----BEGIN ") {
        return Err(format!(
            "{name} ({path}) is not a PEM file; expected a -----BEGIN line"
        ));
    }
    Ok(bytes)
}

pub async fn run(hu: Arc<Shared>, setup: &AndroidAutoSetup) -> Result<(), String> {
    // How this head unit introduces itself to the phone. Android Auto may treat head units differently
    // (for example which apps it lists), so AUTOLAB_NATIVE_IDENTITY=dhu presents it as Google's DHU.
    let as_dhu = std::env::var("AUTOLAB_NATIVE_IDENTITY").is_ok_and(|v| v == "dhu");
    let unit = if as_dhu {
        // Field for field what Google's DHU 2.0 sends, as Android Auto logs it.
        HeadUnitInfo {
            name: "Desktop Head Unit".into(),
            car_model: "Desktop Head Unit".into(),
            car_year: "2015".into(),
            car_serial: "0".into(),
            left_hand: true,
            head_manufacturer: "Google".into(),
            head_model: "Desktop Head Unit".into(),
            sw_build: "2022-03-30-438482292".into(),
            sw_version: "2.0-mac-arm64".into(),
            native_media: true,
            hide_clock: Some(true),
        }
    } else {
        HeadUnitInfo {
            name: "AutoLab".into(),
            car_model: "autolab-native-hu".into(),
            car_year: "2026".into(),
            car_serial: "1".into(),
            left_hand: false,
            head_manufacturer: "AutoLab".into(),
            head_model: "autolab-native-hu".into(),
            sw_build: "1".into(),
            sw_version: env!("CARGO_PKG_VERSION").into(),
            native_media: true,
            hide_clock: Some(true),
        }
    };
    let config = AndroidAutoConfiguration {
        unit,
        custom_certificate: load_custom_certificate(
            env_value(CERT_ENV).as_deref(),
            env_value(KEY_ENV).as_deref(),
        )?,
    };
    let mut tasks = tokio::task::JoinSet::new();
    Box::new(Hu(hu)).run(config, &mut tasks, setup).await
}

#[async_trait::async_trait]
impl AndroidAutoVideoChannelTrait for Hu {
    async fn receive_video(&self, data: Vec<u8>, _timestamp: Option<u64>) {
        self.on_frame(&data);
    }
    async fn setup_video(&self) -> Result<(), ()> {
        Ok(())
    }
    async fn teardown_video(&self) {}
    async fn wait_for_focus(&self) {
        // The library answers the setup request by granting focus right after this returns.
        self.lock().reported = FocusState::On;
    }
    async fn set_focus(&self, _focus: bool) {}
    fn retrieve_video_configuration(&self) -> &VideoConfiguration {
        &self.video
    }
    async fn phone_video_started(&self, _session: i32) {
        {
            let mut i = self.lock();
            i.observed = FocusState::On;
            i.phone_events += 1;
        }
        crate::emit(&Line::Event(Event::Video {
            state: FocusState::On,
        }));
    }
    async fn phone_video_stopped(&self) {
        {
            let mut i = self.lock();
            i.observed = FocusState::Off;
            i.phone_events += 1;
        }
        crate::emit(&Line::Event(Event::Video {
            state: FocusState::Off,
        }));
    }
}

#[async_trait::async_trait]
impl AndroidAutoSensorTrait for Hu {
    fn get_supported_sensors(&self) -> &SensorInformation {
        &self.sensors
    }
    async fn start_sensor(&self, stype: Wifi::sensor_type::Enum) -> Result<(), ()> {
        let mut m = Wifi::SensorEventIndication::new();
        match stype {
            Wifi::sensor_type::Enum::DRIVING_STATUS => {
                let mut d = Wifi::DrivingStatus::new();
                d.set_status(Wifi::DrivingStatusEnum::UNRESTRICTED as i32);
                m.driving_status.push(d);
            }
            Wifi::sensor_type::Enum::NIGHT_DATA => {
                let mut d = Wifi::NightMode::new();
                d.set_is_night(false);
                m.night_mode.push(d);
            }
            _ => return Err(()),
        }
        self.tx
            .send(AndroidAutoMessage::Sensor(m).sendable())
            .await
            .map_err(|_| ())
    }
}

#[async_trait::async_trait]
impl AndroidAutoAudioOutputTrait for Hu {
    async fn open_output_channel(&self, _t: AudioChannelType) -> Result<(), ()> {
        Ok(())
    }
    async fn close_output_channel(&self, _t: AudioChannelType) -> Result<(), ()> {
        Ok(())
    }
    async fn receive_output_audio(&self, t: AudioChannelType, data: Vec<u8>) {
        let channel = match t {
            AudioChannelType::Media => AudioChannel::Media,
            AudioChannelType::System => AudioChannel::System,
            AudioChannelType::Speech => AudioChannel::Speech,
        };
        let slot = audio_slot(channel);
        let mut i = self.lock();
        i.audio_bytes[slot] += data.len() as u64;
        if i.audio_streaming[slot] {
            crate::emit_audio(channel, &data);
        }
    }
    async fn start_output_audio(&self, _t: AudioChannelType) {}
    async fn stop_output_audio(&self, _t: AudioChannelType) {}
}

#[async_trait::async_trait]
impl AndroidAutoInputChannelTrait for Hu {
    async fn binding_request(&self, _code: u32) -> Result<(), ()> {
        Ok(())
    }
    fn retrieve_input_configuration(&self) -> &InputConfiguration {
        &self.input
    }
}

#[async_trait::async_trait]
impl AndroidAutoAudioInputTrait for Hu {
    async fn open_input_channel(&self) -> Result<(), ()> {
        Ok(())
    }
    async fn close_input_channel(&self) -> Result<(), ()> {
        Ok(())
    }
    async fn start_input_audio(&self) {}
    async fn stop_input_audio(&self) {}
    async fn audio_input_ack(&self, _chan: u8, _ack: Wifi::AVMediaAckIndication) {}
}

impl AndroidAutoWiredTrait for Hu {}

#[async_trait::async_trait]
impl AndroidAutoMainTrait for Hu {
    async fn connect(&self) {}
    async fn disconnect(&self) {}
    async fn get_receiver(
        &self,
    ) -> Option<tokio::sync::mpsc::Receiver<SendableAndroidAutoMessage>> {
        self.rx.lock().await.take()
    }
    fn supports_wired(&self) -> Option<Arc<dyn AndroidAutoWiredTrait>> {
        Some(Arc::new(self.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neither_certificate_path_keeps_the_built_in_certificate() {
        assert_eq!(load_custom_certificate(None, None).unwrap(), None);
        assert_eq!(load_custom_certificate(Some(""), Some("  ")).unwrap(), None);
    }

    #[test]
    fn one_certificate_path_without_the_other_is_refused() {
        let missing_key = load_custom_certificate(Some("cert.pem"), None).unwrap_err();
        assert!(missing_key.contains(KEY_ENV), "{missing_key}");
        let missing_cert = load_custom_certificate(None, Some("key.pem")).unwrap_err();
        assert!(missing_cert.contains(CERT_ENV), "{missing_cert}");
    }

    #[test]
    fn both_pem_files_are_passed_through_and_a_bad_file_is_named() {
        let dir = std::env::temp_dir().join(format!(
            "autolab-cert-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cert = dir.join("head-unit-cert.pem");
        let key = dir.join("head-unit-key.pem");
        std::fs::write(&cert, b"-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n")
            .unwrap();
        std::fs::write(&key, b"not a pem file").unwrap();
        let bad = load_custom_certificate(cert.to_str(), key.to_str()).unwrap_err();
        assert!(bad.contains(KEY_ENV) && bad.contains("PEM"), "{bad}");

        std::fs::write(&key, b"-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----\n")
            .unwrap();
        let (loaded_cert, loaded_key) =
            load_custom_certificate(cert.to_str(), key.to_str())
                .unwrap()
                .expect("both files are present");
        assert!(loaded_cert.starts_with(b"-----BEGIN CERTIFICATE-----"));
        assert!(loaded_key.starts_with(b"-----BEGIN PRIVATE KEY-----"));

        let missing = load_custom_certificate(
            Some(dir.join("missing-cert.pem").to_str().unwrap()),
            key.to_str(),
        )
        .unwrap_err();
        assert!(missing.contains(CERT_ENV), "{missing}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lerp_runs_from_start_to_end_in_either_direction() {
        assert_eq!(lerp(0, 100, 0, 4), 0);
        assert_eq!(lerp(0, 100, 2, 4), 50);
        assert_eq!(lerp(0, 100, 4, 4), 100);
        assert_eq!(lerp(100, 0, 1, 4), 75);
        assert_eq!(lerp(7, 7, 3, 5), 7);
    }

    #[tokio::test]
    async fn touches_outside_the_screen_or_without_video_or_with_a_bad_duration_are_refused() {
        let hu = Shared::new();
        let outside = hu
            .handle(Command::Tap {
                x: SCREEN_WIDTH.into(),
                y: 0,
            })
            .await;
        match outside {
            Outcome::Err(e) => {
                assert_eq!(e.kind, ErrorKind::InvalidArgs);
                assert!(e.message.contains("outside the 800x480 screen"), "{e:?}");
            }
            _ => panic!("must refuse"),
        }
        let none = hu.handle(Command::Tap { x: 1, y: 1 }).await;
        assert!(matches!(
            none,
            Outcome::Err(ErrorInfo {
                kind: ErrorKind::Other,
                ..
            })
        ));
        for ms in [0, SWIPE_MIN_MS - 1, SWIPE_MAX_MS + 1] {
            let r = hu
                .handle(Command::Swipe {
                    x1: 0,
                    y1: 0,
                    x2: 5,
                    y2: 5,
                    duration_ms: ms,
                })
                .await;
            assert!(
                matches!(
                    r,
                    Outcome::Err(ErrorInfo {
                        kind: ErrorKind::InvalidArgs,
                        ..
                    })
                ),
                "{ms} ms"
            );
        }
    }

    #[tokio::test]
    async fn a_button_press_sends_down_then_up_and_is_refused_without_video() {
        let hu = Shared::new();
        let refused = hu
            .handle(Command::Key {
                key: Key::Home,
                long_press: false,
            })
            .await;
        assert!(matches!(
            refused,
            Outcome::Err(ErrorInfo {
                kind: ErrorKind::Other,
                ..
            })
        ));
        hu.lock().observed = FocusState::On;
        let mut rx = hu.rx.lock().await.take().unwrap();
        let Outcome::Ok(Reply::Input(r)) = hu
            .handle(Command::Key {
                key: Key::Back,
                long_press: false,
            })
            .await
        else {
            panic!("a key press must be accepted");
        };
        assert_eq!(r.status, OpStatus::Accepted);
        assert!(
            r.evidence[0].contains("back") && r.evidence[0].contains("code 4"),
            "{:?}",
            r.evidence
        );
        let mut sent = 0;
        while rx.try_recv().is_ok() {
            sent += 1;
        }
        assert_eq!(sent, 2, "a press is a down and an up");
    }

    #[test]
    fn a_viewer_that_asks_before_a_keyframe_gets_its_burst_when_one_arrives() {
        let hu = Shared::new();
        {
            let mut i = hu.lock();
            i.streaming = true;
            i.burst_pending = true;
            Shared::send_burst(&mut i); // nothing to send yet
            assert!(i.burst_pending, "still waiting for a keyframe");
        }
        hu.on_frame(&SPS);
        hu.on_frame(&PPS);
        assert!(hu.lock().burst_pending);
        hu.on_frame(&IDR);
        assert!(!hu.lock().burst_pending, "the keyframe completed the burst");
    }

    #[test]
    fn every_button_is_declared_to_the_phone() {
        let hu = Shared::new();
        for k in Key::ALL {
            assert!(
                hu.input.keycodes.contains(&k.android_code()),
                "{k} is not declared"
            );
        }
    }

    #[tokio::test]
    async fn a_tap_sends_down_then_up_and_a_swipe_drags_in_between() {
        let hu = Shared::new();
        hu.lock().observed = FocusState::On;
        let mut rx = hu.rx.lock().await.take().unwrap();
        let Outcome::Ok(Reply::Input(r)) = hu.handle(Command::Tap { x: 10, y: 20 }).await else {
            panic!("tap must be accepted");
        };
        assert_eq!(r.status, OpStatus::Accepted);
        assert!(
            r.evidence[0].contains("tap at (10, 20)"),
            "{:?}",
            r.evidence
        );
        let mut sent = 0;
        while rx.try_recv().is_ok() {
            sent += 1;
        }
        assert_eq!(sent, 2, "a tap is a down and an up");
        let Outcome::Ok(Reply::Input(_)) = hu
            .handle(Command::Swipe {
                x1: 0,
                y1: 0,
                x2: 100,
                y2: 0,
                duration_ms: 100,
            })
            .await
        else {
            panic!("swipe must be accepted");
        };
        let mut sent = 0;
        while rx.try_recv().is_ok() {
            sent += 1;
        }
        assert!(sent >= 4, "down, several drags, up: {sent}");
    }

    const SPS: [u8; 8] = [0, 0, 0, 1, 0x67, 0x42, 0, 0x1f];
    const PPS: [u8; 7] = [0, 0, 0, 1, 0x68, 0xce, 0x3c];
    const IDR: [u8; 7] = [0, 0, 0, 1, 0x65, 0xb8, 0x4f];
    const P: [u8; 7] = [0, 0, 0, 1, 0x21, 0xe2, 0x05];

    #[test]
    fn the_buffer_for_a_screenshot_is_config_then_the_last_keyframe_and_what_followed() {
        let hu = Shared::new();
        hu.on_frame(&SPS);
        hu.on_frame(&PPS);
        hu.on_frame(&P); // before any keyframe: not decodable, so not kept
        hu.on_frame(&IDR);
        hu.on_frame(&P);
        hu.on_frame(&IDR); // a new keyframe starts over
        hu.on_frame(&P);
        let i = hu.lock();
        assert!(i.have_keyframe);
        assert_eq!(i.frames, 7);
        assert_eq!((i.sps.as_slice(), i.pps.as_slice()), (&SPS[..], &PPS[..]));
        let mut gop = IDR.to_vec();
        gop.extend_from_slice(&P);
        assert_eq!(i.gop, gop);
    }

    #[test]
    fn a_keyframe_that_arrives_behind_the_parameter_sets_in_one_buffer_is_found() {
        // Regression: judging a buffer by its first NAL unit missed this, and screenshots failed with
        // "no complete picture has arrived" right after a focus change.
        let hu = Shared::new();
        let mut all = SPS.to_vec();
        all.extend_from_slice(&PPS);
        all.extend_from_slice(&IDR);
        hu.on_frame(&all);
        let i = hu.lock();
        assert!(i.have_keyframe);
        assert_eq!((i.sps.as_slice(), i.pps.as_slice()), (&SPS[..], &PPS[..]));
        assert_eq!(i.gop, IDR);
    }

    #[test]
    fn the_head_unit_is_ready_only_when_a_decodable_picture_has_arrived() {
        // Regression: ready used to fire on the phone's start message, about 3 s before the first
        // picture, so a screenshot right after `autolab start` failed.
        let hu = Shared::new();
        hu.on_frame(&SPS);
        hu.on_frame(&PPS);
        assert!(
            !hu.ready.load(Ordering::SeqCst),
            "parameter sets alone are not a picture"
        );
        hu.on_frame(&P);
        assert!(!hu.ready.load(Ordering::SeqCst));
        hu.on_frame(&IDR);
        assert!(hu.ready.load(Ordering::SeqCst));
    }

    #[test]
    fn nal_units_are_split_at_three_and_four_byte_start_codes() {
        let mut buf = vec![0, 0, 1, 0x67, 9];
        buf.extend_from_slice(&[0, 0, 0, 1, 0x68, 8]);
        buf.extend_from_slice(&[0, 0, 1, 0x65, 7, 7]);
        let kinds: Vec<(u8, usize)> = nal_units(&buf)
            .iter()
            .map(|n| (n.kind, n.bytes.len()))
            .collect();
        assert_eq!(kinds, [(7, 5), (8, 6), (5, 6)]);
        assert!(nal_units(&[1, 2, 3]).is_empty());
        assert!(
            nal_units(&[0, 0, 1]).is_empty(),
            "a start code with no header byte"
        );
    }

    #[tokio::test]
    async fn screenshots_are_refused_without_video_or_a_keyframe() {
        let hu = Shared::new();
        let path = std::path::Path::new("/tmp/x.png");
        let none = hu.screenshot(path).await;
        assert!(matches!(
            none,
            Outcome::Err(ErrorInfo {
                kind: ErrorKind::Unavailable,
                ..
            })
        ));
        hu.on_frame(&SPS);
        hu.on_frame(&IDR);
        hu.lock().observed = FocusState::Off;
        let off = hu.screenshot(path).await;
        match off {
            Outcome::Err(e) => assert!(e.message.contains("nothing to screenshot"), "{e:?}"),
            _ => panic!("must refuse while video is off"),
        }
        let rel = hu.screenshot(std::path::Path::new("x.png")).await;
        assert!(matches!(
            rel,
            Outcome::Err(ErrorInfo {
                kind: ErrorKind::InvalidArgs,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn focus_that_the_phone_never_confirms_is_a_timeout_not_a_success() {
        let hu = Shared::new();
        hu.lock().observed = FocusState::On;
        // The channel has a receiver, so the send works, but no phone answers.
        let Outcome::Ok(Reply::Focus(r)) = hu.focus(false).await else {
            panic!("expected a focus reply");
        };
        assert_eq!(r.status, OpStatus::TimedOut);
        assert_eq!(r.requested, FocusState::Off);
        assert_eq!(
            r.observed,
            FocusState::On,
            "observed is the phone's word, not ours"
        );
    }

    #[tokio::test]
    async fn focus_the_phone_confirms_is_observed() {
        let hu = Arc::new(Shared::new());
        hu.lock().observed = FocusState::On;
        let h2 = Arc::clone(&hu);
        let phone = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let mut i = h2.lock();
            i.observed = FocusState::Off;
            i.phone_events += 1;
        });
        let Outcome::Ok(Reply::Focus(r)) = hu.focus(false).await else {
            panic!("expected a focus reply");
        };
        phone.await.unwrap();
        assert_eq!(r.status, OpStatus::Observed);
        assert_eq!(r.observed, FocusState::Off);
    }

    #[tokio::test]
    async fn asking_for_the_state_the_phone_already_reported_is_observed_with_a_note() {
        let hu = Shared::new();
        hu.lock().observed = FocusState::Off;
        let Outcome::Ok(Reply::Focus(r)) = hu.focus(false).await else {
            panic!("expected a focus reply");
        };
        // No message is expected here, but we still wait out the confirmation window only if needed.
        assert_eq!(r.status, OpStatus::Observed);
        assert!(
            r.evidence.iter().any(|e| e.contains("already reported")),
            "{:?}",
            r.evidence
        );
    }
}

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
use autolab_core::{FocusState, OpStatus, OperationResult, VideoFocusStatus};
use autolab_native::protocol::{
    Command, ErrorInfo, ErrorKind, Event, Line, Outcome, Reply, StatusReport,
};
use std::sync::Arc;

use crate::screenshot;

/// How long to wait for the phone to confirm a focus change. Measured on a Pixel 6: about 0.5 s
/// for a release and a few ms for a take-back.
const CONFIRM_WAIT: Duration = Duration::from_secs(4);
/// Frames since the last keyframe are kept for screenshots. A phone that sends this much without a
/// keyframe is not behaving, so the buffer is dropped and a new keyframe awaited.
const GOP_CAP: usize = 16 * 1024 * 1024;

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
    config: Vec<u8>,
    /// Every frame since the last keyframe, including it.
    gop: Vec<u8>,
    have_keyframe: bool,
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
                config: Vec::new(),
                gop: Vec::new(),
                have_keyframe: false,
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
                keycodes: vec![1, 2, 3, 4, 5],
                touchscreen: Some((800, 480)),
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
                }))
            }
            Command::Focus { enabled } => self.focus(enabled).await,
            Command::Screenshot { path } => self.screenshot(&path).await,
            Command::Quit => Outcome::Ok(Reply::Done),
        }
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
        let err = |kind, message: &str| {
            Outcome::Err(ErrorInfo {
                kind,
                message: message.into(),
            })
        };
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
            if !i.have_keyframe || i.config.is_empty() {
                return err(
                    ErrorKind::Unavailable,
                    "no complete picture has arrived from the phone yet",
                );
            }
            let mut s = i.config.clone();
            s.extend_from_slice(&i.gop);
            s
        };
        match screenshot::decode_to_png(stream, path).await {
            Ok(r) => Outcome::Ok(Reply::Screenshot(r)),
            Err(e) => Outcome::Err(e),
        }
    }

    fn on_frame(&self, data: &[u8]) {
        let mut i = self.lock();
        i.frames += 1;
        // The first NAL unit's type is in the low five bits of the byte after the start code.
        let nal = data.get(4).map(|b| b & 0x1f);
        match nal {
            Some(7) => i.config = data.to_vec(),
            Some(8) if !i.config.is_empty() && !contains_pps(&i.config) => {
                i.config.extend_from_slice(data);
            }
            Some(5) => {
                i.gop.clear();
                i.gop.extend_from_slice(data);
                i.have_keyframe = true;
            }
            _ if i.have_keyframe => {
                if i.gop.len() + data.len() > GOP_CAP {
                    i.gop.clear();
                    i.have_keyframe = false;
                } else {
                    i.gop.extend_from_slice(data);
                }
            }
            _ => {}
        }
    }
}

fn word(f: FocusState) -> &'static str {
    match f {
        FocusState::On => "on",
        FocusState::Off => "off",
        FocusState::Unknown => "unknown",
    }
}

/// Whether an Annex-B buffer already holds a PPS (NAL type 8).
fn contains_pps(buf: &[u8]) -> bool {
    buf.windows(5)
        .any(|w| w[..4] == [0, 0, 0, 1] && w[4] & 0x1f == 8)
}

#[derive(Clone)]
pub struct Hu(pub Arc<Shared>);

impl std::ops::Deref for Hu {
    type Target = Shared;
    fn deref(&self) -> &Shared {
        &self.0
    }
}

pub async fn run(hu: Arc<Shared>, setup: &AndroidAutoSetup) -> Result<(), String> {
    let config = AndroidAutoConfiguration {
        unit: HeadUnitInfo {
            name: "AutoLab".into(),
            car_model: "AutoLab".into(),
            car_year: "2026".into(),
            car_serial: "1".into(),
            left_hand: false,
            head_manufacturer: "AutoLab".into(),
            head_model: "autolab-native-hu".into(),
            sw_build: "1".into(),
            sw_version: env!("CARGO_PKG_VERSION").into(),
            native_media: true,
            hide_clock: Some(true),
        },
        custom_certificate: None,
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
        if !self.ready.swap(true, Ordering::SeqCst) {
            crate::emit(&Line::Event(Event::Ready {
                backend_version: BACKEND_VERSION.into(),
                backend_build: BACKEND_BUILD.into(),
            }));
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
    async fn receive_output_audio(&self, _t: AudioChannelType, _data: Vec<u8>) {}
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
        let mut expect = SPS.to_vec();
        expect.extend_from_slice(&PPS);
        assert_eq!(i.config, expect);
        let mut gop = IDR.to_vec();
        gop.extend_from_slice(&P);
        assert_eq!(i.gop, gop);
    }

    #[test]
    fn a_pps_is_not_added_twice() {
        let hu = Shared::new();
        let mut both = SPS.to_vec();
        both.extend_from_slice(&PPS);
        hu.on_frame(&both);
        hu.on_frame(&PPS);
        assert_eq!(hu.lock().config, both);
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

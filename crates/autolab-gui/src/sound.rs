//! Playing the phone's audio: one `ffplay` per channel, fed with the daemon's raw PCM.
//!
//! A player starts only when sound actually arrives on its channel (the phone sends nothing while a
//! channel is silent), and stops when the window stops wanting audio or the session ends.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use autolab_core::AudioChannel;
use autolab_daemon::Client;

const RETRY: Duration = Duration::from_secs(2);

struct Channel {
    channel: AudioChannel,
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    bytes: Arc<AtomicU64>,
    problem: Arc<Mutex<Option<String>>>,
    last_try: Option<Instant>,
}

pub struct Sound {
    channels: Vec<Channel>,
}

impl Sound {
    pub fn new() -> Self {
        Self {
            channels: AudioChannel::ALL
                .into_iter()
                .map(|channel| Channel {
                    channel,
                    stop: Arc::default(),
                    running: Arc::default(),
                    bytes: Arc::default(),
                    problem: Arc::default(),
                    last_try: None,
                })
                .collect(),
        }
    }

    /// Start the listeners when audio is wanted and stop them when it is not.
    pub fn tick(&mut self, want: bool) {
        for c in &mut self.channels {
            let running = c.running.load(Ordering::SeqCst);
            if !want {
                if running {
                    c.stop.store(true, Ordering::SeqCst);
                }
                continue;
            }
            if running || c.last_try.is_some_and(|t| t.elapsed() < RETRY) {
                continue;
            }
            c.last_try = Some(Instant::now());
            c.stop = Arc::new(AtomicBool::new(false));
            c.running.store(true, Ordering::SeqCst);
            *c.problem.lock().unwrap_or_else(|e| e.into_inner()) = None;
            let (channel, stop, running, bytes, problem) = (
                c.channel,
                Arc::clone(&c.stop),
                Arc::clone(&c.running),
                Arc::clone(&c.bytes),
                Arc::clone(&c.problem),
            );
            thread::spawn(move || {
                let result = listen(channel, &stop, &bytes);
                *problem.lock().unwrap_or_else(|e| e.into_inner()) = result.err();
                running.store(false, Ordering::SeqCst);
            });
        }
    }

    /// One line for the window: how much sound has arrived on each channel, and any problem.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = self
            .channels
            .iter()
            .map(|c| {
                let b = c.bytes.load(Ordering::SeqCst);
                format!(
                    "{} {:.1} s",
                    c.channel,
                    b as f64 / f64::from(c.channel.bytes_per_second())
                )
            })
            .collect();
        if let Some(p) = self
            .channels
            .iter()
            .find_map(|c| c.problem.lock().unwrap_or_else(|e| e.into_inner()).clone())
        {
            parts.push(format!("problem: {p}"));
        }
        format!("audio received: {}", parts.join(", "))
    }
}

/// True if any sample is not zero. A channel that is open but silent can still send zeros.
fn has_sound(pcm: &[u8]) -> bool {
    pcm.iter().any(|b| *b != 0)
}

fn listen(channel: AudioChannel, stop: &AtomicBool, bytes: &AtomicU64) -> Result<(), String> {
    let client = Client::connect().map_err(|e| e.to_string())?;
    let mut audio = client.audio(channel).map_err(|e| e.to_string())?;
    let mut player: Option<std::process::Child> = None;
    let mut buf = vec![0u8; 16 * 1024];
    let outcome = loop {
        if stop.load(Ordering::SeqCst) {
            break Ok(());
        }
        let n = match audio.read(&mut buf) {
            Ok(0) => break Err("the audio stream ended".to_string()),
            Ok(n) => n,
            Err(e) => break Err(format!("the audio stream broke: {e}")),
        };
        bytes.fetch_add(n as u64, Ordering::SeqCst);
        if player.is_none() && has_sound(&buf[..n]) {
            player = Some(spawn_player(channel)?);
        }
        if let Some(p) = &mut player
            && let Some(stdin) = p.stdin.as_mut()
            && stdin.write_all(&buf[..n]).is_err()
        {
            let child = player.take().expect("checked above");
            break Err(why_it_stopped(child));
        }
    };
    if let Some(mut p) = player {
        drop(p.stdin.take());
        let _ = p.kill();
        let _ = p.wait();
    }
    outcome
}

fn spawn_player(channel: AudioChannel) -> Result<std::process::Child, String> {
    let ffmpeg = autolab_native::find_ffmpeg()
        .ok_or("ffmpeg not found; ffplay plays the audio (macOS: brew install ffmpeg)")?;
    let ffplay = ffmpeg.with_file_name("ffplay");
    let mut cmd = Command::new(&ffplay);
    // Errors are kept (not discarded) so the window can say why ffplay stopped.
    cmd.args(["-nodisp", "-loglevel", "error", "-nostats", "-f", "s16le"])
        .args(["-ar", &channel.sample_rate().to_string()]);
    if supports_ch_layout(&ffplay) {
        cmd.args([
            "-ch_layout",
            if channel.channels() == 1 {
                "mono"
            } else {
                "stereo"
            },
        ]);
    } else {
        cmd.args(["-ac", &channel.channels().to_string()]);
    }
    cmd.args(["-i", "pipe:0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run ffplay: {e}"))
}

/// ffplay 7 and later dropped `-ac` for raw input and use `-ch_layout` (added in 5.1). Anything whose
/// version cannot be read is assumed to be new.
fn supports_ch_layout(ffplay: &std::path::Path) -> bool {
    let out = Command::new(ffplay).arg("-version").output();
    let text = out
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    ch_layout_for_version(text.lines().next().unwrap_or(""))
}

fn ch_layout_for_version(first_line: &str) -> bool {
    let version = first_line
        .split_whitespace()
        .nth(2)
        .map(|v| v.trim_start_matches(['n', 'N']))
        .unwrap_or("");
    match version
        .split('.')
        .next()
        .and_then(|m| m.parse::<u32>().ok())
    {
        Some(major) => major >= 6,
        None => true,
    }
}

/// What ffplay printed before it stopped, for the window's problem line.
fn why_it_stopped(mut child: std::process::Child) -> String {
    let _ = child.wait();
    let mut text = String::new();
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut text);
    }
    match text.lines().find(|l| !l.trim().is_empty()) {
        Some(line) => format!("ffplay stopped: {}", line.trim()),
        None => "ffplay stopped".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_non_zero_samples_count_as_sound() {
        assert!(!has_sound(&[0; 100]));
        assert!(has_sound(&[0, 0, 1, 0]));
        assert!(!has_sound(&[]));
    }

    #[test]
    fn newer_ffplay_gets_ch_layout_and_older_gets_ac() {
        assert!(ch_layout_for_version(
            "ffplay version 9.0.2 Copyright (c) 2003-2026 the FFmpeg developers"
        ));
        assert!(ch_layout_for_version("ffplay version 7.1 Copyright"));
        assert!(ch_layout_for_version("ffplay version n6.1.1 Copyright"));
        assert!(!ch_layout_for_version("ffplay version 4.4.2 Copyright"));
        assert!(!ch_layout_for_version("ffplay version n5.0 Copyright"));
        assert!(
            ch_layout_for_version("ffplay version N-123456-gabcdef Copyright"),
            "git builds count as new"
        );
        assert!(ch_layout_for_version(""), "unreadable counts as new");
    }

    #[test]
    fn the_summary_names_every_channel_with_seconds_of_sound() {
        let s = Sound::new();
        let line = s.summary();
        assert!(
            line.contains("media 0.0 s")
                && line.contains("system 0.0 s")
                && line.contains("speech 0.0 s"),
            "{line}"
        );
    }
}

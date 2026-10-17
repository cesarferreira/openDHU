//! `aa audio`: record what the phone plays through the car to a WAV file.
//!
//! This is also how to check audio without listening: the summary says how much sound arrived and how
//! loud it was.

use std::io::Read;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use opendhu_core::{AudioChannel, Error, Result};
use serde::Serialize;

use crate::daemon_ctl;

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub path: String,
    pub channel: AudioChannel,
    pub format: String,
    /// Bytes of PCM that arrived.
    pub bytes: u64,
    /// Seconds of sound that arrived. The phone sends nothing while a channel is silent, so this is
    /// usually less than the wall time.
    pub audio_seconds: f64,
    pub wall_seconds: f64,
    /// Loudest sample and root-mean-square level, each from 0 (silence) to 1 (full scale).
    pub peak: f64,
    pub rms: f64,
}

/// A 44-byte RIFF/WAVE header for 16-bit PCM.
pub fn wav_header(rate: u32, channels: u32, data_len: u32) -> Vec<u8> {
    let (bits, block) = (16u16, (channels * 2) as u16);
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + data_len).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes()); // PCM
    h.extend_from_slice(&(channels as u16).to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
    h.extend_from_slice(&block.to_le_bytes());
    h.extend_from_slice(&bits.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data_len.to_le_bytes());
    h
}

/// Peak and RMS of 16-bit little-endian samples, as fractions of full scale.
pub fn levels(pcm: &[u8]) -> (f64, f64) {
    let samples: Vec<f64> = pcm
        .chunks_exact(2)
        .map(|b| f64::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0)
        .collect();
    if samples.is_empty() {
        return (0.0, 0.0);
    }
    let peak = samples.iter().fold(0.0f64, |m, s| m.max(s.abs()));
    let rms = (samples.iter().map(|s| s * s).sum::<f64>() / samples.len() as f64).sqrt();
    (peak, rms)
}

pub fn record(channel: AudioChannel, seconds: u64, out: &Path) -> Result<Summary> {
    let client = daemon_ctl::connect()?;
    let mut reader = client.audio(channel)?;
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    // The read blocks while the phone is silent, so a thread reads and this one keeps the clock.
    thread::spawn(move || {
        let mut buf = vec![0u8; 16 * 1024];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let started = Instant::now();
    let deadline = started + Duration::from_secs(seconds);
    let mut pcm = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(chunk) => pcm.extend_from_slice(&chunk),
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let wall = started.elapsed().as_secs_f64();
    let data_len = u32::try_from(pcm.len())
        .map_err(|_| Error::Other("the recording is too large for a WAV file".into()))?;
    let mut file = wav_header(channel.sample_rate(), channel.channels(), data_len);
    file.extend_from_slice(&pcm);
    std::fs::write(out, &file)
        .map_err(|e| Error::Other(format!("cannot write {}: {e}", out.display())))?;
    let (peak, rms) = levels(&pcm);
    Ok(Summary {
        path: out.display().to_string(),
        channel,
        format: channel.format(),
        bytes: pcm.len() as u64,
        audio_seconds: pcm.len() as f64 / f64::from(channel.bytes_per_second()),
        wall_seconds: wall,
        peak,
        rms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_44_bytes_and_describes_the_format() {
        let h = wav_header(48_000, 2, 1000);
        assert_eq!(h.len(), 44);
        assert_eq!(&h[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(h[4..8].try_into().unwrap()), 1036);
        assert_eq!(
            u16::from_le_bytes(h[22..24].try_into().unwrap()),
            2,
            "channels"
        );
        assert_eq!(
            u32::from_le_bytes(h[24..28].try_into().unwrap()),
            48_000,
            "rate"
        );
        assert_eq!(
            u32::from_le_bytes(h[28..32].try_into().unwrap()),
            192_000,
            "bytes/s"
        );
        assert_eq!(
            u32::from_le_bytes(h[40..44].try_into().unwrap()),
            1000,
            "data length"
        );
    }

    #[test]
    fn levels_of_silence_a_square_wave_and_nothing() {
        assert_eq!(levels(&[]), (0.0, 0.0));
        assert_eq!(levels(&[0, 0, 0, 0]), (0.0, 0.0));
        let loud: Vec<u8> = [16384i16, -16384, 16384, -16384]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let (peak, rms) = levels(&loud);
        assert!(
            (peak - 0.5).abs() < 1e-9 && (rms - 0.5).abs() < 1e-9,
            "{peak} {rms}"
        );
    }
}

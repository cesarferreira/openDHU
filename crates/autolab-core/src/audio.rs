//! The audio the phone plays through the car. Android Auto sends raw 16-bit PCM on three channels.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioChannel {
    /// Music, podcasts, video sound.
    Media,
    /// Navigation prompts, notification sounds.
    System,
    /// The assistant's voice.
    Speech,
}

impl AudioChannel {
    pub const ALL: [AudioChannel; 3] = [Self::Media, Self::System, Self::Speech];

    pub fn name(self) -> &'static str {
        match self {
            Self::Media => "media",
            Self::System => "system",
            Self::Speech => "speech",
        }
    }

    /// Samples per second, per channel.
    pub fn sample_rate(self) -> u32 {
        match self {
            Self::Media => 48_000,
            Self::System | Self::Speech => 16_000,
        }
    }

    pub fn channels(self) -> u32 {
        match self {
            Self::Media => 2,
            Self::System | Self::Speech => 1,
        }
    }

    /// Always 16-bit signed little-endian samples.
    pub fn bytes_per_second(self) -> u32 {
        self.sample_rate() * self.channels() * 2
    }

    /// `s16le;rate=48000;channels=2`, for a header or a log line.
    pub fn format(self) -> String {
        format!(
            "s16le;rate={};channels={}",
            self.sample_rate(),
            self.channels()
        )
    }
}

impl fmt::Display for AudioChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for AudioChannel {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|c| c.name() == s)
            .ok_or_else(|| format!("unknown audio channel `{s}`; use media, system or speech"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_match_the_protocol_and_names_round_trip() {
        assert_eq!(AudioChannel::Media.bytes_per_second(), 192_000);
        assert_eq!(AudioChannel::System.bytes_per_second(), 32_000);
        assert_eq!(AudioChannel::Speech.format(), "s16le;rate=16000;channels=1");
        for c in AudioChannel::ALL {
            assert_eq!(c.name().parse::<AudioChannel>().unwrap(), c);
        }
        assert!("music".parse::<AudioChannel>().is_err());
    }
}

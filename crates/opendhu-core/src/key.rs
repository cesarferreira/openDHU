//! The car's buttons. Android Auto sends them to the phone as Android key codes.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    Home,
    Back,
    Up,
    Down,
    Left,
    Right,
    /// The centre of the directional pad, or a click of the rotary dial.
    Enter,
    Search,
    PlayPause,
    Next,
    Previous,
    Call,
    EndCall,
}

impl Key {
    pub const ALL: [Key; 13] = [
        Key::Home,
        Key::Back,
        Key::Up,
        Key::Down,
        Key::Left,
        Key::Right,
        Key::Enter,
        Key::Search,
        Key::PlayPause,
        Key::Next,
        Key::Previous,
        Key::Call,
        Key::EndCall,
    ];

    /// The Android `KeyEvent` code the phone expects.
    pub fn android_code(self) -> u32 {
        match self {
            Self::Home => 3,
            Self::Back => 4,
            Self::Call => 5,
            Self::EndCall => 6,
            Self::Up => 19,
            Self::Down => 20,
            Self::Left => 21,
            Self::Right => 22,
            Self::Enter => 23,
            Self::Search => 84,
            Self::PlayPause => 85,
            Self::Next => 87,
            Self::Previous => 88,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Back => "back",
            Self::Up => "up",
            Self::Down => "down",
            Self::Left => "left",
            Self::Right => "right",
            Self::Enter => "enter",
            Self::Search => "search",
            Self::PlayPause => "play_pause",
            Self::Next => "next",
            Self::Previous => "previous",
            Self::Call => "call",
            Self::EndCall => "end_call",
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Key {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Key::ALL.into_iter().find(|k| k.name() == s).ok_or_else(|| {
            format!(
                "unknown key `{s}`; use one of: {}",
                Key::ALL.map(Key::name).join(", ")
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_round_trips_through_its_name_and_has_a_distinct_android_code() {
        let mut codes = std::collections::HashSet::new();
        for k in Key::ALL {
            assert_eq!(k.name().parse::<Key>().unwrap(), k);
            assert_eq!(
                serde_json::to_string(&k).unwrap(),
                format!("\"{}\"", k.name())
            );
            assert!(codes.insert(k.android_code()), "{k} repeats a code");
        }
    }

    #[test]
    fn android_codes_match_the_android_key_event_constants() {
        assert_eq!(Key::Home.android_code(), 3); // KEYCODE_HOME
        assert_eq!(Key::Back.android_code(), 4); // KEYCODE_BACK
        assert_eq!(Key::Enter.android_code(), 23); // KEYCODE_DPAD_CENTER
        assert_eq!(Key::PlayPause.android_code(), 85); // KEYCODE_MEDIA_PLAY_PAUSE
    }

    #[test]
    fn an_unknown_key_lists_the_valid_ones() {
        let e = "volume".parse::<Key>().unwrap_err();
        assert!(e.contains("home") && e.contains("end_call"), "{e}");
    }
}

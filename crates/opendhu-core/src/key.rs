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
    /// A turn of the rotary dial, one step anticlockwise: focus moves to the previous item.
    RotateLeft,
    /// A turn of the rotary dial, one step clockwise: focus moves to the next item.
    RotateRight,
}

impl Key {
    pub const ALL: [Key; 15] = [
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
        Key::RotateLeft,
        Key::RotateRight,
    ];

    /// The Android `KeyEvent` code the phone expects. Both turns of the dial share
    /// `KEYCODE_ROTARY_CONTROLLER`; [`Key::rotation`] tells them apart.
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
            Self::RotateLeft | Self::RotateRight => 65536,
        }
    }

    /// The dial's steps for a turn (negative is anticlockwise), or `None` for a button. Android Auto
    /// sends a turn as a relative input event, not as a press and release.
    pub fn rotation(self) -> Option<i32> {
        match self {
            Self::RotateLeft => Some(-1),
            Self::RotateRight => Some(1),
            _ => None,
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
            Self::RotateLeft => "rotate_left",
            Self::RotateRight => "rotate_right",
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
    fn every_key_round_trips_through_its_name_and_every_button_has_a_distinct_android_code() {
        let mut codes = std::collections::HashSet::new();
        for k in Key::ALL {
            assert_eq!(k.name().parse::<Key>().unwrap(), k);
            assert_eq!(
                serde_json::to_string(&k).unwrap(),
                format!("\"{}\"", k.name())
            );
            if k.rotation().is_none() {
                assert!(codes.insert(k.android_code()), "{k} repeats a code");
            }
        }
    }

    #[test]
    fn android_codes_match_the_android_key_event_constants() {
        assert_eq!(Key::Home.android_code(), 3); // KEYCODE_HOME
        assert_eq!(Key::Back.android_code(), 4); // KEYCODE_BACK
        assert_eq!(Key::Enter.android_code(), 23); // KEYCODE_DPAD_CENTER
        assert_eq!(Key::PlayPause.android_code(), 85); // KEYCODE_MEDIA_PLAY_PAUSE
        assert_eq!(Key::RotateRight.android_code(), 65536); // KEYCODE_ROTARY_CONTROLLER
    }

    #[test]
    fn only_the_dial_turns_and_left_is_anticlockwise() {
        assert_eq!(Key::RotateLeft.rotation(), Some(-1));
        assert_eq!(Key::RotateRight.rotation(), Some(1));
        assert!(Key::ALL[..13].iter().all(|k| k.rotation().is_none()));
    }

    #[test]
    fn an_unknown_key_lists_the_valid_ones() {
        let e = "volume".parse::<Key>().unwrap_err();
        assert!(e.contains("home") && e.contains("end_call"), "{e}");
    }
}

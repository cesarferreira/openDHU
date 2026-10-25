//! The terminal: raw mode, the alternate screen, mouse reporting, and the input escape sequences.

use std::io::{self, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub cols: u32,
    pub rows: u32,
    pub width_px: u32,
    pub height_px: u32,
}

impl Size {
    /// One cell in pixels. Terminals that do not report their pixel size get a typical 10x20.
    pub fn cell(&self) -> (u32, u32) {
        if self.width_px == 0 || self.height_px == 0 || self.cols == 0 || self.rows == 0 {
            (10, 20)
        } else {
            (
                (self.width_px / self.cols).max(1),
                (self.height_px / self.rows).max(1),
            )
        }
    }
}

pub fn size() -> Size {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) } == 0;
    if !ok || ws.ws_col == 0 || ws.ws_row == 0 {
        return Size {
            cols: 80,
            rows: 24,
            width_px: 0,
            height_px: 0,
        };
    }
    Size {
        cols: ws.ws_col.into(),
        rows: ws.ws_row.into(),
        width_px: ws.ws_xpixel.into(),
        height_px: ws.ws_ypixel.into(),
    }
}

/// Raw mode and the alternate screen, undone on drop so a crash never strands the shell.
pub struct Terminal {
    saved: libc::termios,
}

impl Terminal {
    pub fn enter() -> io::Result<Self> {
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut saved) } != 0 {
            return Err(io::Error::other("stdin is not a terminal"));
        }
        let mut raw = saved;
        unsafe { libc::cfmakeraw(&mut raw) };
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut out = io::stdout();
        // Alternate screen, hidden cursor, button + drag reporting in SGR form, then ask whether the
        // terminal can report the mouse in pixels (the reply arrives as `Event::PixelMouse`).
        out.write_all(b"\x1b[?1049h\x1b[2J\x1b[?25l\x1b[?1000h\x1b[?1002h\x1b[?1006h\x1b[?1016$p")?;
        out.flush()?;
        Ok(Self { saved })
    }

    pub fn use_pixel_mouse(&self) -> io::Result<()> {
        let mut out = io::stdout();
        out.write_all(b"\x1b[?1016h")?;
        out.flush()
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let mut out = io::stdout();
        let _ = out.write_all(
            b"\x1b_Ga=d,d=A,q=2\x1b\\\x1b[?1016l\x1b[?1006l\x1b[?1002l\x1b[?1000l\x1b[?25h\x1b[?1049l",
        );
        let _ = out.flush();
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.saved) };
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrow {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mouse {
    Press,
    Release,
    Drag,
    WheelUp,
    WheelDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Char(char),
    Arrow(Arrow),
    Enter,
    Escape,
    /// Page Up (`true`) or Page Down.
    Page(bool),
    /// 1-based cell, or pixel when the terminal reports pixels (`PixelMouse(true)`).
    Mouse {
        kind: Mouse,
        x: u32,
        y: u32,
    },
    /// The terminal's answer to whether it can report the mouse in pixels.
    PixelMouse(bool),
}

/// Turns raw input bytes into events. A sequence cut off by a read waits for the next one; a lone
/// escape at the end of a read is the Escape key.
#[derive(Default)]
pub struct Parser {
    pending: Vec<u8>,
}

impl Parser {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Event> {
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        let mut i = 0;
        let buf = std::mem::take(&mut self.pending);
        while i < buf.len() {
            match buf[i] {
                0x1b if i + 1 == buf.len() => {
                    events.push(Event::Escape);
                    i += 1;
                }
                0x1b if buf[i + 1] == b'[' => {
                    let Some(end) = buf[i + 2..]
                        .iter()
                        .position(|b| (0x40..=0x7e).contains(b))
                        .map(|p| i + 2 + p)
                    else {
                        self.pending = buf[i..].to_vec();
                        return events;
                    };
                    if let Some(e) = csi(&buf[i + 2..end], buf[end]) {
                        events.push(e);
                    }
                    i = end + 1;
                }
                0x1b => {
                    events.push(Event::Escape);
                    i += 1;
                }
                b'\r' | b'\n' => {
                    events.push(Event::Enter);
                    i += 1;
                }
                // Ctrl-C arrives as a byte in raw mode.
                0x03 => {
                    events.push(Event::Char('q'));
                    i += 1;
                }
                b if b < 0x20 => i += 1,
                b if b < 0x80 => {
                    events.push(Event::Char(b as char));
                    i += 1;
                }
                _ => {
                    let len = match buf[i] {
                        0xc0..=0xdf => 2,
                        0xe0..=0xef => 3,
                        _ => 4,
                    };
                    if i + len > buf.len() {
                        self.pending = buf[i..].to_vec();
                        return events;
                    }
                    if let Some(c) = std::str::from_utf8(&buf[i..i + len])
                        .ok()
                        .and_then(|s| s.chars().next())
                    {
                        events.push(Event::Char(c));
                    }
                    i += len;
                }
            }
        }
        events
    }
}

fn csi(params: &[u8], last: u8) -> Option<Event> {
    match last {
        b'A' => Some(Event::Arrow(Arrow::Up)),
        b'B' => Some(Event::Arrow(Arrow::Down)),
        b'C' => Some(Event::Arrow(Arrow::Right)),
        b'D' => Some(Event::Arrow(Arrow::Left)),
        b'~' if params == b"5" => Some(Event::Page(true)),
        b'~' if params == b"6" => Some(Event::Page(false)),
        b'M' | b'm' if params.first() == Some(&b'<') => {
            let text = std::str::from_utf8(&params[1..]).ok()?;
            let mut parts = text.split(';').map(|p| p.parse::<u32>().ok());
            let (button, x, y) = (parts.next()??, parts.next()??, parts.next()??);
            let kind = match (button & 0b1100_0011, last) {
                (64, _) => Mouse::WheelUp,
                (65, _) => Mouse::WheelDown,
                (0, b'M') if button & 32 != 0 => Mouse::Drag,
                (0, b'M') => Mouse::Press,
                (0, _) => Mouse::Release,
                _ => return None,
            };
            Some(Event::Mouse { kind, x, y })
        }
        // `CSI ? 1016 ; Ps $ y`: Ps 1 or 2 means the mode exists (set or reset).
        b'y' if params.ends_with(b"$") => {
            let text = std::str::from_utf8(params).ok()?.strip_prefix("?1016;")?;
            let status = text.trim_end_matches('$').parse::<u32>().ok()?;
            Some(Event::PixelMouse(matches!(status, 1 | 2)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: &[u8]) -> Vec<Event> {
        Parser::default().feed(bytes)
    }

    #[test]
    fn keys_arrows_and_escape() {
        assert_eq!(
            parse(b"h\x1b[A\r"),
            [Event::Char('h'), Event::Arrow(Arrow::Up), Event::Enter]
        );
        assert_eq!(parse(b"\x1b"), [Event::Escape]);
        assert_eq!(
            parse(b"\x1b[5~\x1b[6~"),
            [Event::Page(true), Event::Page(false)]
        );
        assert_eq!(parse(b"\x03"), [Event::Char('q')]);
    }

    #[test]
    fn sgr_mouse_press_release_drag_and_wheel() {
        assert_eq!(
            parse(b"\x1b[<0;12;7M\x1b[<32;13;7M\x1b[<0;14;7m\x1b[<64;1;1M\x1b[<65;1;1M"),
            [
                Event::Mouse {
                    kind: Mouse::Press,
                    x: 12,
                    y: 7
                },
                Event::Mouse {
                    kind: Mouse::Drag,
                    x: 13,
                    y: 7
                },
                Event::Mouse {
                    kind: Mouse::Release,
                    x: 14,
                    y: 7
                },
                Event::Mouse {
                    kind: Mouse::WheelUp,
                    x: 1,
                    y: 1
                },
                Event::Mouse {
                    kind: Mouse::WheelDown,
                    x: 1,
                    y: 1
                },
            ]
        );
        // Right-button and bare-motion reports are not touches.
        assert!(parse(b"\x1b[<2;5;5M\x1b[<35;5;5M").is_empty());
    }

    #[test]
    fn a_sequence_split_across_reads_is_joined() {
        let mut parser = Parser::default();
        assert!(parser.feed(b"\x1b[<0;1").is_empty());
        assert_eq!(
            parser.feed(b"2;7M"),
            [Event::Mouse {
                kind: Mouse::Press,
                x: 12,
                y: 7
            }]
        );
    }

    #[test]
    fn the_pixel_mouse_reply() {
        assert_eq!(parse(b"\x1b[?1016;2$y"), [Event::PixelMouse(true)]);
        assert_eq!(parse(b"\x1b[?1016;0$y"), [Event::PixelMouse(false)]);
    }

    #[test]
    fn multibyte_characters_survive() {
        assert_eq!(parse("é".as_bytes()), [Event::Char('é')]);
    }
}

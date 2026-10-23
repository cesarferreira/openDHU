//! The kitty graphics protocol: one RGB frame as a single image that each new frame replaces.

use std::io::{self, Write};

/// The protocol caps each escape's payload at 4096 base64 bytes.
const CHUNK: usize = 4096;
/// The image id every frame reuses, so the terminal keeps one image rather than a pile of them.
const IMAGE_ID: u32 = 1;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let n = group
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= group.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]);
            } else {
                out.push(b'=');
            }
        }
    }
    out
}

/// Where a frame sits: its top-left cell and the box of cells the terminal scales it into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub col: u32,
    pub row: u32,
    pub cols: u32,
    pub rows: u32,
}

/// Draw `rgb` (`width` x `height`, 3 bytes a pixel), scaled to the placement's cell box. Cursor movement is off, so
/// the picture never scrolls the screen.
pub fn draw(
    out: &mut impl Write,
    rgb: &[u8],
    (width, height): (u32, u32),
    at: Placement,
) -> io::Result<()> {
    write!(out, "\x1b[?2026h\x1b[{};{}H", at.row + 1, at.col + 1)?;
    let data = base64(rgb);
    let mut chunks = data.chunks(CHUNK).peekable();
    let mut first = true;
    while let Some(chunk) = chunks.next() {
        let more = u8::from(chunks.peek().is_some());
        if first {
            write!(
                out,
                "\x1b_Ga=T,f=24,s={width},v={height},i={IMAGE_ID},p=1,c={},r={},C=1,q=2,m={more};",
                at.cols, at.rows
            )?;
            first = false;
        } else {
            write!(out, "\x1b_Gm={more};")?;
        }
        out.write_all(chunk)?;
        out.write_all(b"\x1b\\")?;
    }
    out.write_all(b"\x1b[?2026l")?;
    out.flush()
}

/// Remove every image this program drew.
pub fn clear(out: &mut impl Write) -> io::Result<()> {
    out.write_all(b"\x1b_Ga=d,d=A,q=2\x1b\\")?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_vectors() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(plain.as_bytes()), coded.as_bytes());
        }
    }

    #[test]
    fn a_large_frame_is_split_into_chunks_with_a_continuation_flag() {
        let rgb = vec![7u8; 10_000];
        let mut out = Vec::new();
        let at = Placement {
            col: 2,
            row: 3,
            cols: 7,
            rows: 4,
        };
        draw(&mut out, &rgb, (40, 40), at).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("\x1b[4;3H"));
        assert!(text.contains("a=T,f=24,s=40,v=40,i=1,p=1,c=7,r=4,C=1,q=2,m=1;"));
        assert!(text.contains("\x1b_Gm=1;"));
        assert_eq!(text.matches("\x1b_Gm=0;").count(), 1);
        assert!(text.ends_with("\x1b[?2026l"));
        let payload: usize = text
            .split("\x1b_G")
            .skip(1)
            .map(|c| {
                c.split_once(';')
                    .unwrap()
                    .1
                    .split_once("\x1b\\")
                    .unwrap()
                    .0
                    .len()
            })
            .sum();
        assert_eq!(payload, rgb.len().div_ceil(3) * 4);
    }

    #[test]
    fn a_small_frame_is_one_final_chunk() {
        let mut out = Vec::new();
        draw(
            &mut out,
            &[1, 2, 3],
            (1, 1),
            Placement {
                col: 0,
                row: 0,
                cols: 1,
                rows: 1,
            },
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("m=0;AQID\x1b\\"));
        assert!(!text.contains("m=1"));
    }
}

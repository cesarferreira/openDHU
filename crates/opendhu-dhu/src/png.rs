//! Just enough PNG to report a screenshot's size and to reject files that are not PNGs.

const MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// Width and height from the IHDR chunk at the start of a PNG, or `None` if `head` is not one.
pub fn dimensions(head: &[u8]) -> Option<(u32, u32)> {
    if head.len() < 24 || head[..8] != MAGIC || &head[12..16] != b"IHDR" {
        return None;
    }
    let be = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    let (w, h) = (be(&head[16..20]), be(&head[20..24]));
    (w > 0 && h > 0).then_some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(w: u32, h: u32) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend([0, 0, 0, 13]);
        v.extend(b"IHDR");
        v.extend(w.to_be_bytes());
        v.extend(h.to_be_bytes());
        v
    }

    #[test]
    fn reads_dimensions() {
        assert_eq!(dimensions(&header(1280, 720)), Some((1280, 720)));
    }

    #[test]
    fn rejects_non_png_truncated_and_zero_sized() {
        assert_eq!(dimensions(b"not a png at all, just text...."), None);
        assert_eq!(dimensions(&header(1280, 720)[..20]), None);
        assert_eq!(dimensions(&header(0, 720)), None);
        let mut bad_chunk = header(1, 1);
        bad_chunk[12..16].copy_from_slice(b"IDAT");
        assert_eq!(dimensions(&bad_chunk), None);
    }
}

//! Where the picture goes in the terminal, and how a pixel in the terminal maps back to the phone.

use crate::kitty::Placement;
use crate::term::Size;

/// Rows kept for the header above the picture and the key hints below it.
const HEADER_ROWS: u32 = 1;
const FOOTER_ROWS: u32 = 1;
/// Widest frame sent to the terminal, in pixels.
const MAX_DECODE_WIDTH: u32 = 960;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// The picture's size in terminal pixels: the largest the screen's aspect ratio allows.
    pub image: (u32, u32),
    /// The picture's top-left corner in terminal pixels.
    pub origin: (u32, u32),
    pub cell: (u32, u32),
    pub screen: (u32, u32),
}

impl Layout {
    pub fn new(term: Size, screen: (u32, u32), log_rows: u32) -> Self {
        let (cw, ch) = term.cell();
        let area_w = term.cols * cw;
        let area_h = term
            .rows
            .saturating_sub(HEADER_ROWS + FOOTER_ROWS + log_rows)
            * ch;
        let (sw, sh) = (screen.0.max(1), screen.1.max(1));
        // Compare area_w/sw with area_h/sh without division, so no precision is lost.
        let (w, h) = if u64::from(area_w) * u64::from(sh) <= u64::from(area_h) * u64::from(sw) {
            (
                area_w,
                (u64::from(area_w) * u64::from(sh) / u64::from(sw)) as u32,
            )
        } else {
            (
                (u64::from(area_h) * u64::from(sw) / u64::from(sh)) as u32,
                area_h,
            )
        };
        // Whole cells, so kitty scales the picture into an exact cell box and the mouse maps cleanly.
        let (cols, rows) = ((w / cw).max(1), (h / ch).max(1));
        let (w, h) = (cols * cw, rows * ch);
        Self {
            image: (w, h),
            origin: (
                area_w.saturating_sub(w) / 2,
                HEADER_ROWS * ch + area_h.saturating_sub(h) / 2,
            ),
            cell: (cw, ch),
            screen: (sw, sh),
        }
    }

    pub fn placement(&self) -> Placement {
        Placement {
            col: self.origin.0 / self.cell.0,
            row: self.origin.1 / self.cell.1,
            cols: self.image.0 / self.cell.0,
            rows: self.image.1 / self.cell.1,
        }
    }

    /// The size to decode at: the picture's shape, capped so a huge terminal doesn't mean huge
    /// frames. The terminal scales it up to fill the cell box.
    pub fn decode_size(&self) -> (u32, u32) {
        let w = self.image.0.min(MAX_DECODE_WIDTH);
        (
            w,
            (u64::from(self.image.1) * u64::from(w) / u64::from(self.image.0)) as u32 & !1,
        )
    }

    /// The phone screen coordinate under a terminal pixel. `None` outside the picture, unless
    /// `clamp`, which pins it to the nearest edge (the end of a drag that left the picture).
    pub fn screen_at(&self, (px, py): (u32, u32), clamp: bool) -> Option<(u32, u32)> {
        let axis = |p: u32, origin: u32, len: u32, screen: u32| {
            let rel = i64::from(p) - i64::from(origin);
            if !clamp && !(0..i64::from(len)).contains(&rel) {
                return None;
            }
            let rel = rel.clamp(0, i64::from(len) - 1);
            Some((rel * i64::from(screen) / i64::from(len)) as u32)
        };
        Some((
            axis(px, self.origin.0, self.image.0, self.screen.0)?,
            axis(py, self.origin.1, self.image.1, self.screen.1)?,
        ))
    }

    /// A mouse report's 1-based position as a terminal pixel: exact when the terminal reports
    /// pixels, otherwise the middle of the reported cell.
    pub fn mouse_pixel(&self, (x, y): (u32, u32), pixels: bool) -> (u32, u32) {
        let (x, y) = (x.saturating_sub(1), y.saturating_sub(1));
        if pixels {
            (x, y)
        } else {
            (
                x * self.cell.0 + self.cell.0 / 2,
                y * self.cell.1 + self.cell.1 / 2,
            )
        }
    }
}

/// How many rows of the terminal go to the log pane under the picture: a third, within reason.
pub fn log_rows(term: Size) -> u32 {
    (term.rows.saturating_sub(2) / 3).clamp(3, 14)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(cols: u32, rows: u32) -> Size {
        Size {
            cols,
            rows,
            width_px: cols * 10,
            height_px: rows * 20,
        }
    }

    #[test]
    fn a_wide_terminal_is_limited_by_height_and_centred() {
        // 200x50 cells = 2000x1000 px; the picture area is 2000x960, so 800x480 scales to 1600x960.
        let l = Layout::new(term(200, 50), (800, 480), 0);
        assert_eq!(l.image, (1600, 960));
        assert_eq!(l.decode_size(), (960, 576));
        assert_eq!(l.origin, (200, 20));
        assert_eq!(
            l.placement(),
            Placement {
                col: 20,
                row: 1,
                cols: 160,
                rows: 48
            }
        );
    }

    #[test]
    fn a_log_pane_takes_rows_from_the_picture() {
        let with = Layout::new(term(200, 50), (800, 480), 10);
        // 38 rows (760 px) remain: height limits, 760 px tall is 1266 wide, cut to whole cells.
        assert_eq!(with.image, (1260, 760));
        assert_eq!(with.placement().row, 1);
    }

    #[test]
    fn a_narrow_terminal_is_limited_by_width() {
        let l = Layout::new(term(40, 40), (800, 480), 0);
        assert_eq!(l.image, (400, 240));
        assert_eq!(l.origin.0, 0);
        assert_eq!(l.origin.1, 20 + (38 * 20 - 240) / 2);
    }

    #[test]
    fn the_placement_is_in_whole_cells() {
        let l = Layout {
            image: (100, 40),
            origin: (20, 40),
            cell: (10, 20),
            screen: (800, 480),
        };
        assert_eq!(
            l.placement(),
            Placement {
                col: 2,
                row: 2,
                cols: 10,
                rows: 2
            }
        );
    }

    #[test]
    fn terminal_pixels_map_to_phone_pixels() {
        let l = Layout::new(term(200, 50), (800, 480), 0);
        assert_eq!(l.screen_at(l.origin, false), Some((0, 0)));
        assert_eq!(
            l.screen_at((l.origin.0 + 800, l.origin.1 + 480), false),
            Some((400, 240))
        );
        assert_eq!(l.screen_at((0, 0), false), None);
        assert_eq!(l.screen_at((0, 0), true), Some((0, 0)));
        assert_eq!(l.screen_at((1999, 999), true), Some((799, 479)));
    }

    #[test]
    fn cell_mouse_reports_land_in_the_middle_of_the_cell() {
        let l = Layout::new(term(200, 50), (800, 480), 0);
        assert_eq!(l.mouse_pixel((1, 1), false), (5, 10));
        assert_eq!(l.mouse_pixel((1, 1), true), (0, 0));
    }

    #[test]
    fn a_terminal_without_pixel_size_assumes_ten_by_twenty() {
        let t = Size {
            cols: 100,
            rows: 30,
            width_px: 0,
            height_px: 0,
        };
        assert_eq!(t.cell(), (10, 20));
    }
}

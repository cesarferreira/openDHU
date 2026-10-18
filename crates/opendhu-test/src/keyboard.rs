//! Touch typing on the observed lowercase native 800x480 head-unit keyboard.
use crate::runner::RunConfig;
use opendhu_core::{Error, HeadUnitBackend, Result};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

static KEYBOARD_ID: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn validate_text(text: &str) -> Result<()> {
    if text.is_empty()
        || text.len() > 128
        || !text
            .bytes()
            .all(|key| key.is_ascii_lowercase() || key.is_ascii_digit() || key == b' ')
    {
        return Err(Error::InvalidArgs(
            "keyboard.type supports 1–128 lowercase ASCII letters, digits and spaces".into(),
        ));
    }
    Ok(())
}

fn position(key: u8) -> (u32, u32) {
    for (row, x, y) in [
        (b"qwertyuiop".as_slice(), 164, 283),
        (b"asdfghjkl".as_slice(), 188, 320),
        (b"zxcvbnm".as_slice(), 211, 359),
        (b"1234567890".as_slice(), 164, 245),
    ] {
        if let Some(index) = row.iter().position(|value| *value == key) {
            return (x + 47 * index as u32, y);
        }
    }
    (392, 397) // Space: all other unsupported characters were rejected before execution.
}

fn stopped(cfg: &RunConfig, deadline: Duration) -> Result<()> {
    if cfg.clock.cancelled() {
        return Err(Error::Other(
            "interrupted while typing on projected keyboard".into(),
        ));
    }
    if cfg.clock.now() >= deadline {
        return Err(Error::Timeout(
            "projected keyboard typing timed out; no further touches sent".into(),
        ));
    }
    Ok(())
}

pub(crate) fn type_text(
    text: &str,
    timeout: Duration,
    backend: &mut dyn HeadUnitBackend,
    cfg: &RunConfig,
) -> Result<String> {
    validate_text(text)?;
    let deadline = cfg.clock.now() + timeout;
    stopped(cfg, deadline)?;
    let status = backend.status()?;
    if status.backend != "native"
        || status.screen_width != Some(800)
        || status.screen_height != Some(480)
    {
        return Err(Error::Unsupported(
            "keyboard.type requires the native 800x480 projection profile".into(),
        ));
    }
    let id = KEYBOARD_ID.fetch_add(1, Ordering::Relaxed);
    let dir = cfg
        .artifacts_dir
        .join("keyboard")
        .join(format!("input-{id:04}"));
    std::fs::create_dir_all(&dir).map_err(|error| Error::Other(error.to_string()))?;
    for (index, key) in text.bytes().enumerate() {
        let image = dir.join(format!("char-{index:03}.png"));
        loop {
            stopped(cfg, deadline)?;
            let frame = backend.screenshot(&image)?;
            if frame.width != 800 || frame.height != 480 {
                return Err(Error::Unsupported(
                    "projected keyboard frame changed from the supported 800x480 profile".into(),
                ));
            }
            let observed = crate::screen::recognize_keyboard(&image, || {
                cfg.clock.cancelled() || cfg.clock.now() >= deadline
            });
            // Translate a cancelled OCR process into the runner's interruption code, rather than
            // reporting its external-process timeout as the reason typing stopped.
            stopped(cfg, deadline)?;
            let words = observed?;
            let markers = [
                ("q", 164, 283),
                ("e", 258, 283),
                ("t", 352, 283),
                ("u", 447, 283),
                ("a", 188, 320),
                ("1", 164, 245),
            ]
            .iter()
            .filter(|(glyph, x, y)| crate::screen::exact_glyph_near(&words, glyph, *x, *y, 18))
            .count();
            // These two anchors distinguish QWERTY from QWERTZ, whose other key rows and
            // coordinates otherwise satisfy the general keyboard markers.
            let qwerty = crate::screen::exact_glyph_near(&words, "y", 399, 283, 18)
                && crate::screen::exact_glyph_near(&words, "z", 211, 359, 18);
            if markers >= 3 && qwerty {
                let (x, y) = position(key);
                let input = backend.tap(x, y)?;
                if !input.status.is_success() {
                    return Err(Error::Other(format!(
                        "projected keyboard touch was not accepted: {:?}",
                        input.status
                    )));
                }
                break;
            }
            let remaining = deadline.saturating_sub(cfg.clock.now());
            cfg.clock.sleep(Duration::from_millis(250).min(remaining));
        }
        if index + 1 < text.len() {
            let remaining = deadline.saturating_sub(cfg.clock.now());
            cfg.clock.sleep(Duration::from_millis(100).min(remaining));
        }
    }
    stopped(cfg, deadline)?;
    Ok(format!(
        "sent {} projected keyboard characters after checking lowercase QWERTY before every touch; assert on the resulting UI; evidence {}",
        text.len(),
        dir.display()
    ))
}

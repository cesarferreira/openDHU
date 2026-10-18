//! OCR of the actual projected image. Text observations never come from app internals.
use opendhu_core::{Error, Result};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Word {
    line: String,
    text: String,
    left: u32,
    top: u32,
    width: u32,
    height: u32,
}

pub fn parse_tsv(tsv: &str) -> Result<Vec<Word>> {
    if !tsv.starts_with("level\tpage_num\t") {
        return Err(Error::Other("OCR returned an invalid TSV header".into()));
    }
    let mut words = Vec::new();
    for line in tsv.lines().skip(1) {
        let fields: Vec<&str> = line.splitn(12, '\t').collect();
        if fields.len() != 12 {
            return Err(Error::Other("OCR returned an invalid TSV row".into()));
        }
        if fields[0] != "5" || fields[11].trim().is_empty() {
            continue;
        }
        let number = |i: usize| {
            fields[i]
                .parse::<u32>()
                .map_err(|_| Error::Other("OCR returned invalid bounds".into()))
        };
        words.push(Word {
            line: fields[1..5].join(":"),
            text: fields[11].into(),
            left: number(6)?,
            top: number(7)?,
            width: number(8)?,
            height: number(9)?,
        });
    }
    Ok(words)
}

fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Locate a phrase's words, rather than the whole line (launcher icons can share a line).
pub fn locate(words: &[Word], text: &str) -> Option<(u32, u32)> {
    let needle = normalized(text);
    if needle.is_empty() {
        return None;
    }
    for start in 0..words.len() {
        let mut phrase = String::new();
        let (mut right, mut bottom) = (0, 0);
        for word in &words[start..] {
            if word.line != words[start].line {
                break;
            }
            if !phrase.is_empty() {
                phrase.push(' ');
            }
            phrase.push_str(&word.text);
            right = right.max(word.left.saturating_add(word.width));
            bottom = bottom.max(word.top.saturating_add(word.height));
            let phrase = normalized(&phrase);
            if phrase.match_indices(&needle).any(|(position, _)| {
                position < normalized(&words[start].text).len()
                    && phrase[..position]
                        .chars()
                        .next_back()
                        .is_none_or(|c| !c.is_alphanumeric())
                    && phrase[position + needle.len()..]
                        .chars()
                        .next()
                        .is_none_or(|c| !c.is_alphanumeric())
            }) {
                return Some((
                    (words[start].left + right) / 2,
                    (words[start].top + bottom) / 2,
                ));
            }
        }
    }
    None
}

pub fn locate_in_region(words: &[Word], text: &str, top: u32, bottom: u32) -> Option<(u32, u32)> {
    let visible: Vec<Word> = words
        .iter()
        .filter(|word| word.top >= top && word.top < bottom)
        .cloned()
        .collect();
    locate(&visible, text)
}

/// Match a complete launcher label, keeping app body text and similarly named apps distinct.
/// Android Auto truncates long labels with either three dots or a Unicode ellipsis.
pub fn locate_launcher_label(
    words: &[Word],
    label: &str,
    top: u32,
    bottom: u32,
) -> Option<(u32, u32)> {
    let expected = normalized(label);
    let mut start = 0;
    while start < words.len() {
        let mut end = start + 1;
        while end < words.len() && words[end].line == words[start].line {
            end += 1;
        }
        let line = &words[start..end];
        if line.iter().all(|word| word.top >= top && word.top < bottom) {
            let shown = normalized(&text(line));
            let truncated = shown
                .strip_suffix("...")
                .or_else(|| shown.strip_suffix('…'));
            let matches = shown == expected
                || truncated.is_some_and(|prefix| {
                    let prefix = prefix.trim_end();
                    prefix.chars().count() >= 3 && expected.starts_with(prefix)
                });
            if matches {
                let left = line.iter().map(|word| word.left).min()?;
                let right = line
                    .iter()
                    .map(|word| word.left.saturating_add(word.width))
                    .max()?;
                let top = line.iter().map(|word| word.top).min()?;
                let bottom = line
                    .iter()
                    .map(|word| word.top.saturating_add(word.height))
                    .max()?;
                return Some((left + (right - left) / 2, top + (bottom - top) / 2));
            }
        }
        start = end;
    }
    None
}

/// Case-sensitive single-key observations anchored to a known projected keyboard layout.
pub fn exact_glyph_near(words: &[Word], glyph: &str, x: u32, y: u32, tolerance: u32) -> bool {
    words.iter().any(|word| {
        word.text == glyph
            && word.width <= 32
            && word.height <= 36
            && word.left.saturating_add(word.width / 2).abs_diff(x) <= tolerance
            && word.top.saturating_add(word.height / 2).abs_diff(y) <= tolerance
    })
}

pub fn text(words: &[Word]) -> String {
    words
        .iter()
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Retain TSV next to the screenshot for failure diagnosis. No shell interpolation is used.
pub fn recognize(image: &Path, cancelled: impl Fn() -> bool) -> Result<Vec<Word>> {
    // Leptonica on macOS can reject symlinked /tmp paths; resolve the existing image first.
    let image = std::fs::canonicalize(image)
        .map_err(|e| Error::Other(format!("cannot resolve screenshot: {e}")))?;
    let output = image.with_extension("ocr");
    let error_log = image.with_extension("ocr.log");
    let enlarged = image.with_extension("ocr.png");
    let mut resize = Command::new("ffmpeg");
    resize
        .args(["-v", "error", "-y", "-i"])
        .arg(&image)
        .args(["-vf", "scale=iw*2:ih*2"])
        .arg(&enlarged);
    finish_command(resize, &error_log, &cancelled)?;
    let program = std::env::var_os("OPENDHU_TESSERACT").unwrap_or_else(|| "tesseract".into());
    let mut command = Command::new(&program);
    command
        .arg(&enlarged)
        .arg(&output)
        .args(["--psm", "11", "tsv"]);
    finish_command(command, &error_log, &cancelled)?;
    let tsv = std::fs::read_to_string(output.with_extension("ocr.tsv"))
        .map_err(|e| Error::Other(format!("cannot read OCR output: {e}")))?;
    let mut words = parse_tsv(&tsv)?;
    for word in &mut words {
        word.line.insert_str(0, "scaled:");
        word.left /= 2;
        word.top /= 2;
        word.width = word.width.div_ceil(2);
        word.height = word.height.div_ceil(2);
    }
    // Original-size text can distinguish glyphs that interpolation merges (notably UI vs Ul).
    let original = image.with_extension("original");
    let mut command = Command::new(&program);
    command
        .arg(&image)
        .arg(&original)
        .args(["--psm", "11", "tsv"]);
    finish_command(command, &error_log, &cancelled)?;
    let tsv = std::fs::read_to_string(original.with_extension("original.tsv"))
        .map_err(|e| Error::Other(format!("cannot read original OCR output: {e}")))?;
    words.extend(parse_tsv(&tsv)?);
    // Dimmed but visible template status text can disappear in normal OCR. Binarize the
    // projected pixels and invert to dark text on white, retaining the unmodified passes
    // above for fine glyphs and keyboard layout checks.
    // Automatic segmentation at a higher threshold also recovers enclosed control labels
    // that sparse-text mode omits, without assuming where those controls appear.
    for (name, threshold, segmentation) in [("contrast", 60, "11"), ("segmented", 110, "3")] {
        let contrast = image.with_extension(format!("{name}.png"));
        let mut enhance = Command::new("ffmpeg");
        enhance
            .args(["-v", "error", "-y", "-i"])
            .arg(&image)
            .args([
                "-vf",
                &format!(
                    "format=gray,lut=y='if(gt(val,{threshold}),255,0)',negate,scale=iw*2:ih*2"
                ),
            ])
            .arg(&contrast);
        finish_command(enhance, &error_log, &cancelled)?;
        let contrast_output = image.with_extension(name);
        let mut command = Command::new(&program);
        command
            .arg(&contrast)
            .arg(&contrast_output)
            .args(["--psm", segmentation, "tsv"]);
        finish_command(command, &error_log, &cancelled)?;
        let tsv = std::fs::read_to_string(image.with_extension(format!("{name}.tsv")))
            .map_err(|error| Error::Other(format!("cannot read {name} OCR output: {error}")))?;
        let mut contrast_words = parse_tsv(&tsv)?;
        for word in &mut contrast_words {
            word.line.insert_str(0, &format!("{name}:"));
            word.left /= 2;
            word.top /= 2;
            word.width = word.width.div_ceil(2);
            word.height = word.height.div_ceil(2);
        }
        words.extend(contrast_words);
    }
    Ok(words)
}

/// Observe the guarded 800x480 keyboard without search results changing glyph segmentation.
/// Preserve the full-frame passes and save the focused pass alongside them for diagnosis.
pub fn recognize_keyboard(image: &Path, cancelled: impl Fn() -> bool) -> Result<Vec<Word>> {
    let mut words = recognize(image, &cancelled)?;
    let image = std::fs::canonicalize(image)
        .map_err(|e| Error::Other(format!("cannot resolve screenshot: {e}")))?;
    let crop = image.with_extension("keyboard.png");
    let output = image.with_extension("keyboard");
    let error_log = image.with_extension("ocr.log");
    let mut resize = Command::new("ffmpeg");
    resize
        .args(["-v", "error", "-y", "-i"])
        .arg(&image)
        .args(["-vf", "crop=520:190:140:225,scale=iw*2:ih*2"])
        .arg(&crop);
    finish_command(resize, &error_log, &cancelled)?;
    let program = std::env::var_os("OPENDHU_TESSERACT").unwrap_or_else(|| "tesseract".into());
    let mut command = Command::new(program);
    // Treat the fixed keyboard rows as a block: sparse-text mode can omit isolated y/z keys.
    command.arg(&crop).arg(&output).args(["--psm", "6", "tsv"]);
    finish_command(command, &error_log, &cancelled)?;
    let tsv = std::fs::read_to_string(output.with_extension("keyboard.tsv"))
        .map_err(|e| Error::Other(format!("cannot read keyboard OCR output: {e}")))?;
    let mut keyboard_words = parse_tsv(&tsv)?;
    for word in &mut keyboard_words {
        word.line.insert_str(0, "keyboard:");
        word.left = word.left / 2 + 140;
        word.top = word.top / 2 + 225;
        word.width = word.width.div_ceil(2);
        word.height = word.height.div_ceil(2);
    }
    words.extend(keyboard_words);
    Ok(words)
}

fn finish_command(
    mut command: Command,
    error_log: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<()> {
    let errors = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(error_log)
        .map_err(|e| Error::Other(e.to_string()))?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(errors)
        .spawn()
        .map_err(|e| {
            Error::BackendUnavailable(format!(
                "cannot start OCR command: {e}; install ffmpeg and tesseract"
            ))
        })?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline && !cancelled() => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Timeout("OCR timed out or was interrupted".into()));
            }
        }
    };
    if status.success() {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "OCR command failed: {status}; see {}",
            error_log.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_does_not_match_driver_before_the_drive_button() {
        let words = parse_tsv("level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n5\t1\t1\t1\t1\t1\t396\t205\t92\t25\t95\tDriver\n5\t1\t1\t1\t2\t1\t260\t444\t87\t30\t95\tDrive\n").unwrap();
        assert_eq!(locate(&words[..1], "Drive"), None);
        assert_eq!(locate(&words, "Drive"), Some((303, 459)));
    }

    #[test]
    fn phrase_matches_respect_word_boundaries_and_allow_punctuation() {
        let words = parse_tsv("level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n5\t1\t1\t1\t1\t1\t10\t20\t40\t20\t95\t11\n5\t1\t1\t1\t1\t2\t60\t20\t80\t20\t95\tavailable\n5\t1\t1\t1\t2\t1\t10\t60\t100\t20\t95\tAV|50mph\n").unwrap();
        assert_eq!(locate(&words, "1 available"), None);
        assert_eq!(locate(&words, "11 available"), Some((75, 30)));
        assert_eq!(locate(&words, "AV"), Some((60, 70)));
        assert_eq!(locate(&words, "mph"), None);
    }

    #[test]
    fn locates_only_the_matching_words_in_a_launcher_row() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n5\t1\t1\t1\t1\t1\t10\t40\t40\t20\t95\tMaps\n5\t1\t1\t1\t1\t2\t100\t40\t60\t20\t95\tRobotUI\n5\t1\t1\t1\t1\t3\t165\t40\t80\t20\t95\tStorybook\n";
        let words = parse_tsv(tsv).unwrap();
        assert_eq!(locate(&words, "robotui storybook"), Some((172, 50)));
        assert_eq!(locate(&words, "absent"), None);
    }

    #[test]
    fn launcher_labels_match_complete_lines_or_explicit_ellipsis_only() {
        let words = |label: &str| {
            parse_tsv(&format!("level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n5\t1\t1\t1\t1\t1\t200\t138\t200\t20\t99\t{label}\n")).unwrap()
        };
        assert!(locate_launcher_label(&words("Robot UI"), "Robot UI", 70, 410).is_some());
        assert!(
            locate_launcher_label(
                &words("Robot UI has encountered an unexpected error"),
                "Robot UI",
                70,
                410
            )
            .is_none()
        );
        assert!(locate_launcher_label(&words("Robot UI Storybook"), "Robot UI", 70, 410).is_none());
        for label in ["RobotUI Stor...", "RobotUI Stor…"] {
            assert!(locate_launcher_label(&words(label), "RobotUI Storybook", 70, 410).is_some());
        }
        assert!(
            locate_launcher_label(&words("RobotUI Stor"), "RobotUI Storybook", 70, 410).is_none()
        );
        assert!(locate_launcher_label(&words("RobotUI Stor..."), "Robot UI", 70, 410).is_none());
    }

    #[test]
    fn keyboard_glyphs_require_exact_case_and_spatially_matching_bounds() {
        let words = parse_tsv("level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n5\t1\t1\t1\t1\t1\t158\t275\t12\t16\t99\tq\n").unwrap();
        assert!(exact_glyph_near(&words, "q", 164, 283, 18));
        assert!(!exact_glyph_near(&words, "Q", 164, 283, 18));
        assert!(!exact_glyph_near(&words, "q", 211, 283, 18));
        assert!(!exact_glyph_near(&words, "q", 164, 245, 18));
    }

    #[test]
    fn malformed_ocr_output_is_an_error_instead_of_an_empty_success() {
        assert!(parse_tsv("not tsv").is_err());
        assert!(parse_tsv("level\tpage_num\n5\tbroken").is_err());
    }
}

//! OCR of the actual projected image. Text observations never come from app internals.
use autolab_core::{Error, Result};
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
            if normalized(&phrase)
                .find(&needle)
                .is_some_and(|position| position < normalized(&words[start].text).len())
            {
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
    let program = std::env::var_os("AUTOLAB_TESSERACT").unwrap_or_else(|| "tesseract".into());
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
    let mut command = Command::new(program);
    command
        .arg(&image)
        .arg(&original)
        .args(["--psm", "11", "tsv"]);
    finish_command(command, &error_log, &cancelled)?;
    let tsv = std::fs::read_to_string(original.with_extension("original.tsv"))
        .map_err(|e| Error::Other(format!("cannot read original OCR output: {e}")))?;
    words.extend(parse_tsv(&tsv)?);
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
    fn locates_only_the_matching_words_in_a_launcher_row() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n5\t1\t1\t1\t1\t1\t10\t40\t40\t20\t95\tMaps\n5\t1\t1\t1\t1\t2\t100\t40\t60\t20\t95\tRobotUI\n5\t1\t1\t1\t1\t3\t165\t40\t80\t20\t95\tStorybook\n";
        let words = parse_tsv(tsv).unwrap();
        assert_eq!(locate(&words, "robotui storybook"), Some((172, 50)));
        assert_eq!(locate(&words, "absent"), None);
    }

    #[test]
    fn malformed_ocr_output_is_an_error_instead_of_an_empty_success() {
        assert!(parse_tsv("not tsv").is_err());
        assert!(parse_tsv("level\tpage_num\n5\tbroken").is_err());
    }
}

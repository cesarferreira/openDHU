//! H.264 to PNG. The phone sends a stream, not pictures, so a screenshot decodes the frames
//! received since the last keyframe with `ffmpeg` and keeps the last picture.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use autolab_core::ScreenshotResult;
use autolab_native::protocol::{ErrorInfo, ErrorKind};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const DECODE_TIMEOUT: Duration = Duration::from_secs(15);

fn err(kind: ErrorKind, message: impl Into<String>) -> ErrorInfo {
    ErrorInfo {
        kind,
        message: message.into(),
    }
}

/// Width and height from a PNG header, or `None` if the bytes are not a PNG.
pub fn png_size(head: &[u8]) -> Option<(u32, u32)> {
    if head.len() < 24 || &head[..8] != b"\x89PNG\r\n\x1a\n" || &head[12..16] != b"IHDR" {
        return None;
    }
    Some((
        u32::from_be_bytes(head[16..20].try_into().ok()?),
        u32::from_be_bytes(head[20..24].try_into().ok()?),
    ))
}

/// Decode `stream` (Annex-B H.264 starting with SPS and PPS, then a keyframe) into `path`.
pub async fn decode_to_png(stream: Vec<u8>, path: &Path) -> Result<ScreenshotResult, ErrorInfo> {
    let ffmpeg = autolab_native::find_ffmpeg().ok_or_else(|| {
        err(
            ErrorKind::Unavailable,
            "ffmpeg not found; it decodes the phone's video into screenshots (macOS: brew install ffmpeg)",
        )
    })?;
    // Written next to the target and renamed, so a half-written picture never has the final name.
    let part = path.with_extension("part.png");
    let _ = std::fs::remove_file(&part);
    let _ = std::fs::remove_file(path);

    let mut child = Command::new(ffmpeg)
        .args(["-v", "error", "-nostdin", "-f", "h264", "-i", "pipe:0"])
        .args(["-update", "1", "-y"])
        .arg(&part)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| err(ErrorKind::Unavailable, format!("cannot run ffmpeg: {e}")))?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let feed = tokio::spawn(async move {
        let _ = stdin.write_all(&stream).await;
        // Dropping stdin ends the stream.
    });
    let out = tokio::time::timeout(DECODE_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| err(ErrorKind::Timeout, "ffmpeg did not finish decoding in time"))?
        .map_err(|e| err(ErrorKind::Other, format!("ffmpeg failed: {e}")))?;
    let _ = feed.await;
    if !out.status.success() || !part.is_file() {
        let _ = std::fs::remove_file(&part);
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(err(
            ErrorKind::Other,
            format!(
                "ffmpeg could not decode a picture: {}",
                stderr.lines().last().unwrap_or("no output")
            ),
        ));
    }
    std::fs::rename(&part, path).map_err(|e| {
        err(
            ErrorKind::Other,
            format!("cannot save {}: {e}", path.display()),
        )
    })?;
    let head = std::fs::read(path)
        .map(|b| b[..b.len().min(24)].to_vec())
        .map_err(|e| {
            err(
                ErrorKind::Other,
                format!("cannot read {}: {e}", path.display()),
            )
        })?;
    let (width, height) = png_size(&head)
        .ok_or_else(|| err(ErrorKind::Other, format!("{} is not a PNG", path.display())))?;
    Ok(ScreenshotResult {
        path: path.to_path_buf(),
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_size_reads_the_header_and_rejects_other_bytes() {
        let mut png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR".to_vec();
        png.extend_from_slice(&800u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(png_size(&png), Some((800, 480)));
        assert_eq!(png_size(b"not a png at all, just text.."), None);
        assert_eq!(png_size(&png[..10]), None);
    }
}

//! Decode every H.264 frame, retaining the final RGB picture and PNG-encoding it once.
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use autolab_core::ScreenshotResult;
use autolab_native::protocol::{ErrorInfo, ErrorKind};
use image::ImageEncoder;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

const DECODE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_HEADER_BYTES: usize = 1024;
const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 64 * 1024;

fn err(kind: ErrorKind, message: impl Into<String>) -> ErrorInfo {
    ErrorInfo {
        kind,
        message: message.into(),
    }
}

/// Width and height from a PNG header, or `None` if the bytes are not a PNG.
#[cfg(test)]
pub fn png_size(head: &[u8]) -> Option<(u32, u32)> {
    if head.len() < 24 || &head[..8] != b"\x89PNG\r\n\x1a\n" || &head[12..16] != b"IHDR" {
        return None;
    }
    Some((
        u32::from_be_bytes(head[16..20].try_into().ok()?),
        u32::from_be_bytes(head[20..24].try_into().ok()?),
    ))
}

#[derive(Debug)]
struct Frame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

async fn header_line<R: AsyncRead + Unpin>(
    reader: &mut R,
    budget: &mut usize,
    boundary: bool,
) -> Result<Option<String>, ErrorInfo> {
    let mut line = Vec::new();
    loop {
        let mut byte = [0];
        let n = reader.read(&mut byte).await.map_err(|error| {
            err(
                ErrorKind::Other,
                format!("cannot read decoded frame header: {error}"),
            )
        })?;
        if n == 0 {
            return if boundary && line.is_empty() {
                Ok(None)
            } else {
                Err(err(ErrorKind::Other, "truncated decoded frame header"))
            };
        }
        *budget += 1;
        if *budget > MAX_HEADER_BYTES {
            return Err(err(ErrorKind::Other, "decoded frame header exceeds limit"));
        }
        if byte[0] == b'\n' {
            return String::from_utf8(line)
                .map(Some)
                .map_err(|_| err(ErrorKind::Other, "invalid decoded frame header"));
        }
        line.push(byte[0]);
    }
}

/// PAM frames have explicit dimensions; no assumption about the phone's declared resolution is used.
/// Reuse one RGB allocation, and reject a partial tail instead of returning an older complete frame.
async fn last_pam_frame<R: AsyncRead + Unpin>(reader: R) -> Result<Frame, ErrorInfo> {
    let mut reader = BufReader::new(reader);
    let mut last: Option<Frame> = None;
    loop {
        let mut budget = 0;
        let Some(magic) = header_line(&mut reader, &mut budget, true).await? else {
            break;
        };
        if magic != "P7" {
            return Err(err(ErrorKind::Other, "expected an RGB PAM frame"));
        }
        let (mut width, mut height, mut depth, mut maxval, mut rgb) =
            (None, None, None, None, false);
        loop {
            let line = header_line(&mut reader, &mut budget, false)
                .await?
                .expect("non-boundary header line");
            if line == "ENDHDR" {
                break;
            }
            let (field, value) = line
                .split_once(' ')
                .ok_or_else(|| err(ErrorKind::Other, "invalid decoded frame field"))?;
            let number = || {
                value
                    .parse::<u32>()
                    .map_err(|_| err(ErrorKind::Other, "invalid decoded frame dimension"))
            };
            match field {
                "WIDTH" if width.is_none() => width = Some(number()?),
                "HEIGHT" if height.is_none() => height = Some(number()?),
                "DEPTH" if depth.is_none() => depth = Some(number()?),
                "MAXVAL" if maxval.is_none() => maxval = Some(number()?),
                "TUPLTYPE" if !rgb && value == "RGB" => rgb = true,
                _ => return Err(err(ErrorKind::Other, "unsupported decoded frame header")),
            }
        }
        let (width, height) = match (width, height, depth, maxval, rgb) {
            (Some(width), Some(height), Some(3), Some(255), true) if width > 0 && height > 0 => {
                (width, height)
            }
            _ => return Err(err(ErrorKind::Other, "incomplete RGB frame header")),
        };
        let size = (width as usize)
            .checked_mul(height as usize)
            .and_then(|size| size.checked_mul(3))
            .filter(|size| *size <= MAX_FRAME_BYTES)
            .ok_or_else(|| err(ErrorKind::Other, "decoded frame exceeds size limit"))?;
        let mut pixels = last.take().map(|frame| frame.pixels).unwrap_or_default();
        pixels.resize(size, 0);
        reader.read_exact(&mut pixels).await.map_err(|error| {
            err(
                ErrorKind::Other,
                format!("truncated decoded RGB frame: {error}"),
            )
        })?;
        last = Some(Frame {
            width,
            height,
            pixels,
        });
    }
    last.ok_or_else(|| {
        err(
            ErrorKind::Other,
            "ffmpeg did not produce a complete RGB frame",
        )
    })
}

async fn diagnostics<R: AsyncRead + Unpin>(mut reader: R) -> Vec<u8> {
    let mut captured = Vec::new();
    let mut buffer = [0; 8192];
    while let Ok(n) = reader.read(&mut buffer).await {
        if n == 0 {
            break;
        }
        let keep = n.min(MAX_STDERR_BYTES.saturating_sub(captured.len()));
        captured.extend_from_slice(&buffer[..keep]);
    }
    captured
}

/// Decode the entire captured GOP to its last displayed frame, then compress only that frame.
pub async fn decode_to_png(stream: Vec<u8>, path: &Path) -> Result<ScreenshotResult, ErrorInfo> {
    let ffmpeg = autolab_native::find_ffmpeg().ok_or_else(|| err(ErrorKind::Unavailable,
        "ffmpeg not found; it decodes the phone's video into screenshots (macOS: brew install ffmpeg)"))?;
    let part = path.with_extension("part.png");
    let _ = tokio::fs::remove_file(&part).await;
    let _ = tokio::fs::remove_file(path).await;
    let mut child = Command::new(ffmpeg)
        .args([
            "-v",
            "error",
            "-nostdin",
            "-f",
            "h264",
            "-i",
            "pipe:0",
            "-f",
            "image2pipe",
            "-c:v",
            "pam",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            err(
                ErrorKind::Unavailable,
                format!("cannot run ffmpeg: {error}"),
            )
        })?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let mut feed = tokio::spawn(async move {
        let _ = stdin.write_all(&stream).await;
    });
    let stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = tokio::spawn(diagnostics(child.stderr.take().expect("stderr is piped")));
    let result = tokio::time::timeout(DECODE_TIMEOUT, async {
        let frame = last_pam_frame(stdout).await;
        let status = child
            .wait()
            .await
            .map_err(|error| err(ErrorKind::Other, format!("ffmpeg failed: {error}")))?;
        let errors = (&mut stderr).await.map_err(|error| {
            err(
                ErrorKind::Other,
                format!("cannot read ffmpeg diagnostics: {error}"),
            )
        })?;
        let _ = (&mut feed).await;
        if !status.success() {
            let errors = String::from_utf8_lossy(&errors);
            return Err(err(
                ErrorKind::Other,
                format!(
                    "ffmpeg could not decode a picture: {}",
                    errors.lines().last().unwrap_or("no output")
                ),
            ));
        }
        let frame = frame?;
        let (width, height) = (frame.width, frame.height);
        // This blocking worker only returns bytes: if the overall deadline expires, it cannot
        // later publish a stale PNG or leave a partially written file behind.
        let png = tokio::task::spawn_blocking(move || {
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(&frame.pixels, width, height, image::ExtendedColorType::Rgb8)
                .map_err(|error| {
                    err(
                        ErrorKind::Other,
                        format!("cannot encode final frame: {error}"),
                    )
                })?;
            Ok::<_, ErrorInfo>(png)
        })
        .await
        .map_err(|error| err(ErrorKind::Other, format!("PNG encoder failed: {error}")))??;
        // Create the name synchronously before handing the open file to Tokio. A timed-out
        // asynchronous write can then finish only on this descriptor; cleanup can unlink it
        // without a queued filesystem operation recreating the partial filename later.
        let file = std::fs::File::create(&part).map_err(|error| {
            err(
                ErrorKind::Other,
                format!("cannot create {}: {error}", part.display()),
            )
        })?;
        let mut file = tokio::fs::File::from_std(file);
        file.write_all(&png).await.map_err(|error| {
            err(
                ErrorKind::Other,
                format!("cannot write {}: {error}", part.display()),
            )
        })?;
        file.flush().await.map_err(|error| {
            err(
                ErrorKind::Other,
                format!("cannot flush {}: {error}", part.display()),
            )
        })?;
        drop(file);
        tokio::fs::rename(&part, path).await.map_err(|error| {
            err(
                ErrorKind::Other,
                format!("cannot save {}: {error}", path.display()),
            )
        })?;
        Ok(ScreenshotResult {
            path: path.into(),
            width,
            height,
        })
    })
    .await
    .unwrap_or_else(|_| {
        Err(err(
            ErrorKind::Timeout,
            "ffmpeg did not finish decoding in time",
        ))
    });
    if result.is_err() {
        let _ = child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
        feed.abort();
        stderr.abort();
        let _ = tokio::fs::remove_file(&part).await;
        let _ = tokio::fs::remove_file(path).await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pam(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
        let mut bytes = format!(
            "P7\nWIDTH {width}\nHEIGHT {height}\nDEPTH 3\nMAXVAL 255\nTUPLTYPE RGB\nENDHDR\n"
        )
        .into_bytes();
        bytes.extend_from_slice(pixels);
        bytes
    }

    #[tokio::test]
    async fn pam_stream_keeps_only_the_last_complete_frame() {
        let mut stream = pam(1, 1, &[255, 0, 0]);
        stream.extend(pam(1, 1, &[0, 0, 255]));
        let frame = last_pam_frame(stream.as_slice()).await.unwrap();
        assert_eq!((frame.width, frame.height), (1, 1));
        assert_eq!(frame.pixels, [0, 0, 255]);
    }

    #[tokio::test]
    async fn malformed_or_truncated_pam_never_returns_a_stale_frame() {
        let mut truncated = pam(1, 1, &[255, 0, 0]);
        truncated.extend(pam(1, 1, &[0, 0]));
        for invalid in [
            vec![],
            b"P6\n1 1\n255\nabc".to_vec(),
            pam(u32::MAX, u32::MAX, &[]),
            truncated,
            b"P7\nWIDTH 1\n".to_vec(),
            format!("P7\n{}\n", "X".repeat(2048)).into_bytes(),
            b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 4\nMAXVAL 255\nENDHDR\nxxxx".to_vec(),
        ] {
            assert!(last_pam_frame(invalid.as_slice()).await.is_err());
        }
    }

    #[test]
    fn png_size_reads_the_header_and_rejects_other_bytes() {
        let mut png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR".to_vec();
        png.extend_from_slice(&800u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(png_size(&png), Some((800, 480)));
        assert_eq!(png_size(b"not a png at all, just text.."), None);
        assert_eq!(png_size(&png[..10]), None);
    }
    #[tokio::test]
    #[ignore = "requires ffmpeg with libx264; long-GOP final-frame and throughput regression"]
    async fn a_long_gop_screenshot_keeps_the_final_blue_frame() {
        let dir = std::env::temp_dir().join(format!("aa-long-gop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let h264 = dir.join("long.h264");
        let output = std::process::Command::new(autolab_native::find_ffmpeg().unwrap())
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=c=red:s=800x480:r=30", "-f", "lavfi", "-i", "color=c=blue:s=800x480:r=30", "-filter_complex", "[0:v]trim=end_frame=599,setpts=PTS-STARTPTS[red];[1:v]trim=end_frame=1,setpts=PTS-STARTPTS[blue];[red][blue]concat=n=2:v=1:a=0", "-frames:v", "600", "-c:v", "libx264", "-preset", "ultrafast", "-tune", "zerolatency", "-g", "9999", "-keyint_min", "9999", "-sc_threshold", "0", "-pix_fmt", "yuv420p", "-f", "h264"])
            .arg(&h264).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stream = std::fs::read(h264).unwrap();
        let started = std::time::Instant::now();
        let path = dir.join("last.png");
        let result = decode_to_png(stream, &path).await;
        eprintln!(
            "600-frame GOP screenshot: {:?}; result {result:?}",
            started.elapsed()
        );
        let result = result.unwrap();
        assert_eq!((result.width, result.height), (800, 480));
        let pixels = image::open(&path).unwrap().to_rgb8();
        let pixel = pixels.get_pixel(400, 240).0;
        assert!(
            pixel[2] > 240 && pixel[0] < 15 && pixel[1] < 15,
            "must keep last blue frame, not first red frame: {pixel:?}"
        );
        assert!(!path.with_extension("part.png").exists());
        let invalid = dir.join("invalid.png");
        std::fs::write(&invalid, "stale screenshot").unwrap();
        let failed = decode_to_png(b"not H.264".to_vec(), &invalid).await;
        assert!(failed.is_err());
        assert!(
            !invalid.exists(),
            "decoding failure must not leave a stale screenshot"
        );
        assert!(!invalid.with_extension("part.png").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

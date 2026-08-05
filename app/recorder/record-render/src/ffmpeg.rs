//! ffmpeg.rs — decode / encode / probe via the `ffmpeg` + `ffprobe` binaries.
//!
//! We shell out to ffmpeg (same approach as ShellX Cut) rather than link libav:
//! simpler, fewer build deps, trivially cross-platform. Binary location honours
//! `SHELLX_RECORD_FFMPEG` / `SHELLX_RECORD_FFPROBE` overrides (needed when running
//! the Windows .exe via WSL interop, where ffmpeg lives at a Windows path), else
//! falls back to `ffmpeg` / `ffprobe` on PATH.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

use record_core::{error_codes, RecordError, Result};

fn ffmpeg_bin() -> String {
    std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string())
}

fn ffprobe_bin() -> String {
    std::env::var("SHELLX_RECORD_FFPROBE").unwrap_or_else(|_| "ffprobe".to_string())
}

fn ff_err(ctx: &str, e: impl std::fmt::Display) -> RecordError {
    RecordError::new(error_codes::FFMPEG, ctx, e.to_string()).with_action(
        "ensure ffmpeg/ffprobe are installed and on PATH (or set SHELLX_RECORD_FFMPEG)",
    )
}

fn fps_str(fps: f64) -> String {
    format!("{}", fps)
}

fn rgba_frame_bytes(width: u32, height: u32, context: &str) -> Result<usize> {
    if width == 0 || height == 0 {
        return Err(RecordError::new(
            error_codes::INVALID_ARGS,
            context,
            "RGBA frame dimensions must be non-zero",
        ));
    }
    let width = usize::try_from(width).map_err(|_| {
        RecordError::new(
            error_codes::INVALID_ARGS,
            context,
            "RGBA frame width does not fit this platform",
        )
    })?;
    let height = usize::try_from(height).map_err(|_| {
        RecordError::new(
            error_codes::INVALID_ARGS,
            context,
            "RGBA frame height does not fit this platform",
        )
    })?;
    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| {
            RecordError::new(
                error_codes::INVALID_ARGS,
                context,
                "RGBA frame byte size overflows this platform",
            )
        })
}

/// Probed source stream facts.
pub struct Probe {
    pub w: u32,
    pub h: u32,
    pub fps: f64,
}

/// Probe a media file's first video stream for width/height/fps.
pub fn probe(path: &str) -> Result<Probe> {
    let out = Command::new(ffprobe_bin())
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,r_frame_rate",
            "-of",
            "csv=p=0:s=,",
            path,
        ])
        .output()
        .map_err(|e| ff_err("ffprobe spawn", e))?;
    if !out.status.success() {
        return Err(ff_err(
            "ffprobe failed",
            String::from_utf8_lossy(&out.stderr),
        ));
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let line = s.lines().next().unwrap_or("").trim();
    let parts: Vec<&str> = line.split(',').collect();
    let w: u32 = parts
        .first()
        .and_then(|x| x.trim().parse().ok())
        .unwrap_or(0);
    let h: u32 = parts
        .get(1)
        .and_then(|x| x.trim().parse().ok())
        .unwrap_or(0);
    let fps = parts
        .get(2)
        .map(|r| {
            let mut it = r.trim().split('/');
            let n: f64 = it.next().unwrap_or("30").parse().unwrap_or(30.0);
            let d: f64 = it.next().unwrap_or("1").parse().unwrap_or(1.0);
            if d != 0.0 {
                n / d
            } else {
                30.0
            }
        })
        .unwrap_or(30.0);
    if w == 0 || h == 0 {
        return Err(ff_err(
            "bad probe dims",
            format!("ffprobe returned '{line}'"),
        ));
    }
    Ok(Probe { w, h, fps })
}

/// Read exactly `buf.len()` bytes; returns the count actually filled (0 = clean
/// EOF before any byte, len = full frame, anything else = truncated tail).
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) => return Err(ff_err("read decode frame", e)),
        }
    }
    Ok(filled)
}

/// Encode `nframes` RGBA frames produced by `frame` into an MP4 (libx264).
/// `frame(i)` returns exactly `w*h*4` bytes for frame `i`.
pub fn encode_frames<F: FnMut(u64) -> Vec<u8>>(
    out: &str,
    w: u32,
    h: u32,
    fps: f64,
    nframes: u64,
    mut frame: F,
) -> Result<u64> {
    let mut child = Command::new(ffmpeg_bin())
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            &format!("{w}x{h}"),
            "-r",
            &fps_str(fps),
            "-i",
            "-",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-crf",
            "20",
            "-preset",
            "medium",
            out,
        ])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ff_err("ffmpeg encode spawn", e))?;
    {
        let mut sin = child.stdin.take().expect("encode stdin");
        for i in 0..nframes {
            let buf = frame(i);
            sin.write_all(&buf)
                .map_err(|e| ff_err("write encode frame", e))?;
        }
    } // drop stdin → EOF
    let res = child
        .wait_with_output()
        .map_err(|e| ff_err("ffmpeg encode wait", e))?;
    if !res.status.success() {
        return Err(ff_err(
            "ffmpeg encode failed",
            String::from_utf8_lossy(&res.stderr),
        ));
    }
    Ok(nframes)
}

/// Stream-render: decode `src` to RGBA frames, transform each via `compose`,
/// encode the results into `out` (MP4). `compose(frame_bytes, t_ms)` returns the
/// composited `out_w*out_h*4` RGBA bytes. Returns the frame count.
// Internal streaming-render plumbing; the arg list is the pipeline's parameters.
#[allow(clippy::too_many_arguments)]
pub fn render_pipe<F: FnMut(&[u8], u64) -> Vec<u8>>(
    src: &str,
    out: &str,
    out_w: u32,
    out_h: u32,
    fps: f64,
    audio_input: &str,
    normalize: bool,
    mut compose: F,
) -> Result<u64> {
    let p = probe(src)?;
    let frame_bytes = rgba_frame_bytes(p.w, p.h, "decode source frames")?;

    let mut dec = Command::new(ffmpeg_bin())
        .args([
            "-v", "error", "-i", src, "-f", "rawvideo", "-pix_fmt", "rgba", "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| ff_err("ffmpeg decode spawn", e))?;
    // Input 0 = our composited frames (piped rawvideo). Input 1 = the audio source
    // (the mic WAV for a narrated recording, else the original capture), mapped
    // optionally (`?` = ignore if silent) so the polished output keeps the audio.
    // The composited video (input 0, piped) MUST drive the output length. When a mic
    // audio is muxed (`normalize`), it can run LONGER than the video — it records through
    // the mic-warm period before the capture window — so we `apad` the audio + `-shortest`
    // to trim it to the video (see the normalize block below). Without that, the recording
    // clip extended past the picture and playback ran into a black-with-audio tail.
    // With no mic (source's own audio) the lengths already match. `normalize` also adds a
    // single-pass loudnorm (~-16 LUFS) so quiet narration is brought up.
    let size = format!("{out_w}x{out_h}");
    let rate = fps_str(fps);
    let mut enc_args: Vec<&str> = vec![
        "-v",
        "error",
        "-y",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "-s",
        &size,
        "-r",
        &rate,
        "-i",
        "-",
        "-i",
        audio_input,
        "-map",
        "0:v:0",
        "-map",
        "1:a:0?",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-crf",
        "20",
        "-preset",
        "medium",
    ];
    if normalize {
        // Cap the output to the VIDEO length. `apad` makes the muxed mic audio
        // effectively infinite; `-shortest` then truncates the output to the (finite,
        // piped) video — so the video drives the length and the longer mic is trimmed to
        // match. The video pipe is never cut mid-stream (the breakage the old comment
        // worried about only happened when -shortest truncated a SHORTER video to the
        // audio; apad guarantees the audio is never the shorter stream).
        enc_args.push("-af");
        enc_args.push("loudnorm=I=-16:TP=-1.5:LRA=11,apad");
        enc_args.push("-shortest");
    }
    enc_args.extend(["-c:a", "aac", out]);
    let mut enc = Command::new(ffmpeg_bin())
        .args(&enc_args)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ff_err("ffmpeg encode spawn", e))?;

    let mut din = dec.stdout.take().expect("decode stdout");
    let mut ein = enc.stdin.take().expect("encode stdin");
    let mut buf = vec![0u8; frame_bytes];
    let mut idx = 0u64;

    loop {
        let n = read_full(&mut din, &mut buf)?;
        if n == 0 {
            break; // clean EOF
        }
        if n != frame_bytes {
            break; // truncated tail frame — stop
        }
        let t_ms = (idx as f64 * 1000.0 / fps) as u64;
        let outbuf = compose(&buf, t_ms);
        if let Err(e) = ein.write_all(&outbuf) {
            // Encoder finished early (e.g. a shorter audio stream) — not an error.
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                break;
            }
            return Err(ff_err("write encode frame", e));
        }
        idx += 1;
    }

    drop(ein); // EOF to encoder
    drop(din);
    let _ = dec.wait();
    let res = enc
        .wait_with_output()
        .map_err(|e| ff_err("ffmpeg encode wait", e))?;
    if !res.status.success() {
        return Err(ff_err(
            "ffmpeg encode failed",
            String::from_utf8_lossy(&res.stderr),
        ));
    }
    Ok(idx)
}

/// Convert an MP4 to a high-quality GIF via the palettegen/paletteuse two-filter
/// pass (the standard good-looking-GIF recipe), scaled to `width` px (height auto).
pub fn mp4_to_gif(src: &str, out: &str, fps: u32, width: u32) -> Result<()> {
    let filter = format!(
        "[0:v] fps={fps},scale={width}:-1:flags=lanczos,split [a][b];[a] palettegen=stats_mode=diff [p];[b][p] paletteuse=dither=bayer:bayer_scale=3"
    );
    let status = Command::new(ffmpeg_bin())
        .args([
            "-v",
            "error",
            "-y",
            "-i",
            src,
            "-filter_complex",
            &filter,
            "-loop",
            "0",
            out,
        ])
        .status()
        .map_err(|e| ff_err("ffmpeg gif spawn", e))?;
    if !status.success() {
        return Err(ff_err("ffmpeg gif failed", format!("exit {status}")));
    }
    Ok(())
}

/// Decode a webcam video into square `bp`×`bp` RGBA frames at `fps` (center-
/// cropped to square then scaled). Returns all frames in memory (bubble-sized, so
/// small); streaming would be the upgrade for very long webcam tracks.
pub fn decode_square(src: &str, bp: u32, fps: f64) -> Result<Vec<Vec<u8>>> {
    const MAX_BUBBLE_PX: u32 = 8192;
    if bp == 0 || bp > MAX_BUBBLE_PX {
        return Err(RecordError::new(
            error_codes::INVALID_ARGS,
            "webcam decode",
            "bubble dimensions must be in 1..=8192",
        ));
    }
    if !(fps.is_finite() && fps > 0.0 && fps <= 240.0) {
        return Err(RecordError::new(
            error_codes::INVALID_ARGS,
            "webcam decode",
            "fps must be finite and in (0, 240]",
        ));
    }
    let frame_bytes = rgba_frame_bytes(bp, bp, "webcam decode")?;
    let vf = format!(
        "crop='min(iw,ih)':'min(iw,ih)',scale={bp}:{bp},fps={}",
        fps.round() as u32
    );
    let out = Command::new(ffmpeg_bin())
        .args([
            "-v", "error", "-i", src, "-vf", &vf, "-f", "rawvideo", "-pix_fmt", "rgba", "-",
        ])
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| ff_err("webcam decode spawn", e))?;
    if !out.status.success() {
        return Err(ff_err(
            "webcam decode failed",
            String::from_utf8_lossy(&out.stderr),
        ));
    }
    Ok(out
        .stdout
        .chunks(frame_bytes)
        .filter(|c| c.len() == frame_bytes)
        .map(|c| c.to_vec())
        .collect())
}

/// Grab a single RGBA frame near `t_ms` (for fast PNG previews / golden checks).
pub fn grab_frame(src: &str, t_ms: u64) -> Result<(u32, u32, Vec<u8>)> {
    let p = probe(src)?;
    let t = format!("{:.3}", t_ms as f64 / 1000.0);
    let out = Command::new(ffmpeg_bin())
        .args([
            "-v",
            "error",
            "-ss",
            &t,
            "-i",
            src,
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| ff_err("ffmpeg grab spawn", e))?;
    if !out.status.success() {
        return Err(ff_err(
            "ffmpeg grab failed",
            String::from_utf8_lossy(&out.stderr),
        ));
    }
    let want = rgba_frame_bytes(p.w, p.h, "grab frame")?;
    if out.stdout.len() != want {
        return Err(ff_err(
            "grab size mismatch",
            format!("got {} want {want}", out.stdout.len()),
        ));
    }
    Ok((p.w, p.h, out.stdout))
}

#[cfg(test)]
mod tests {
    use super::{decode_square, rgba_frame_bytes};

    #[test]
    fn rgba_geometry_rejects_zero_and_overflow_before_allocation() {
        assert_eq!(rgba_frame_bytes(2, 3, "test").unwrap(), 24);
        assert!(rgba_frame_bytes(0, 3, "test").is_err());
        assert!(rgba_frame_bytes(u32::MAX, u32::MAX, "test").is_err());
    }

    #[test]
    fn webcam_decode_rejects_invalid_geometry_and_fps_before_spawning() {
        assert!(decode_square("unused", 0, 30.0).is_err());
        assert!(decode_square("unused", 8193, 30.0).is_err());
        assert!(decode_square("unused", 64, f64::NAN).is_err());
        assert!(decode_square("unused", 64, 241.0).is_err());
    }
}

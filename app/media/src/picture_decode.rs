//! Decode a still once, without the looping input used by the renderer.
use cut_core::{error_codes, CutError};
use std::{path::Path, process::Command, time::Duration};

/// Admission check without changing the existing asynchronous admission of
/// video/audio. Probe by content, not just the suffix (a PNG may be renamed).
pub fn check_if_picture(path: &Path) -> Result<(), CutError> {
    let image = crate::probe(path).is_ok_and(|probe| probe.kind == crate::probe::kinds::IMAGE);
    let image_suffix = path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        matches!(
            s.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "bmp" | "webp" | "tif" | "tiff"
        )
    });
    if image || image_suffix {
        check(path)?;
    }
    Ok(())
}

/// No `-loop`: some malformed PNGs spin in the looping demuxer even after TERM.
/// The owned process budget also covers files whose decoder itself stalls.
pub fn check(path: &Path) -> Result<(), CutError> {
    check_with_budget(path, Duration::from_secs(10))
}

/// A media-check pass gives each child only its remaining shared deadline.
/// Admission still uses `check` and its original per-picture 10 s budget.
pub fn check_with_budget(path: &Path, budget: Duration) -> Result<(), CutError> {
    let mut command = Command::new(crate::ffmpeg::ffmpeg_bin());
    command.args(["-nostdin", "-v", "error"]);
    crate::ffmpeg::push_ffmpeg_literal_input(&mut command, path)?;
    command.args([
        "-map",
        "0:v:0",
        "-frames:v",
        "1",
        "-vf",
        "scale=1:1",
        "-pix_fmt",
        "rgb24",
        "-f",
        "rawvideo",
        "pipe:1",
    ]);
    let control =
        crate::ffmpeg::OwnedProcessControl::bounded(budget.min(Duration::from_secs(10)), || false);
    let output = crate::ffmpeg::run_owned_command(&mut command, &control, "check picture decode")?;
    if output.status.success() && output.stdout.len() == 3 {
        return Ok(());
    }
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    Err(CutError::new(
        error_codes::UNREADABLE_MEDIA,
        format!("This picture file is damaged and cannot be used: {name}"),
        format!(
            "{}: one non-looped frame could not be decoded ({}): {}",
            path.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ),
    )
    .with_suggested_action(
        "Copy or export the picture again from its original source, then import the new copy.",
    ))
}

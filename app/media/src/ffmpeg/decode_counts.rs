//! Counts in the source's own decode domain, before any proxy rate conversion.
use super::*;

/// Read declared, packet and decoded-picture counts together. Unlike a proxy's
/// frame count, `nb_read_frames` is not changed by output timestamp rounding,
/// vsync, or an encoder time base. Keep the same local-input policy and bounded
/// process ownership as the ordinary probe; only video needs decoding here.
pub fn ffprobe_decode_counts(
    path: &Path,
    timeout: std::time::Duration,
) -> Result<serde_json::Value, CutError> {
    let mut command = Command::new(ffprobe_bin());
    command.args([
        "-v",
        "error",
        "-protocol_whitelist",
        LOCAL_INPUT_PROTOCOLS,
        "-format_whitelist",
        LOCAL_INPUT_FORMATS,
        "-select_streams",
        "v",
        "-count_frames",
        "-count_packets",
        "-print_format",
        "json",
        "-show_entries",
        "stream=index,nb_frames,nb_read_frames,nb_read_packets",
    ]);
    push_ffprobe_input(&mut command, path);
    let control = super::process::bounded_current_control(timeout);
    let out = run_owned_command(&mut command, &control, "ffprobe decode count")?;
    if !out.status.success() {
        return Err(CutError::new(
            error_codes::FFMPEG,
            format!(
                "ffprobe could not count decoded pictures of {}",
                path.display()
            ),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| {
        CutError::new(
            error_codes::FFMPEG,
            "ffprobe emitted unparseable decode counts",
            e.to_string(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn source_count_obeys_its_timeout_and_inherited_cancellation() {
        let error = ffprobe_decode_counts(Path::new("unused.mp4"), Duration::ZERO).unwrap_err();
        assert_eq!(error.message, "operation timed out");
        let cancelled = OwnedProcessControl::bounded(Duration::from_secs(60), || true);
        let error = with_render_process_control(&cancelled, || {
            ffprobe_decode_counts(Path::new("unused.mp4"), Duration::from_secs(300))
        })
        .unwrap_err();
        assert_eq!(error.code, error_codes::RENDER_CANCELLED);
        let expired = OwnedProcessControl::bounded(Duration::ZERO, || false);
        let error = with_render_process_control(&expired, || {
            ffprobe_decode_counts(Path::new("unused.mp4"), Duration::from_secs(300))
        })
        .unwrap_err();
        assert_eq!(error.message, "operation timed out");
    }
}

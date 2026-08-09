//! Bounded, owned media verification used before publication and before recovery.

use std::path::Path;
use std::process::{Command, ExitStatus};
use std::time::Duration;

use cut_media::ffmpeg::{run_owned_command, OwnedProcessControl};
use serde::Deserialize;

use crate::{ManifestError, MediaFacts};

const MEDIA_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_PROBE_BYTES: u64 = 1024 * 1024;

#[derive(Deserialize)]
struct Probe {
    format: ProbeFormat,
    streams: Vec<ProbeStream>,
}
#[derive(Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
}
#[derive(Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    nb_read_frames: Option<String>,
}

/// Probe container duration, count all video frames, and decode its video stream.
/// Every helper uses CUT-JOB-02's owned tree and concurrently capped drains, so
/// verbose output cannot hold pipes open or outlive this recovery operation.
pub fn verify_media(ffmpeg: &str, ffprobe: &str, path: &Path) -> Result<MediaFacts, ManifestError> {
    verify_checkpoint_media(ffmpeg, ffprobe, path)?.ok_or_else(|| {
        ManifestError::Invalid("checkpoint media did not meet the playable video contract".into())
    })
}

/// Distinguish evidence that a completed file is malformed from a verification
/// outage. Recovery may quarantine only `Ok(None)`: a missing tool, a timeout,
/// a failed probe, or unparseable tool output must leave every candidate and
/// receipt untouched for a later retry.
pub(crate) fn verify_checkpoint_media(
    ffmpeg: &str,
    ffprobe: &str,
    path: &Path,
) -> Result<Option<MediaFacts>, ManifestError> {
    let mut probe = Command::new(ffprobe);
    probe
        .args([
            "-v",
            "error",
            "-count_frames",
            "-show_entries",
            "format=duration:stream=codec_type,nb_read_frames",
            "-of",
            "json",
        ])
        .arg(path);
    let probe = bounded_output(&mut probe, "probe checkpoint media")?;
    let status = probe.status;
    let bytes = probe.stdout;
    if !status.success() || u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_PROBE_BYTES {
        return Err(ManifestError::Invalid(
            "ffprobe verification was unavailable or did not produce bounded media facts".into(),
        ));
    }
    let probe: Probe = serde_json::from_slice(&bytes)?;
    let Some(duration_ms) = probe
        .format
        .duration
        .as_deref()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .and_then(|value| u64::try_from((value * 1000.0).round() as u128).ok())
    else {
        return Ok(None);
    };
    let Some(video) = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"))
    else {
        return Ok(None);
    };
    let Some(decoded_video_frames) = video
        .nb_read_frames
        .as_deref()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|frames| *frames > 0)
    else {
        return Ok(None);
    };
    let has_audio = probe
        .streams
        .iter()
        .any(|stream| stream.codec_type.as_deref() == Some("audio"));
    let mut decode = Command::new(ffmpeg);
    decode
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0:v:0", "-f", "null", "-"]);
    if !bounded_status(&mut decode, "decode checkpoint video")?.success() {
        return Ok(None);
    }
    if has_audio {
        let mut audio = Command::new(ffmpeg);
        audio
            .args(["-v", "error", "-i"])
            .arg(path)
            .args(["-map", "0:a:0", "-f", "null", "-"]);
        if !bounded_status(&mut audio, "decode checkpoint audio")?.success() {
            return Ok(None);
        }
    }
    Ok(Some(MediaFacts {
        duration_ms,
        decoded_video_frames,
        has_audio,
    }))
}

pub(crate) fn bounded_status(
    command: &mut Command,
    context: &str,
) -> Result<ExitStatus, ManifestError> {
    Ok(bounded_output(command, context)?.status)
}

fn bounded_output(
    command: &mut Command,
    context: &str,
) -> Result<std::process::Output, ManifestError> {
    let control = OwnedProcessControl::bounded(MEDIA_TIMEOUT, || false);
    run_owned_command(command, &control, context)
        .map_err(|error| ManifestError::Invalid(format!("{context}: {error}")))
}

pub(crate) fn matches_expected(expected: &MediaFacts, actual: &MediaFacts) -> bool {
    expected.decoded_video_frames == actual.decoded_video_frames
        && expected.has_audio == actual.has_audio
        && expected.duration_ms.abs_diff(actual.duration_ms) <= 20
}

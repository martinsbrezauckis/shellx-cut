//! What the stitch tells the person about a saved take that is not exact.
//!
//! A check that fails no longer discards the take: it is saved with what was
//! captured and these sentences say what is wrong with it. They are recorded
//! beside the output with its hash, so a later stitch of the same capture
//! (recovery, a repeated Stop) accepts that same file with the same notes.
//! Only notes this engine saved here count: the notes file and the take have
//! origin records outside the capture folder (`cut_media::cache_trust`). Notes
//! that came with a folder from elsewhere never excuse a take that fails the
//! checkpoint contract.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::manifest::{is_plain_regular_file, sha256};
use crate::{Checkpoint, ManifestError, MediaFacts};

const NOTES_FILE: &str = "stitch-notes.json";
const NOTES_SCHEMA: &str = "shellx-cut/stitch-notes/1";
const MAX_NOTES_BYTES: u64 = 64 * 1024;

#[derive(Serialize, Deserialize)]
struct Recorded {
    schema: String,
    output: String,
    output_sha256: String,
    notes: Vec<String>,
}

fn seconds(ms: u64) -> String {
    format!("{:.1} s", ms as f64 / 1000.0)
}

pub(super) fn lost_segment(segment: &Checkpoint, error: &ManifestError) -> String {
    format!(
        "The screen from {} to {} could not be read back, so the picture before it is shown for that time ({error}).",
        seconds(segment.facts.start_ms),
        seconds(segment.facts.end_ms),
    )
}

pub(super) fn contract_mismatches(
    media: &MediaFacts,
    admitted: Option<record_core::FrameRate>,
    (parts_ms, parts_frames): (u64, u64),
    elapsed_ms: u64,
    (parts_tolerance_ms, elapsed_tolerance_ms): (u64, u64),
) -> Vec<String> {
    let mut notes = Vec::new();
    if media.has_audio {
        notes.push("The saved screen video unexpectedly contains a sound track.".into());
    }
    if let Some(rate) = admitted {
        if media.avg_frame_rate != Some(rate) || media.r_frame_rate != Some(rate) {
            let measured = media.avg_frame_rate.map_or("an unknown".into(), |found| {
                format!("{:.2}", found.num as f64 / found.den.max(1) as f64)
            });
            notes.push(format!(
                "The saved video's frame timing is uneven: it averages {measured} frames per second instead of {}.",
                rate.num.div_ceil(rate.den.max(1)),
            ));
        }
    }
    if media.decoded_video_frames.abs_diff(parts_frames) > 1 {
        notes.push(format!(
            "The saved video has {} frames, but its parts have {parts_frames}.",
            media.decoded_video_frames
        ));
    }
    if media.duration_ms.abs_diff(parts_ms) > parts_tolerance_ms {
        notes.push(format!(
            "The saved video is {} long, but its parts add up to {}.",
            seconds(media.duration_ms),
            seconds(parts_ms)
        ));
    }
    if media.duration_ms.abs_diff(elapsed_ms) > elapsed_tolerance_ms {
        notes.push(format!(
            "The saved video is {} long, but the screen was recorded for {}.",
            seconds(media.duration_ms),
            seconds(elapsed_ms)
        ));
    }
    notes
}

pub(super) fn record(
    root: &Path,
    part: &Path,
    output_name: &str,
    notes: &[String],
) -> Result<String, ManifestError> {
    let output_sha256 = sha256(part)?;
    let recorded = Recorded {
        schema: NOTES_SCHEMA.into(),
        output: output_name.into(),
        output_sha256: output_sha256.clone(),
        notes: notes.to_vec(),
    };
    let path = root.join(NOTES_FILE);
    crate::atomic::replace_synced(&path, &serde_json::to_vec(&recorded)?)
        .map_err(|source| ManifestError::Io { path, source })?;
    Ok(output_sha256)
}

/// After the take `output` (whose hash [`record`] returned) is published with
/// notes: record that this engine saved both, so a later stitch here keeps
/// accepting the take with them.
pub(super) fn remember_take(output: &Path, output_sha256: &str) {
    let Some(root) = output.parent() else {
        return;
    };
    cut_media::cache_trust::remember_hash(output, output_sha256);
    cut_media::cache_trust::remember(&root.join(NOTES_FILE));
}

/// The notes this engine recorded for exactly this output file, if any.
pub(super) fn recorded_for(root: &Path, output: &Path) -> Option<Vec<String>> {
    let path = root.join(NOTES_FILE);
    if !is_plain_regular_file(&path).ok()?
        || std::fs::metadata(&path).ok()?.len() > MAX_NOTES_BYTES
        || !cut_media::cache_trust::trusted(&path)
    {
        return None;
    }
    let recorded: Recorded = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    let name = output.file_name()?.to_str()?;
    let hash = sha256(output).ok()?;
    (recorded.schema == NOTES_SCHEMA
        && recorded.output == name
        && hash == recorded.output_sha256
        && cut_media::cache_trust::trusted_hash(output, &hash))
    .then_some(recorded.notes)
}

const PROGRESS_FILE: &str = "stitch-progress.json";

/// How far a running stitch is, for the person waiting on Stop. Best effort.
pub(super) fn write_progress(root: &Path, parts_done: usize, parts_total: usize) {
    let body = serde_json::json!({ "parts_done": parts_done, "parts_total": parts_total });
    let _ = crate::atomic::replace_synced(&root.join(PROGRESS_FILE), body.to_string().as_bytes());
}

/// `(parts_done, parts_total)` of the capture's stitch, when one has started.
pub fn stitch_progress(root: &Path) -> Option<(u64, u64)> {
    let path = root.join(PROGRESS_FILE);
    if !is_plain_regular_file(&path).ok()? || std::fs::metadata(&path).ok()?.len() > 4096 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    Some((
        value["parts_done"].as_u64()?,
        value["parts_total"].as_u64()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media(duration_ms: u64, frames: u64, rate: (u64, u64)) -> MediaFacts {
        MediaFacts {
            duration_ms,
            decoded_video_frames: frames,
            has_audio: false,
            width: None,
            height: None,
            codec_name: None,
            avg_frame_rate: record_core::FrameRate::new(rate.0, rate.1).ok(),
            r_frame_rate: record_core::FrameRate::new(30, 1).ok(),
        }
    }

    #[test]
    fn progress_is_readable_while_the_stitch_runs() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(stitch_progress(dir.path()), None);
        write_progress(dir.path(), 2, 5);
        assert_eq!(stitch_progress(dir.path()), Some((2, 5)));
    }

    #[test]
    fn an_exact_take_has_no_notes_and_each_mismatch_is_named() {
        let thirty = record_core::FrameRate::new(30, 1).ok();
        let exact = media(62_233, 1_867, (30, 1));
        assert!(
            contract_mismatches(&exact, thirty, (62_233, 1_867), 62_233, (120, 1_120)).is_empty()
        );
        let off = media(61_000, 1_830, (1_829, 61));
        let notes = contract_mismatches(&off, thirty, (62_233, 1_867), 64_000, (120, 1_120));
        assert_eq!(notes.len(), 4, "{notes:?}");
        assert!(
            notes[0].contains("averages 29.98 frames per second instead of 30"),
            "{notes:?}"
        );
        assert!(notes[3].contains("recorded for 64.0 s"));
    }
}

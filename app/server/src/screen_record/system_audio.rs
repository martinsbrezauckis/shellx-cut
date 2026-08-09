//! Persisted system-audio timing and timeline placement.
//!
//! Native system-audio WAVs publish the first real backend packet offset on the
//! shared recorder clock. A finalized no-packet WAV explicitly records an
//! unknown offset and polish refuses to place it rather than shifting it onto
//! the gap-padded timeline. Older captures with no sidecar retain legacy
//! zero-offset behavior.

use super::system_audio_capture::capture_system_audio_until;
use cut_core::{error_codes, CutError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

mod publication;

pub(crate) const SYSTEM_AUDIO_TIMING_FILE: &str = "system-audio.json";
const SYSTEM_AUDIO_TIMING_SCHEMA: &str = "shellx-cut/system-audio-timing/1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SystemAudioTiming {
    pub schema: String,
    pub first_packet_offset_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SystemAudioPlacement {
    Insert { at_ms: u64, source_duration_ms: u64 },
    Skip { warning: String },
}

pub(crate) fn timing_path(capture_dir: &Path) -> PathBuf {
    capture_dir.join(SYSTEM_AUDIO_TIMING_FILE)
}

pub(crate) fn capture_system_audio_artifact(
    out: &Path,
    duration_ms: Option<u64>,
    stop: Arc<AtomicBool>,
    capture_started: Instant,
) -> Result<(), CutError> {
    let capture_dir = out.parent().ok_or_else(|| {
        CutError::new(
            error_codes::IO,
            "system-audio output has no parent directory",
            "the capture output must remain inside its capture directory",
        )
    })?;
    // Publish a durable intent marker before the recorder can publish a WAV. If
    // cutd dies after the WAV rename but before its timing receipt, a later polish
    // refuses the incomplete current-format artifact instead of treating it as a
    // legacy zero-offset recording.
    publication::begin_timing_publication(capture_dir)?;
    let capture = match capture_system_audio_until(out, duration_ms, stop, capture_started) {
        Ok(capture) => capture,
        Err(error) => {
            publication::discard_incomplete_timing_publication(out, capture_dir);
            return Err(error);
        }
    };
    let timing = SystemAudioTiming {
        schema: SYSTEM_AUDIO_TIMING_SCHEMA.to_string(),
        first_packet_offset_ms: capture.and_then(|capture| capture.first_packet_offset_ms),
    };
    if let Err(error) = publication::write_timing(capture_dir, &timing) {
        // Do not leave a WAV that legacy placement would incorrectly assume
        // starts at zero if its required timing sidecar was not saved.
        publication::discard_incomplete_timing_publication(out, capture_dir);
        return Err(error);
    }
    if let Err(error) = publication::clear_timing_publication(capture_dir) {
        publication::discard_incomplete_timing_publication(out, capture_dir);
        return Err(error);
    }
    Ok(())
}

pub(crate) fn read_timing(capture_dir: &Path) -> Result<Option<SystemAudioTiming>, CutError> {
    if publication::timing_publication_is_pending(capture_dir)? {
        let pending = publication::pending_timing_path(capture_dir);
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "system-audio timing publication at {} is incomplete",
                pending.display()
            ),
            "retry the capture; Cut will not place a current-format system WAV at a legacy zero offset",
        ));
    }
    let path = timing_path(capture_dir);
    if !publication::local_regular_file_or_absent(&path, "inspect system-audio timing")? {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).map_err(|error| {
        CutError::new(
            error_codes::IO,
            format!(
                "could not read system-audio timing at {}: {error}",
                path.display()
            ),
            "retry the capture or remove the incomplete system-audio timing sidecar",
        )
    })?;
    let timing: SystemAudioTiming = serde_json::from_slice(&bytes).map_err(|error| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "system-audio timing at {} is invalid: {error}",
                path.display()
            ),
            "retry the capture so its system-audio timing sidecar is rewritten",
        )
    })?;
    if timing.schema != SYSTEM_AUDIO_TIMING_SCHEMA {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "system-audio timing at {} has an unsupported schema",
                path.display()
            ),
            "retry the capture with this version of ShellX Cut",
        ));
    }
    Ok(Some(timing))
}

pub(crate) fn plan_placement(
    insert_at_ms: u64,
    video_duration_ms: Option<u64>,
    system_duration_ms: Option<u64>,
    timing: Option<&SystemAudioTiming>,
) -> SystemAudioPlacement {
    let Some(timing) = timing else {
        // Older captures began at the recorder clock by contract.
        return match (video_duration_ms, system_duration_ms) {
            (Some(video), Some(system)) if video > 0 => SystemAudioPlacement::Insert {
                at_ms: insert_at_ms,
                source_duration_ms: video.min(system),
            },
            _ => SystemAudioPlacement::Skip {
                warning: "system audio has no probe duration; skipped to avoid an unbounded timeline clip".into(),
            },
        };
    };
    let Some(offset_ms) = timing.first_packet_offset_ms else {
        return SystemAudioPlacement::Skip {
            warning:
                "system audio has no proven first-packet offset; no system-audio clip was inserted"
                    .into(),
        };
    };
    let Some(at_ms) = insert_at_ms.checked_add(offset_ms) else {
        return SystemAudioPlacement::Skip {
            warning: "system-audio packet offset overflowed the timeline position".into(),
        };
    };
    let (Some(video_duration_ms), Some(system_duration_ms)) =
        (video_duration_ms, system_duration_ms)
    else {
        return SystemAudioPlacement::Skip {
            warning: "system audio has no probe duration; skipped to preserve its recorded offset"
                .into(),
        };
    };
    let Some(remaining_video_ms) = video_duration_ms.checked_sub(offset_ms) else {
        return SystemAudioPlacement::Skip {
            warning:
                "system audio started after the recording ended; no system-audio clip was inserted"
                    .into(),
        };
    };
    let source_duration_ms = remaining_video_ms.min(system_duration_ms);
    if source_duration_ms == 0 {
        return SystemAudioPlacement::Skip {
            warning: "system audio has no playable duration after its packet offset".into(),
        };
    }
    SystemAudioPlacement::Insert {
        at_ms,
        source_duration_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        plan_placement, publication, read_timing, timing_path, SystemAudioPlacement,
        SystemAudioTiming,
    };
    use cut_core::error_codes;

    fn timing(offset_ms: Option<u64>) -> SystemAudioTiming {
        SystemAudioTiming {
            schema: "shellx-cut/system-audio-timing/1".into(),
            first_packet_offset_ms: offset_ms,
        }
    }

    #[test]
    fn delayed_packet_offsets_create_honest_shorter_placements() {
        for (offset, expected_at, expected_duration) in
            [(0, 100, 3_000), (37, 137, 2_963), (2_350, 2_450, 650)]
        {
            assert_eq!(
                plan_placement(100, Some(3_000), Some(5_000), Some(&timing(Some(offset)))),
                SystemAudioPlacement::Insert {
                    at_ms: expected_at,
                    source_duration_ms: expected_duration,
                }
            );
        }
    }

    #[test]
    fn absent_sidecar_keeps_legacy_zero_offset() {
        assert_eq!(
            plan_placement(100, Some(3_000), Some(5_000), None),
            SystemAudioPlacement::Insert {
                at_ms: 100,
                source_duration_ms: 3_000,
            }
        );
    }

    #[test]
    fn no_packet_sidecar_skips_empty_system_clip() {
        assert!(matches!(
            plan_placement(0, Some(3_000), Some(5_000), Some(&timing(None))),
            SystemAudioPlacement::Skip { .. }
        ));
    }

    #[test]
    fn missing_sidecar_is_backward_compatible() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_timing(dir.path()).unwrap(), None);
        assert_eq!(
            timing_path(dir.path()),
            dir.path().join("system-audio.json")
        );
    }

    #[test]
    fn pending_current_capture_never_falls_back_to_legacy_zero_offset() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("system.wav"), b"current-format-wav").unwrap();
        std::fs::write(publication::pending_timing_path(dir.path()), b"pending").unwrap();

        let error = read_timing(dir.path()).unwrap_err();
        assert_eq!(error.code, error_codes::INVALID_ARGS);
        assert!(error.message.contains("incomplete"));
    }

    #[test]
    fn timing_sidecar_round_trips_the_packet_offset_contract() {
        let dir = tempfile::tempdir().unwrap();
        let expected = timing(Some(2_350));
        publication::begin_timing_publication(dir.path()).unwrap();
        assert!(publication::timing_publication_is_pending(dir.path()).unwrap());
        publication::write_timing(dir.path(), &expected).unwrap();
        assert!(publication::timing_publication_is_pending(dir.path()).unwrap());
        publication::clear_timing_publication(dir.path()).unwrap();
        assert_eq!(read_timing(dir.path()).unwrap(), Some(expected));
        assert!(!publication::pending_timing_path(dir.path()).exists());
        assert!(!dir.path().join("system-audio.json.pending.part").exists());
    }

    #[cfg(unix)]
    #[test]
    fn timing_publication_never_follows_pending_or_legacy_part_links() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let pending_target = outside.path().join("pending-target");
        let part_target = outside.path().join("part-target");
        std::fs::write(&pending_target, b"pending outside").unwrap();
        std::fs::write(&part_target, b"part outside").unwrap();
        let pending = publication::pending_timing_path(dir.path());
        symlink(&pending_target, &pending).unwrap();
        assert!(publication::begin_timing_publication(dir.path()).is_err());
        assert_eq!(std::fs::read(&pending_target).unwrap(), b"pending outside");
        std::fs::remove_file(&pending).unwrap();

        publication::begin_timing_publication(dir.path()).unwrap();
        symlink(
            &part_target,
            dir.path().join("system-audio.json.pending.part"),
        )
        .unwrap();
        publication::write_timing(dir.path(), &timing(Some(37))).unwrap();
        assert_eq!(std::fs::read(&part_target).unwrap(), b"part outside");
        assert!(
            std::fs::symlink_metadata(dir.path().join("system-audio.json.pending.part"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}

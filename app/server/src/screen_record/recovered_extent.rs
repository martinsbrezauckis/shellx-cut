//! What a recovered recording kept, against how long it was recorded.
//!
//! A recording that is interrupted (the app was killed, the computer slept or
//! lost power) keeps its video only up to the last checkpoint the recorder
//! finished, while the sound files are written every second. The person must
//! be told both numbers: "Recovered 15 s of about 20 s. The last 5 s were not
//! saved before the recording stopped." This module measures them from the
//! files the recording left and words them; the same text and fields are used
//! for every system, because recovery runs in the engine.
//!
//! Evidence of how long the recording ran, from strongest to weakest:
//! - the sound files: their samples were written while the recording ran, so
//!   their length is a lower bound of the recorded time;
//! - the last checkpoint that was opened: the recording was still running when
//!   it began;
//! - the checkpoints that were finished.
//!
//! `lost_tail_lower_bound_ms` is the recorded time this evidence proves minus
//! what was recovered. When an unfinished video part exists but nothing tells
//! how long it ran, the loss is reported as not known rather than as zero.

use std::path::Path;

use record_recovery::CaptureManifest;

/// The sound file is written about once a second: a difference under this is
/// not evidence of lost picture.
pub(crate) const SOUND_SPOOL_SLACK_MS: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Extent {
    /// Length of what was recovered (the video, or the sound when only sound
    /// was saved).
    pub(crate) recovered_ms: u64,
    /// The recorded time the files prove; the recording may have run longer.
    pub(crate) recorded_ms_at_least: u64,
    /// `recorded_ms_at_least - recovered_ms`.
    pub(crate) lost_tail_lower_bound_ms: u64,
    /// An unfinished part shows the recording went on after what was saved,
    /// but nothing tells for how long.
    pub(crate) longer_unknown: bool,
    /// Sound of the lost seconds exists in the recording's folder.
    pub(crate) sound_of_lost_kept: bool,
}

/// Length of a WAV file the recorder wrote (or the playable copy recovery
/// made of an unfinished one), or `None` when it cannot be read.
pub(crate) fn sound_ms(path: &Path) -> Option<u64> {
    let reader = hound::WavReader::open(path).ok()?;
    let rate = u64::from(reader.spec().sample_rate);
    (rate > 0).then(|| u64::from(reader.duration()) * 1_000 / rate)
}

/// Length of a sound file that may still be unfinished: a streaming writer
/// leaves zero sizes in its header, so the length is counted from the bytes
/// that follow the 44-byte header. Used before recovery (the notice); recovery
/// itself measures the playable copy it makes.
pub(crate) fn sound_ms_even_if_unfinished(path: &Path) -> Option<u64> {
    if let Some(ms) = sound_ms(path).filter(|ms| *ms > 0) {
        return Some(ms);
    }
    let reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();
    let frame_bytes = u64::from(spec.channels) * u64::from(spec.bits_per_sample / 8);
    let rate = u64::from(spec.sample_rate);
    if frame_bytes == 0 || rate == 0 {
        return None;
    }
    let bytes = std::fs::metadata(path).ok()?.len().saturating_sub(44);
    Some(bytes / frame_bytes * 1_000 / rate)
}

/// End of the recovered video on the capture clock: the end of the last
/// checkpoint that recovery kept.
fn kept_video_ms(manifest: &CaptureManifest) -> u64 {
    // A take saved whole (at close) keeps every checkpoint.
    if manifest
        .receipt
        .as_ref()
        .is_some_and(|receipt| receipt.state == record_recovery::RecoveryState::Complete)
    {
        return manifest
            .checkpoints
            .last()
            .map_or(0, |checkpoint| checkpoint.facts.end_ms);
    }
    let kept = manifest
        .receipt
        .as_ref()
        .map_or(0, |receipt| receipt.recovered_segments as usize);
    kept.checked_sub(1)
        .and_then(|index| manifest.checkpoints.get(index))
        .map_or(0, |checkpoint| checkpoint.facts.end_ms)
}

/// Measure from plain facts (kept apart from file reading so every rule is
/// unit-tested).
pub(crate) fn measure_from(
    video_kept_ms: Option<u64>,
    committed_end_ms: u64,
    open_segment_start_ms: Option<u64>,
    sound_ms: Option<u64>,
) -> Extent {
    let sound = sound_ms.unwrap_or(0);
    let recovered_ms = video_kept_ms.unwrap_or(sound);
    let recorded_ms_at_least = recovered_ms
        .max(committed_end_ms)
        .max(open_segment_start_ms.unwrap_or(0))
        .max(sound);
    let lost_tail_lower_bound_ms = recorded_ms_at_least - recovered_ms;
    // Sound shorter than a second past the picture proves nothing more; an
    // open part with no longer sound means the length after it is unknown.
    let sound_covers_open = sound_ms.is_some_and(|sound| {
        open_segment_start_ms.is_none_or(|open| sound + SOUND_SPOOL_SLACK_MS > open)
    });
    let longer_unknown = open_segment_start_ms.is_some() && !sound_covers_open;
    Extent {
        recovered_ms,
        recorded_ms_at_least,
        lost_tail_lower_bound_ms,
        longer_unknown,
        sound_of_lost_kept: video_kept_ms.is_some() && sound > recovered_ms + SOUND_SPOOL_SLACK_MS,
    }
}

/// Measure a sealed capture: `video` says whether its video is recovered,
/// `sounds` are the sound files recovery uses.
pub(crate) fn measure(capture_dir: &Path, video: bool, sounds: &[Option<&Path>]) -> Extent {
    let manifest = record_recovery::read_manifest(capture_dir).ok();
    let sound = sounds
        .iter()
        .flatten()
        .filter_map(|path| sound_ms_even_if_unfinished(path))
        .max();
    let (kept, committed, open) = manifest.as_ref().map_or((0, 0, None), |manifest| {
        (
            kept_video_ms(manifest),
            manifest
                .checkpoints
                .last()
                .map_or(0, |checkpoint| checkpoint.facts.end_ms),
            manifest.open_segment_start_ms(),
        )
    });
    measure_from(video.then_some(kept), committed, open, sound)
}

/// "15 s", "1 min 5 s", rounded to whole seconds.
pub(crate) fn duration_words(ms: u64) -> String {
    let seconds = (ms + 500) / 1_000;
    match (seconds / 60, seconds % 60) {
        (0, s) => format!("{s} s"),
        (m, 0) => format!("{m} min"),
        (m, s) => format!("{m} min {s} s"),
    }
}

impl Extent {
    /// The sentence a person reads about what was recovered. The same pattern
    /// is used on every system.
    /// Empty when the files tell nothing (no checkpoint and no sound length).
    #[cfg(test)]
    pub(crate) fn sentence(&self) -> String {
        self.sentence_after_placing(false)
    }

    /// As [`Self::sentence`]; `sound_placed` says the sound of the lost
    /// seconds was placed on the timeline after the picture.
    pub(crate) fn sentence_after_placing(&self, sound_placed: bool) -> String {
        if self.recovered_ms == 0 && self.recorded_ms_at_least == 0 && !self.longer_unknown {
            return String::new();
        }
        let recovered = duration_words(self.recovered_ms);
        if self.lost_tail_lower_bound_ms >= SOUND_SPOOL_SLACK_MS {
            let mut text = format!(
                "Recovered {recovered} of about {}. The last {} were not saved before the recording stopped.",
                duration_words(self.recorded_ms_at_least),
                duration_words(self.lost_tail_lower_bound_ms),
            );
            if self.longer_unknown {
                text.push_str(" It may have run longer.");
            }
            if sound_placed {
                text.push_str(" Their sound was recovered and placed after the picture.");
            } else if self.sound_of_lost_kept {
                text.push_str(" The sound of those seconds stays in the recording's folder.");
            }
            text
        } else if self.longer_unknown {
            format!(
                "Recovered {recovered}. The recording went on after that, but nothing after {recovered} was saved before it stopped, and how long it ran is not known."
            )
        } else {
            format!("Recovered {recovered}, all that was recorded.")
        }
    }
}

impl Extent {
    /// The same truth BEFORE the person presses recover: what can be recovered
    /// against what the files prove was recorded. Future tense, because nothing
    /// has been added yet; the numbers are the ones `recover` will report.
    pub(crate) fn notice_sentence(&self) -> String {
        if self.recovered_ms == 0 && self.recorded_ms_at_least == 0 && !self.longer_unknown {
            return String::new();
        }
        let recovered = duration_words(self.recovered_ms);
        if self.lost_tail_lower_bound_ms >= SOUND_SPOOL_SLACK_MS {
            let mut text = format!(
                "About {recovered} of about {} can be recovered; the last {} were not saved before the recording stopped.",
                duration_words(self.recorded_ms_at_least),
                duration_words(self.lost_tail_lower_bound_ms),
            );
            if self.longer_unknown {
                text.push_str(" It may have run longer.");
            }
            text
        } else if self.longer_unknown {
            format!(
                "About {recovered} can be recovered. The recording went on after that, but nothing after {recovered} was saved before it stopped, and how long it ran is not known."
            )
        } else {
            format!("About {recovered} can be recovered, all that was recorded.")
        }
    }
}

#[cfg(test)]
#[path = "recovered_extent_tests.rs"]
mod tests;

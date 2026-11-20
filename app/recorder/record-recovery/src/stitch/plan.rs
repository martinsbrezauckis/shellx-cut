//! Where each checkpoint sits on the capture clock in the stitched take.
//!
//! A screen recorder repeats the last picture until the next one arrives
//! (OBS renders its scene at a fixed rate; ScreenCaptureKit and Windows
//! Graphics Capture deliver a frame only when the screen changes). The
//! stitched take follows the same rule: a gap between two checkpoints holds
//! the last frame of the earlier one. Only the time before the very first
//! frame, where no earlier picture exists, shows the first frame instead.

use crate::{Checkpoint, ManifestError, MediaFacts};

// A native encoder can drain a few already-captured frames after the Stop clock
// is sampled. Keep this separate from concat/output tolerance: a native
// GStreamer capture measured 7_033 ms while its verified 25 fps file held
// 7_160 ms (179 frames). Larger overruns still indicate inconsistent input.
pub(super) const CAPTURE_DRAIN_TOLERANCE_MS: u64 = 200;

/// No recording lasts a week; a journal that says a part or the whole take
/// does is not believed. It bounds the frames, hold repeats and part list a
/// stitch builds from a journal that came with a capture folder from elsewhere
/// (about 600,000 one-second hold units at most).
pub(crate) const MAX_TAKE_MS: u64 = 7 * 24 * 60 * 60 * 1_000;

/// The capture-clock time one checkpoint occupies in the stitched take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SegmentPlan {
    /// Where this part begins on the take's clock (0 for the first part).
    /// Frame boundaries are rounded from these absolute times, so rounding
    /// never accumulates over many parts.
    pub start_ms: u64,
    /// Where it ends on the take's clock: where the next part starts, so parts
    /// tile the take exactly even when a native encoder drained a few
    /// milliseconds past its measured span.
    pub end_ms: u64,
    /// Time before this segment's first frame that shows that first frame
    /// (only the first stitched segment has any).
    pub lead_ms: u64,
    /// The segment's own measured capture span (first frame to its end).
    pub span_ms: u64,
    /// Time after the segment, until the next stitched segment starts, that
    /// holds this segment's last frame.
    pub hold_ms: u64,
    /// Media duration the checkpoint file itself holds.
    pub media_ms: u64,
}

impl SegmentPlan {
    pub(crate) fn target_ms(&self) -> u64 {
        self.lead_ms
            .saturating_add(self.span_ms)
            .saturating_add(self.hold_ms)
    }
}

/// Plan every segment against its neighbours in `segments` (the ones that
/// will actually be stitched, in order).
pub(crate) fn plan_segments(
    segments: &[Checkpoint],
    media: &[MediaFacts],
) -> Result<Vec<SegmentPlan>, ManifestError> {
    if segments.len() != media.len() {
        return Err(ManifestError::Invalid(
            "stitch plan has no media facts for every checkpoint".into(),
        ));
    }
    let plans = segments
        .iter()
        .zip(media)
        .enumerate()
        .map(|(index, (segment, facts))| {
            let next_start_ms = segments.get(index + 1).map(|next| next.facts.start_ms);
            plan_segment(index, segment, facts, next_start_ms)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let total = plans
        .iter()
        .fold(0_u64, |total, plan| total.saturating_add(plan.target_ms()));
    if total > MAX_TAKE_MS {
        return Err(ManifestError::Invalid(format!(
            "the checkpoints claim a take longer than any recording ({total} ms)"
        )));
    }
    Ok(plans)
}

/// Plan one segment. `next_start_ms` is where the next stitched segment's
/// first frame sits (`None` for the last one). A recorder can prepare a
/// segment before the next checkpoint is published, as soon as it knows that
/// first frame time; the final stitch then computes the same plan.
pub(crate) fn plan_segment(
    index: usize,
    segment: &Checkpoint,
    media: &MediaFacts,
    next_start_ms: Option<u64>,
) -> Result<SegmentPlan, ManifestError> {
    if segment.facts.event_offset_ms != segment.facts.start_ms {
        return Err(ManifestError::Invalid(
            "checkpoint event offset differs from capture start".into(),
        ));
    }
    let hold_ms = match next_start_ms {
        Some(next_start_ms) => next_start_ms
            .checked_sub(segment.facts.end_ms)
            .ok_or_else(|| ManifestError::Invalid("checkpoint facts overlap".into()))?,
        None => 0,
    };
    let lead_ms = if index == 0 {
        segment.facts.start_ms
    } else {
        0
    };
    let span_ms = capture_span_ms(segment, media)?;
    if lead_ms.saturating_add(span_ms).saturating_add(hold_ms) > MAX_TAKE_MS {
        return Err(ManifestError::Invalid(format!(
            "checkpoint {} claims a part longer than any recording ({} ms)",
            segment.sequence,
            lead_ms.saturating_add(span_ms).saturating_add(hold_ms)
        )));
    }
    Ok(SegmentPlan {
        start_ms: segment.facts.start_ms - lead_ms,
        end_ms: next_start_ms.unwrap_or(segment.facts.start_ms.saturating_add(span_ms)),
        lead_ms,
        span_ms,
        hold_ms,
        media_ms: media.duration_ms,
    })
}

/// Return the capture-clock span this segment must occupy. A closed native encoder
/// can deliver fewer frames than elapsed wall time; stitching must preserve that
/// loss as cloned trailing video, not erase it. Media that materially exceeds its
/// independently measured span remains inconsistent and is refused.
pub(crate) fn capture_span_ms(
    segment: &Checkpoint,
    media: &MediaFacts,
) -> Result<u64, ManifestError> {
    let observed = segment
        .facts
        .end_ms
        .checked_sub(segment.facts.start_ms)
        .ok_or_else(|| ManifestError::Invalid("checkpoint capture span is negative".into()))?;
    if media.duration_ms > observed.saturating_add(CAPTURE_DRAIN_TOLERANCE_MS) {
        return Err(ManifestError::Invalid(format!(
            "checkpoint media exceeds its observed capture span: segment={} media={}ms observed={}ms tolerance={}ms",
            segment.sequence, media.duration_ms, observed, CAPTURE_DRAIN_TOLERANCE_MS,
        )));
    }
    Ok(observed.max(media.duration_ms))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CheckpointFacts;

    fn checkpoint(sequence: u64, start_ms: u64, end_ms: u64) -> Checkpoint {
        Checkpoint {
            sequence,
            file: format!("checkpoints/segment-{sequence:06}.mp4"),
            bytes: 1,
            sha256: String::new(),
            media: None,
            held_last_frame_ms: None,
            facts: CheckpointFacts {
                start_ms,
                end_ms,
                event_offset_ms: start_ms,
                audio_offset_ms: None,
            },
        }
    }

    fn media(duration_ms: u64) -> MediaFacts {
        MediaFacts {
            duration_ms,
            decoded_video_frames: 3,
            has_audio: false,
            width: None,
            height: None,
            codec_name: None,
            avg_frame_rate: None,
            r_frame_rate: None,
        }
    }

    #[test]
    fn a_gap_holds_the_earlier_picture_and_the_total_is_the_capture_clock() {
        // Shaped like the still-screen run: a late first frame, restart gaps.
        let segments = [
            checkpoint(0, 120, 15_013),
            checkpoint(1, 16_900, 30_920),
            checkpoint(2, 31_759, 46_760),
        ];
        let plans =
            plan_segments(&segments, &[media(14_283), media(13_900), media(14_720)]).unwrap();
        assert_eq!(plans[0].lead_ms, 120);
        assert_eq!(plans[0].hold_ms, 16_900 - 15_013);
        assert_eq!(plans[1].lead_ms, 0);
        assert_eq!(plans[1].hold_ms, 31_759 - 30_920);
        assert_eq!(plans[2].hold_ms, 0);
        let total: u64 = plans.iter().map(SegmentPlan::target_ms).sum();
        assert_eq!(total, 46_760);
    }

    #[test]
    fn overlap_and_overrun_are_refused() {
        let overlap = [checkpoint(0, 0, 1_000), checkpoint(1, 900, 2_000)];
        assert!(plan_segments(&overlap, &[media(900), media(1_000)]).is_err());
        let overrun = [checkpoint(0, 0, 1_000)];
        assert!(plan_segments(&overrun, &[media(1_201)]).is_err());
        assert_eq!(
            plan_segments(&overrun, &[media(1_150)]).unwrap()[0].span_ms,
            1_150
        );
    }
}

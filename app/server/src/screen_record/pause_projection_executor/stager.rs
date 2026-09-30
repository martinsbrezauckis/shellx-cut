//! Private exact-CFR source staging for a completed pause projection.
//!
//! Inputs are copied into random private snapshots after their sealed hash is
//! rechecked. FFmpeg never receives the legacy root output path: the executor
//! supplies a private no-replace stage and probes it before publication.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use cut_core::{error_codes, CutError};
use cut_media::ffmpeg::{run_owned_command, OwnedProcessControl};
use record_recovery::{FrameGridStitchPlan, RunAwareStitchSpan};

use super::artifacts::snapshot_sources;
use super::types::VerifiedPauseProjectionSource;

const STAGE_TIMEOUT: Duration = Duration::from_secs(300);

/// The private seam accepted by the strict pause executor. A real native owner
/// may supply a different implementation only if it preserves this same
/// source-snapshot and independently probed exact-grid contract.
pub(crate) trait FrameGridPauseProjectionSourceStager {
    fn stage_frame_grid(
        &mut self,
        capture_dir: &Path,
        sources: &[VerifiedPauseProjectionSource],
        grid: &FrameGridStitchPlan,
        output: &Path,
    ) -> Result<(), CutError>;
}

/// FFmpeg implementation of the private frame-grid source stager.
pub(crate) struct FfmpegFrameGridPauseProjectionSourceStager {
    ffmpeg: String,
    ffprobe: String,
}

impl FfmpegFrameGridPauseProjectionSourceStager {
    pub(crate) fn new(ffmpeg: impl Into<String>, ffprobe: impl Into<String>) -> Self {
        Self {
            ffmpeg: ffmpeg.into(),
            ffprobe: ffprobe.into(),
        }
    }
}

impl FrameGridPauseProjectionSourceStager for FfmpegFrameGridPauseProjectionSourceStager {
    fn stage_frame_grid(
        &mut self,
        capture_dir: &Path,
        sources: &[VerifiedPauseProjectionSource],
        grid: &FrameGridStitchPlan,
        output: &Path,
    ) -> Result<(), CutError> {
        let control = OwnedProcessControl::bounded(STAGE_TIMEOUT, || false);
        let snapshots = snapshot_sources(capture_dir, sources)?;
        for (snapshot, source) in snapshots.iter().zip(sources) {
            super::timestamps::verify(&self.ffprobe, snapshot.path(), source, &control)?;
        }
        let filter = filter_graph(sources, grid)?;
        let mut command = Command::new(&self.ffmpeg);
        command.args(["-v", "error", "-nostdin", "-n"]);
        for snapshot in &snapshots {
            command
                .args([
                    "-protocol_whitelist",
                    cut_media::ffmpeg::LOCAL_INPUT_PROTOCOLS,
                    "-format_whitelist",
                    cut_media::ffmpeg::LOCAL_INPUT_FORMATS,
                    "-i",
                ])
                .arg(snapshot.path());
        }
        let rate = format!(
            "{}/{}",
            grid.output_frame_rate.num, grid.output_frame_rate.den
        );
        command
            .args(["-filter_complex", &filter, "-map", "[source]"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .args(["-r", &rate, "-map_metadata", "-1"])
            .arg(output);
        let result = run_owned_command(
            &mut command,
            &control,
            "assemble exact pause-session source stage",
        )
        .map_err(stage_error)?;
        if !result.status.success() {
            return Err(invalid(format!(
                "FFmpeg could not assemble the exact pause-session source: {}",
                String::from_utf8_lossy(&result.stderr)
                    .chars()
                    .take(512)
                    .collect::<String>()
            )));
        }
        Ok(())
    }
}

fn filter_graph(
    sources: &[VerifiedPauseProjectionSource],
    grid: &FrameGridStitchPlan,
) -> Result<String, CutError> {
    if sources.is_empty() || grid.spans.is_empty() || grid.output_frame_count == 0 {
        return Err(invalid(
            "frame-grid source stage has no sealed video inputs",
        ));
    }
    validate_grid_frames(grid)?;
    validate_padding_topology(grid)?;
    let mut filters = Vec::with_capacity(sources.len() + 1);
    let mut source_index = 0_usize;
    let mut assembled_frames = 0_u64;
    for (grid_index, grid_span) in grid.spans.iter().enumerate() {
        let RunAwareStitchSpan::Source {
            run_sequence,
            checkpoint_sequence,
            ..
        } = &grid_span.span
        else {
            continue;
        };
        let source = sources
            .get(source_index)
            .ok_or_else(|| invalid("frame-grid plan has more source spans than verified inputs"))?;
        if source.run_sequence() != *run_sequence
            || source.checkpoint_sequence() != *checkpoint_sequence
            || source.decoded_video_frames() == 0
            || source.avg_frame_rate().is_none()
            || source.r_frame_rate().is_none()
        {
            return Err(invalid(
                "verified source identity or media facts drift from the frame-grid plan",
            ));
        }
        // An interior encoder gap is owned by the preceding source only. The
        // first source may own an explicit leading gap, and the last source may
        // own a trailing one. Giving an interior gap to both neighbors would
        // duplicate frames and let the final trim discard real later content.
        let leading = leading_padding_frames(grid, grid_index, *run_sequence)?;
        let trailing = trailing_padding_frames(grid, grid_index, *run_sequence)?;
        let total = leading
            .checked_add(grid_span.frame_count())
            .and_then(|frames| frames.checked_add(trailing))
            .ok_or_else(|| invalid("frame-grid source staging frame count overflowed"))?;
        assembled_frames = assembled_frames
            .checked_add(total)
            .ok_or_else(|| invalid("frame-grid source staging frame count overflowed"))?;
        let rate = format!(
            "{}/{}",
            grid.output_frame_rate.num, grid.output_frame_rate.den
        );
        let source_frames = grid_span.frame_count();
        // Resample the verified presentation clock first. Hold its final frame
        // only to the quantized source endpoint; journal-declared encoder gaps
        // then add their separate, exact frame counts. Reset PTS to the output
        // grid so an input's nominal rate cannot retime explicit padding.
        filters.push(format!(
            "[{source_index}:v]setpts=PTS-STARTPTS,fps=fps={rate}:round=near,tpad=stop_mode=clone:stop=-1,trim=end_frame={source_frames},setpts=N/({rate}*TB),tpad=start_mode=clone:start={leading}:stop_mode=clone:stop={trailing},trim=end_frame={total},setpts=N/({rate}*TB)[s{source_index}]"
        ));
        source_index += 1;
    }
    if source_index != sources.len() {
        return Err(invalid(
            "verified source inputs contain an artifact absent from the frame-grid plan",
        ));
    }
    if assembled_frames != grid.output_frame_count {
        return Err(invalid(
            "frame-grid padding ownership does not account for the exact output frame count",
        ));
    }
    let labels = (0..source_index)
        .map(|index| format!("[s{index}]"))
        .collect::<String>();
    let rate = format!(
        "{}/{}",
        grid.output_frame_rate.num, grid.output_frame_rate.den
    );
    filters.push(format!(
        "{labels}concat=n={source_index}:v=1:a=0,fps=fps={rate}:round=near,trim=end_frame={},setpts=PTS-STARTPTS[source]",
        grid.output_frame_count
    ));
    Ok(filters.join(";"))
}

fn leading_padding_frames(
    grid: &FrameGridStitchPlan,
    source_index: usize,
    source_run: u64,
) -> Result<u64, CutError> {
    if source_index != 1 {
        return Ok(0);
    }
    padding_frames(grid.spans.first(), source_run)
}

fn trailing_padding_frames(
    grid: &FrameGridStitchPlan,
    source_index: usize,
    source_run: u64,
) -> Result<u64, CutError> {
    padding_frames(grid.spans.get(source_index + 1), source_run)
}

fn padding_frames(
    adjacent: Option<&record_recovery::FrameGridStitchSpan>,
    source_run: u64,
) -> Result<u64, CutError> {
    let Some(adjacent) = adjacent else {
        return Ok(0);
    };
    let RunAwareStitchSpan::EncoderGapPadding { run_sequence, .. } = &adjacent.span else {
        return Ok(0);
    };
    if *run_sequence != source_run {
        return Err(invalid(
            "frame-grid encoder padding crosses a sealed recording-run boundary",
        ));
    }
    Ok(adjacent.frame_count())
}

fn validate_padding_topology(grid: &FrameGridStitchPlan) -> Result<(), CutError> {
    for (index, span) in grid.spans.iter().enumerate() {
        let RunAwareStitchSpan::EncoderGapPadding { run_sequence, .. } = &span.span else {
            continue;
        };
        let owner = if index == 0 {
            grid.spans.get(index + 1)
        } else {
            grid.spans.get(index - 1)
        };
        let Some(owner) = owner else {
            return Err(invalid(
                "frame-grid encoder padding has no source frame to clone",
            ));
        };
        let RunAwareStitchSpan::Source {
            run_sequence: owner_run,
            ..
        } = &owner.span
        else {
            return Err(invalid(
                "frame-grid encoder padding is not adjacent to exactly one source owner",
            ));
        };
        if owner_run != run_sequence {
            return Err(invalid(
                "frame-grid encoder padding crosses a sealed recording-run boundary",
            ));
        }
    }
    Ok(())
}

fn validate_grid_frames(grid: &FrameGridStitchPlan) -> Result<(), CutError> {
    let mut cursor = 0_u64;
    for span in &grid.spans {
        if span.start_frame != cursor || span.end_frame < span.start_frame {
            return Err(invalid(
                "frame-grid source stage has non-contiguous or inverted frame spans",
            ));
        }
        if matches!(span.span, RunAwareStitchSpan::Source { .. })
            && span.end_frame == span.start_frame
        {
            return Err(invalid(
                "frame-grid source stage would remove a sealed video source",
            ));
        }
        cursor = span.end_frame;
    }
    if cursor != grid.output_frame_count {
        return Err(invalid(
            "frame-grid source stage does not end at its declared output frame count",
        ));
    }
    Ok(())
}

fn stage_error(error: CutError) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "cannot execute pause-session legacy projection",
        format!("assemble exact pause-session source stage: {error}"),
    )
}

fn invalid(detail: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "cannot execute pause-session legacy projection",
        detail.into(),
    )
    .with_suggested_action(
        "retain sealed fragments and retry only with the same exact qualified frame-grid inputs",
    )
}

#[cfg(test)]
#[path = "stager_tests.rs"]
mod tests;

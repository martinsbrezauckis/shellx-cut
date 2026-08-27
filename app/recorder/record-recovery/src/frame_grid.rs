//! Exact frame-grid policy for a future pause-aware source assembler.
//!
//! This module does not claim media output exists. It turns one already-valid
//! compact logical stitch plan into an expected CFR output grid. The eventual
//! assembler must separately prove its emitted media rate and frame count.

use record_core::FrameRate;

use crate::{RunAwareStitchPlan, RunAwareStitchSpan};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FrameGridError {
    #[error("frame-grid source plan is empty or has a zero duration")]
    Empty,
    #[error("frame-grid source plan has a logical gap, overlap, or malformed span")]
    MalformedPlan,
    #[error("frame-grid arithmetic overflowed its exact integer domain")]
    Overflow,
    #[error("frame-grid quantization would remove a sealed video source")]
    SourceLost,
}

/// A positive exact duration expressed in milliseconds as `numerator / denominator`.
/// It is an expected output-policy fact, not a probe claim about media already
/// written to disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExactFrameGridDuration {
    pub numerator: u64,
    pub denominator: u64,
}

/// One original source/padding span on the selected output frame grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameGridStitchSpan {
    pub span: RunAwareStitchSpan,
    pub start_frame: u64,
    pub end_frame: u64,
}

impl FrameGridStitchSpan {
    pub fn frame_count(&self) -> u64 {
        self.end_frame - self.start_frame
    }
}

/// A deterministic CFR output policy derived from compact session time.
///
/// Boundaries are quantized from their absolute compact timestamps, never by
/// summing individually-rounded source or padding durations. Therefore every
/// adjacent span shares exactly one frame boundary and repeated sub-frame gaps
/// cannot accumulate rounding error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameGridStitchPlan {
    pub output_frame_rate: FrameRate,
    pub output_frame_count: u64,
    pub expected_duration_ms: ExactFrameGridDuration,
    pub spans: Vec<FrameGridStitchSpan>,
}

/// Quantize a compact logical stitch plan onto one exact CFR output policy.
///
/// Ties round upward. A zero-frame padding span is intentional: it represents
/// a measured sub-frame encoder gap that cannot become media without creating a
/// false extra frame. A source span may never quantize to zero frames.
pub fn quantize_run_aware_stitch(
    plan: &RunAwareStitchPlan,
    output_frame_rate: FrameRate,
) -> Result<FrameGridStitchPlan, FrameGridError> {
    if plan.duration_ms == 0 || plan.spans.is_empty() {
        return Err(FrameGridError::Empty);
    }
    let mut logical_cursor = 0_u64;
    let mut frame_cursor = 0_u64;
    let mut spans = Vec::with_capacity(plan.spans.len());
    for span in &plan.spans {
        let (start_ms, end_ms, is_source) = span_bounds(span)?;
        if start_ms != logical_cursor {
            return Err(FrameGridError::MalformedPlan);
        }
        let end_frame = rounded_frame_boundary(end_ms, output_frame_rate)?;
        if end_frame < frame_cursor {
            return Err(FrameGridError::MalformedPlan);
        }
        if is_source && end_frame == frame_cursor {
            return Err(FrameGridError::SourceLost);
        }
        spans.push(FrameGridStitchSpan {
            span: span.clone(),
            start_frame: frame_cursor,
            end_frame,
        });
        logical_cursor = end_ms;
        frame_cursor = end_frame;
    }
    if logical_cursor != plan.duration_ms || frame_cursor == 0 {
        return Err(FrameGridError::MalformedPlan);
    }
    Ok(FrameGridStitchPlan {
        output_frame_rate,
        output_frame_count: frame_cursor,
        expected_duration_ms: exact_duration_ms(frame_cursor, output_frame_rate)?,
        spans,
    })
}

fn span_bounds(span: &RunAwareStitchSpan) -> Result<(u64, u64, bool), FrameGridError> {
    match span {
        RunAwareStitchSpan::Source {
            logical_offset_ms,
            source_duration_ms,
            ..
        } => {
            let end = logical_offset_ms
                .checked_add(*source_duration_ms)
                .ok_or(FrameGridError::Overflow)?;
            if end <= *logical_offset_ms {
                return Err(FrameGridError::MalformedPlan);
            }
            Ok((*logical_offset_ms, end, true))
        }
        RunAwareStitchSpan::EncoderGapPadding {
            logical_start_ms,
            logical_end_ms,
            ..
        } if logical_end_ms > logical_start_ms => Ok((*logical_start_ms, *logical_end_ms, false)),
        RunAwareStitchSpan::EncoderGapPadding { .. } => Err(FrameGridError::MalformedPlan),
    }
}

fn rounded_frame_boundary(ms: u64, rate: FrameRate) -> Result<u64, FrameGridError> {
    let numerator = u128::from(ms)
        .checked_mul(u128::from(rate.num))
        .ok_or(FrameGridError::Overflow)?;
    let denominator = u128::from(rate.den)
        .checked_mul(1_000)
        .ok_or(FrameGridError::Overflow)?;
    let rounded = numerator
        .checked_add(denominator / 2)
        .ok_or(FrameGridError::Overflow)?
        / denominator;
    u64::try_from(rounded).map_err(|_| FrameGridError::Overflow)
}

fn exact_duration_ms(
    frame_count: u64,
    rate: FrameRate,
) -> Result<ExactFrameGridDuration, FrameGridError> {
    let numerator = u128::from(frame_count)
        .checked_mul(u128::from(rate.den))
        .and_then(|value| value.checked_mul(1_000))
        .ok_or(FrameGridError::Overflow)?;
    let denominator = u128::from(rate.num);
    let divisor = gcd(numerator, denominator);
    Ok(ExactFrameGridDuration {
        numerator: u64::try_from(numerator / divisor).map_err(|_| FrameGridError::Overflow)?,
        denominator: u64::try_from(denominator / divisor).map_err(|_| FrameGridError::Overflow)?,
    })
}

fn gcd(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

#[cfg(test)]
#[path = "frame_grid_tests.rs"]
mod tests;

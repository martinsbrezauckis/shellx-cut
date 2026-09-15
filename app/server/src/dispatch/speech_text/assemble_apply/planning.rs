//! Validation and source-span planning for a reviewed Assemble proposal.

use super::super::*;
use super::binding::{aspect_label, aspect_parts, Materialization};

#[derive(Clone, Debug)]
pub(super) struct SourceSpan {
    pub(super) word_range: [usize; 2],
    pub(super) source_range: [u64; 2],
}
pub(super) fn source_spans(
    words: &[cut_perception::WordSpan],
    selected_ranges: &[[usize; 2]],
    ignores: &[cut_core::TranscriptIgnore],
    asset: &str,
    source_duration: Option<u64>,
) -> Result<Vec<SourceSpan>, CutError> {
    let pad = cut_perception::CUT_ON_WORD_TOLERANCE_MS;
    let mut spans = Vec::new();
    for &[lo, hi] in selected_ranges {
        if lo > hi || hi >= words.len() {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("word_range [{lo},{hi}] is no longer valid for asset '{asset}'"),
                format!("the current transcript has {} words", words.len()),
            )
            .with_suggested_action("review the Assemble plan again before applying it"));
        }
        for [span_lo, span_hi] in split_word_range_by_ignores(ignores, asset, [lo, hi]) {
            let start = words[span_lo].start_ms.saturating_sub(pad);
            let mut end = words[span_hi].end_ms.saturating_add(pad);
            if let Some(duration) = source_duration {
                end = end.min(duration);
            }
            if start >= end {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("reviewed transcript range [{span_lo},{span_hi}] has no insertable media span"),
                    "the source timing no longer provides a non-empty range",
                )
                .with_suggested_action("review the Assemble plan again before applying it"));
            }
            spans.push(SourceSpan {
                word_range: [span_lo, span_hi],
                source_range: [start, end],
            });
        }
    }
    if spans.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "every reviewed transcript range is ignored",
            "unignore a matched transcript range or review a different Assemble plan",
        ));
    }
    Ok(spans)
}

pub(super) fn destination_track(
    project: &cut_core::Project,
    kind: cut_core::TrackKind,
    label: &str,
) -> Result<String, CutError> {
    let track = project
        .tracks
        .iter()
        .find(|track| track.kind == kind)
        .ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("no {label} track is available for Assemble"),
                "create the required timeline track before applying this plan",
            )
        })?;
    if track.locked {
        return Err(locked_track_error(&track.id));
    }
    Ok(track.id.clone())
}

pub(super) fn locked_track_error(track: &str) -> CutError {
    CutError::new(
        error_codes::CONFLICT,
        format!("track '{track}' is locked"),
        "unlock the destination track before applying this Assemble plan",
    )
}

pub(super) fn ensure_short_geometry(
    project: &cut_core::Project,
    materialization: &Materialization,
) -> Result<(), CutError> {
    let Materialization::Shorts { aspect, .. } = materialization else {
        return Ok(());
    };
    let [required_width, required_height] = aspect_parts(aspect).expect("validated by planner");
    let settings = &project.settings;
    if u64::from(settings.width) * u64::from(required_height)
        != u64::from(settings.height) * u64::from(required_width)
    {
        return Err(CutError::new(
            error_codes::CONFLICT,
            format!(
                "project aspect {} does not match the reviewed short aspect {aspect}",
                aspect_label(settings.width, settings.height)
            ),
            "change the project format to the reviewed short aspect, then plan again",
        ));
    }
    Ok(())
}

pub(super) fn crop_pixels(source: &cut_core::Asset, crop: [f64; 4]) -> Result<[u32; 4], CutError> {
    let probe = source.probe.as_ref().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "the reviewed short source has no frame geometry",
            "run media.probe, then review the short plan again before applying it",
        )
    })?;
    let dimension = |name: &str| {
        probe
            .get(name)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0)
    };
    let (width, height) = match (dimension("width"), dimension("height")) {
        (Some(width), Some(height)) => (width, height),
        _ => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "the reviewed short source has invalid frame geometry",
                "run media.probe, then review the short plan again before applying it",
            ));
        }
    };
    let quantize = |fraction: f64, extent: u32| -> Result<u32, CutError> {
        if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "the reviewed crop is not a valid source fraction",
                "review the short plan again before applying it",
            ));
        }
        Ok((fraction * f64::from(extent)).round() as u32)
    };
    let mut x = quantize(crop[0], width)?;
    let mut y = quantize(crop[1], height)?;
    let w = quantize(crop[2], width)?.clamp(1, width);
    let h = quantize(crop[3], height)?.clamp(1, height);
    x = x.min(width - w);
    y = y.min(height - h);
    Ok([x, y, w, h])
}

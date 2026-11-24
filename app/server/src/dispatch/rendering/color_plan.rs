//! The colour handling of a render, decided before any work.
//!
//! Default exports are SDR BT.709 processed in 8 bits (HDR sources are
//! tone-mapped with zscale). Keep HDR exports target 10-bit BT.2020 (HLG or PQ)
//! and verify HDR clip presence, zscale, and 10-bit encoder support up front.

use cut_core::{ColorReport, CutError};
use cut_media::render::RenderColorMode;

/// The colour report for this render, or the refusal when a tone map / Keep HDR
/// requirement is missing before any work runs.
pub(super) fn plan_color_for(
    project: &cut_core::Project,
    edl: &cut_core::Edl,
    video_args: &[String],
    mode: RenderColorMode,
) -> Result<ColorReport, CutError> {
    if mode == RenderColorMode::KeepHdr {
        let encoder = cut_media::render::resources::video_encoder(video_args).unwrap_or("");
        cut_media::render::hdr::require_support_for_mode(project, edl, mode, encoder)?;
        return Ok(cut_media::render::source_color::color_report_with_mode(
            project, edl, video_args, mode,
        ));
    }
    let report = cut_media::render::source_color::color_report(project, edl, video_args);
    if cut_media::render::source_color::needs_zscale(&report)
        && !cut_media::hwencode::zscale_available()
    {
        return Err(missing_zscale(&report));
    }
    Ok(report)
}

/// The refusal: which clips need the conversion, what is missing and both ways
/// forward. The same words as the verb gate and the media layer use
/// (`tone_map_support::missing_tone_map_error`).
fn missing_zscale(report: &ColorReport) -> CutError {
    let files: Vec<String> = report
        .sources
        .iter()
        .filter(|s| s.handling == cut_media::render::source_color::HANDLING_TONE_MAPPED)
        .map(|s| s.file.clone())
        .collect();
    cut_media::render::tone_map_support::missing_tone_map_error(&files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refusal_names_the_clip_the_tool_and_both_ways_forward() {
        let report = ColorReport {
            output: "SDR BT.709".into(),
            transfer: None,
            processing_bit_depth: 8,
            file_bit_depth: 8,
            hdr10_metadata: None,
            sources: vec![cut_core::SourceColorReport {
                asset: "a1".into(),
                file: "hlg.mp4".into(),
                transfer: Some("arib-std-b67".into()),
                bit_depth: Some(10),
                handling: "tone_mapped_to_sdr".into(),
            }],
            note: None,
        };
        let e = missing_zscale(&report);
        assert_eq!(e.code, cut_core::error_codes::TOOL_MISSING);
        assert!(e.message.contains("(hlg.mp4)"), "{}", e.message);
        assert!(e.message.contains("zscale"), "{}", e.message);
        assert!(e.cause.contains("edit.color_space"), "{}", e.cause);
        assert!(e.suggested_action.unwrap().contains("zimg"));
    }
}

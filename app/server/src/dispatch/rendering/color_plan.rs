//! The colour handling of a render, decided before any work.
//!
//! Every export is SDR BT.709 processed in 8 bits; an HDR (HLG/PQ) source is
//! tone-mapped with ffmpeg's zscale filter. When the ffmpeg Cut can use has no
//! zscale, the render is refused up front with what is needed, instead of
//! failing mid-encode or writing a wrong file.

use cut_core::{ColorReport, CutError};

/// The colour report for this render, or the refusal when a tone map is needed
/// and no zscale-capable ffmpeg exists.
pub(super) fn plan_color(
    project: &cut_core::Project,
    edl: &cut_core::Edl,
    video_args: &[String],
) -> Result<ColorReport, CutError> {
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
            processing_bit_depth: 8,
            file_bit_depth: 8,
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

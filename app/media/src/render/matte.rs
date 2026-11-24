//! Renderer-side composition for `edit.matte` replacement backgrounds.
//!
//! Matting inference produces only a cached gray alpha stream.  This module owns
//! the deterministic software-FFmpeg step that combines that alpha with the
//! subject and a declared replacement background.  `remove` deliberately stays
//! on the existing overlay-alpha path: it reveals the track beneath the clip.

use super::{conform_filter_for_asset, fps_filter_for_asset, secs, Fit};
use cut_core::{error_codes, ClipMatte, CutError, MatteBg, MatteMode, Project};
use std::collections::BTreeMap;

/// A concrete background source for a `mode=replace` matte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReplacementBackground<'a> {
    Color(&'a str),
    Asset(&'a str),
}

/// `color=` is part of an FFmpeg filtergraph, so only pass a color token that
/// cannot introduce another filter. The core's chroma-key allowlist covers names
/// and `0xRRGGBB[AA]`; the human Matte panel also sends `#RRGGBB`.
fn is_safe_color(color: &str) -> bool {
    cut_core::is_valid_chroma_color(color)
        || (color.len() == 7
            && color.starts_with('#')
            && color.as_bytes()[1..]
                .iter()
                .all(|byte| byte.is_ascii_hexdigit()))
}

/// Return the requested replace background.  `bg` is optional at the wire level
/// because it is irrelevant to `mode=remove`; it is required once replace reaches
/// the renderer so we never silently render replace as remove.
pub(super) fn replacement_background(
    matte: Option<&ClipMatte>,
) -> Result<Option<ReplacementBackground<'_>>, CutError> {
    let Some(matte) = matte else {
        return Ok(None);
    };
    if matte.mode == MatteMode::Remove {
        return Ok(None);
    }
    match matte.bg.as_ref() {
        Some(MatteBg::Color { color }) if is_safe_color(color) => {
            Ok(Some(ReplacementBackground::Color(color)))
        }
        Some(MatteBg::Color { color }) => Err(CutError::new(
            cut_core::error_codes::INVALID_ARGS,
            format!("invalid matte replacement color '{color}'"),
            "use a color name, 0xRRGGBB[AA], or the Matte panel's #RRGGBB form",
        )),
        Some(MatteBg::Asset { asset }) => Ok(Some(ReplacementBackground::Asset(asset))),
        None => Err(CutError::new(
            cut_core::error_codes::INVALID_ARGS,
            "matte replace needs a background",
            "pass bg:{type:'color', color:'…'} or bg:{type:'asset', asset:'…'} when mode is replace",
        )),
    }
}

/// Build the source side of a replacement background. Video backgrounds begin
/// at their own first frame and hold their final frame when the matted segment is
/// longer; stills are already looped by `graph_args`. Both are conformed before
/// the caller applies the subject's placement/fade geometry.
#[allow(clippy::too_many_arguments)]
pub(super) fn background_filter_for(
    project: &Project,
    input_idx: &BTreeMap<String, usize>,
    background: ReplacementBackground<'_>,
    w: u32,
    h: u32,
    fps: &str,
    fit: Fit,
    dur_ms: u64,
    hdr_target: Option<super::source_color::Hdr>,
) -> Result<String, CutError> {
    let dur = secs(dur_ms);
    let afmt = if hdr_target.is_some() {
        "yuva420p10le"
    } else {
        "yuva420p"
    };
    match background {
        ReplacementBackground::Color(color) => {
            let hdr_conv = hdr_target
                .map(|t| super::hdr::sdr_to_hdr_filter(t, None, false))
                .unwrap_or_default();
            Ok(format!(
                "color=c={color}:s={w}x{h}:r={fps}:d={dur}{hdr_conv},format={afmt}"
            ))
        }
        ReplacementBackground::Asset(asset_id) => {
            let idx = input_idx.get(asset_id).copied().ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("matte background asset {asset_id} is not available to render"),
                    "the declared replace background must remain an imported project asset",
                )
            })?;
            let asset = project.assets.get(asset_id).ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("matte background asset {asset_id} does not exist"),
                    "import the replacement image/video before rendering the matted clip",
                )
            })?;
            let conform = conform_filter_for_asset(asset, None, w, h, fit);
            let bg_fps = fps_filter_for_asset(asset, project.settings.fps, 1.0, false);
            let hdr_conv = if let Some(target) = hdr_target {
                let c = super::source_color::source_color(asset);
                if c.hdr.is_some() {
                    super::hdr::hdr_to_hdr_filter(&c, target)
                } else {
                    let b601 = super::source_prep::bt601_to_bt709_filter(&c);
                    let to_hdr = super::hdr::sdr_to_hdr_filter(target, c.range.as_deref(), false);
                    format!("{b601}{to_hdr}")
                }
            } else {
                String::new()
            };
            Ok(format!(
                "[{idx}:v]trim=start=0:end={dur},setpts=PTS-STARTPTS,\
                 tpad=stop_mode=clone:stop_duration={dur},{conform},{bg_fps}{hdr_conv},format={afmt}"
            ))
        }
    }
}

/// Join a subject with its cached alpha over a prepared replacement background.
/// `background_filter` is a complete chain head (for example a `color` source or
/// a trimmed imported asset). The caller applies placement, opacity and fades to
/// the completed replacement stream, once, after this composite is opaque.
pub(super) fn replace_block(
    foreground: &str,
    alpha: &str,
    subject: &str,
    background_filter: &str,
    background: &str,
    output: &str,
    output_format: &str,
    output_suffix: &str,
) -> String {
    let subject_fmt = if output_format.contains("10") {
        ",format=yuva420p10le"
    } else {
        ""
    };
    format!(
        "[{foreground}][{alpha}]alphamerge{subject_fmt}[{subject}];\n\
         {background_filter}[{background}];\n\
         [{background}][{subject}]overlay=shortest=1:format=auto,format={output_format}{output_suffix}[{output}];"
    )
}

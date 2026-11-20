//! What a REBUILT editing proxy must carry, the same as the first one.
//!
//! The import's proxy job tone-maps an HDR source so the preview matches the
//! export, and records how much of the source really decodes
//! (`probe.decode_check`, see `dispatch/media/decode_check.rs`). The cache
//! rebuild used to skip both: a rebuilt HDR proxy was 8-bit video with HDR
//! tags, and a project whose probe file was lost came back without its
//! "File cut off" warning. This module gives the rebuild the same two steps.

use serde_json::Value;
use std::path::Path;

/// The proxy tone map for a source probe, and a warning when the source is
/// HDR but no installed ffmpeg can tone-map it (the proxy is then made
/// unconverted, as the import's proxy job does).
pub(super) fn tone_map_for(
    probe: &Value,
    zscale_available: impl FnOnce() -> bool,
) -> (String, Option<String>) {
    let color = cut_media::render::source_color::source_color_from_probe(probe);
    if color.hdr.is_none() {
        return (String::new(), None);
    }
    if zscale_available() {
        (
            cut_media::render::source_color::hdr_to_sdr_filter(&color),
            None,
        )
    } else {
        (
            String::new(),
            Some(
                "This HDR clip's rebuilt proxy previews without conversion to SDR because the \
                 installed FFmpeg has no zscale filter; choose an FFmpeg with zimg in Settings > \
                 Video processing."
                    .into(),
            ),
        )
    }
}

/// The decode-check record for a rebuilt proxy of a video source.
pub(super) fn decode_check_for(probe: &Value, proxy: &Path) -> Result<Value, cut_core::CutError> {
    let streams = cut_media::probe::stream_extents(proxy)?;
    Ok(cut_media::decode_check::decode_check(probe, &streams))
}

/// Keep `receipts/<id>.probe.json` (the copy a log rebuild restores the probe
/// from) in step with the probe that now carries the decode check.
/// Best-effort, like the import's own write.
pub(super) fn persist_probe(project_dir: &Path, asset_id: &str, probe: &Value) {
    if asset_id.is_empty()
        || asset_id.len() > 128
        || !asset_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return;
    }
    let Ok(receipts) =
        crate::output_paths::ensure_plain_project_relative_dir(project_dir, Path::new("receipts"))
    else {
        return;
    };
    if let Ok(text) = serde_json::to_string(probe) {
        let _ = crate::output_paths::write_output_atomic(
            &receipts.join(format!("{asset_id}.probe.json")),
            text,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_hdr_probe_gets_the_tone_map_or_a_plain_warning() {
        let hlg = json!({"kind": "video", "raw": {"streams": [{"codec_type": "video",
            "pix_fmt": "yuv420p10le", "color_transfer": "arib-std-b67",
            "color_primaries": "bt2020", "color_space": "bt2020nc"}]}});
        let (filter, warning) = tone_map_for(&hlg, || true);
        assert!(filter.starts_with(",zscale=tin=arib-std-b67"), "{filter}");
        assert!(warning.is_none());
        let (filter, warning) = tone_map_for(&hlg, || false);
        assert!(filter.is_empty());
        assert!(warning.unwrap().contains("zscale"));
        let sdr = json!({"kind": "video", "raw": {"streams": [{"codec_type": "video", "pix_fmt": "yuv420p"}]}});
        assert_eq!(
            tone_map_for(&sdr, || panic!("no probe for SDR")),
            (String::new(), None)
        );
    }
}

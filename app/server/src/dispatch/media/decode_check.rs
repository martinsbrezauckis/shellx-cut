//! Record how much of a video source really decodes, once its proxy exists,
//! and choose the proxy's HDR to SDR tone map before it is made.
//!
//! The proxy job decodes the whole source. Its proxy's stream lengths are the
//! decodable length; `cut_media::decode_check` compares them with what the
//! source claims. The record is stored on the asset as `probe.decode_check`
//! (an agent reads it from project.state / media.probe; the Assets card shows
//! its `warning`), and a warning also rides on the proxy job result.

use super::*;
use std::path::Path;

/// Measure `proxy`, store `probe.decode_check` on `asset_id`, and return the
/// plain warning when the source decodes shorter than it claims.
pub(super) async fn record_decode_check(
    state: &AppState,
    asset_id: &str,
    proxy: &Path,
) -> Result<Option<String>, CutError> {
    let proxy = proxy.to_path_buf();
    let streams = run_blocking("media.decode_check", move || {
        cut_media::probe::stream_extents(&proxy)
    })
    .await?;
    let source_probe = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        store
            .project
            .assets
            .get(asset_id)
            .and_then(|a| a.probe.clone())
            .unwrap_or(Value::Null)
    };
    if !source_probe.is_object() {
        return Ok(None);
    }
    let record = cut_media::decode_check::decode_check(&source_probe, &streams);
    let warning = record
        .get("warning")
        .and_then(Value::as_str)
        .map(String::from);
    let mut probe = source_probe;
    probe["decode_check"] = record;
    let stored = probe.clone();
    // The check adds an engine fact to the probe: it stays verified only if
    // the probe it extends was (a received probe stays unknown).
    let was_verified = super::probe_verified(state, asset_id).await;
    update_asset(state, asset_id, |a| a.probe = Some(stored)).await?;
    if was_verified {
        super::mark_probe_verified(state, asset_id).await;
    }
    persist_probe_file(state, asset_id, &probe).await;
    Ok(warning)
}

/// Keep an earlier decode check when a probe is refreshed (`media.probe`): the
/// new probe comes from the index and knows nothing about decoding.
pub(super) fn keep_decode_check(old: Option<&Value>, mut new: Value) -> Value {
    if let Some(check) = old.and_then(|p| p.get("decode_check")) {
        new["decode_check"] = check.clone();
    }
    new
}

/// The proxy's HDR -> SDR tone map for `asset_id`, or "" for an SDR source.
/// Without a zscale-capable ffmpeg the proxy is made unconverted and a warning
/// says the preview shows the HDR picture as-is.
pub(super) async fn proxy_tone_map(
    state: &AppState,
    asset_id: &str,
    warnings: &mut Vec<String>,
) -> String {
    let asset = {
        let guard = state.project.read().await;
        guard
            .as_ref()
            .and_then(|store| store.project.assets.get(asset_id).cloned())
    };
    let Some(asset) = asset else {
        return String::new();
    };
    let color = cut_media::render::source_color::source_color(&asset);
    if color.hdr.is_none() {
        return String::new();
    }
    let available = run_blocking("media.zscale_probe", || {
        Ok(cut_media::hwencode::zscale_available())
    })
    .await
    .unwrap_or(false);
    if available {
        cut_media::render::source_color::hdr_to_sdr_filter(&color)
    } else {
        warnings.push(
            "This HDR clip previews without conversion to SDR because the ffmpeg Cut uses has no \
             zscale filter; choose an ffmpeg that includes zscale in Settings > Video processing."
                .into(),
        );
        String::new()
    }
}

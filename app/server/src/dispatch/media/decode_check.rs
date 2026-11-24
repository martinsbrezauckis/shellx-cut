//! Record how much of a video source really decodes, once its proxy exists,
//! and choose the proxy's HDR to SDR tone map before it is made.
//!
//! The proxy comparison is cheap. Only suspected frame gaps are confirmed by
//! bounded source decoding, before proxy timestamp conversion can drop frames.
//! The record is stored as `probe.decode_check`
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
    let (source, _) = asset_info(state, asset_id).await?;
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
    let probe_for_check = source_probe.clone();
    let control = owned_job_process_control(crate::jobs::current_job_cancellation());
    let record = run_blocking("media.decode_check", move || {
        cut_media::ffmpeg::with_render_process_control(&control, || {
            cut_media::decode_check::check_source(&probe_for_check, &source, &proxy)
        })
    })
    .await?;
    let warning = record
        .get("warning")
        .and_then(Value::as_str)
        .map(String::from);
    let mut probe = source_probe;
    probe["decode_check"] = record;
    if let Some(obj) = probe.as_object_mut() {
        obj.remove("decode_check_error");
        obj.remove("proxy_error");
    }
    save_probe_preservation(state, asset_id, probe).await?;
    Ok(warning)
}

async fn save_probe_preservation(
    state: &AppState,
    asset_id: &str,
    probe: Value,
) -> Result<(), CutError> {
    let stored = probe.clone();
    // The check adds an engine fact to the probe: it stays verified only if
    // the probe it extends was (a received probe stays unknown).
    let was_verified = super::probe_verified(state, asset_id).await;
    update_asset(state, asset_id, |a| a.probe = Some(stored)).await?;
    if was_verified {
        super::mark_probe_verified(state, asset_id).await;
    }
    persist_probe_file(state, asset_id, &probe).await;
    Ok(())
}

/// Record a failed proxy decode check without fabricating `probe.decode_check`
/// so export-check diagnostics keep the real error reason.
pub(super) async fn record_decode_check_or_error(
    state: &AppState,
    asset_id: &str,
    proxy: &Path,
) -> Result<Option<String>, CutError> {
    match record_decode_check(state, asset_id, proxy).await {
        Ok(warning) => Ok(warning),
        Err(error) => {
            record_probe_check_error(state, asset_id, "decode_check_error", &error).await;
            Err(error)
        }
    }
}

/// Record why proxy preparation failed for `asset_id` so pregate never treats
/// a failed proxy transcode as an unrun decode check.
pub(super) async fn record_proxy_error(state: &AppState, asset_id: &str, error: &CutError) {
    record_probe_check_error(state, asset_id, "proxy_error", error).await;
}

async fn record_probe_check_error(state: &AppState, asset_id: &str, key: &str, error: &CutError) {
    let Some(mut probe) = super::asset_probe(state, asset_id).await else {
        return;
    };
    let Some(obj) = probe.as_object_mut() else {
        return;
    };
    obj.remove("decode_check");
    obj.insert(
        key.to_string(),
        serde_json::json!({
            "code": error.code,
            "message": error.message,
            "cause": error.cause,
            "suggested_action": error.suggested_action,
        }),
    );
    let _ = save_probe_preservation(state, asset_id, probe).await;
}

/// Keep an earlier decode check (or preserved failure) when a probe is
/// refreshed (`media.probe`): the new probe comes from the index and knows
/// nothing about decoding.
pub(super) fn keep_decode_check(old: Option<&Value>, mut new: Value) -> Value {
    if let Some(check) = old.and_then(|p| p.get("decode_check")) {
        new["decode_check"] = check.clone();
    }
    if let Some(err) = old.and_then(|p| p.get("decode_check_error")) {
        new["decode_check_error"] = err.clone();
    }
    if let Some(err) = old.and_then(|p| p.get("proxy_error")) {
        new["proxy_error"] = err.clone();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn failed_decode_check_preserves_error_and_successful_recheck_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new();
        let actor = Actor {
            kind: cut_core::ActorKind::Agent,
            name: "test".into(),
            via: "test".into(),
            request: None,
        };
        let created = crate::dispatch::dispatch(
            &state,
            "project.create",
            json!({"name": "dc", "dir": dir.path().join("dc.cutproj")}),
            actor,
        )
        .await;
        assert!(created.ok, "{:?}", created.error);
        let source = dir.path().join("src.mp4");
        std::fs::write(&source, b"source").unwrap();
        update_asset_or_insert(&state, &source).await;

        let missing_proxy = dir.path().join("missing-proxy.mp4");
        let err = record_decode_check_or_error(&state, "a1", &missing_proxy)
            .await
            .unwrap_err();
        assert!(!err.message.is_empty());
        let probe = super::super::asset_probe(&state, "a1").await.unwrap();
        assert!(probe.get("decode_check").is_none());
        assert_eq!(probe["decode_check_error"]["message"], json!(err.message));

        let kept = keep_decode_check(Some(&probe), json!({"kind": "video"}));
        assert_eq!(kept["decode_check_error"]["message"], json!(err.message));
    }

    async fn update_asset_or_insert(state: &AppState, source: &Path) {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        let asset: cut_core::Asset = serde_json::from_value(json!({
            "path": source.display().to_string(),
            "hash": "sha256:test",
            "probe": {"kind": "video", "duration_ms": 1000}
        }))
        .unwrap();
        store.project.assets.insert("a1".into(), asset);
        store.save().unwrap();
    }
}

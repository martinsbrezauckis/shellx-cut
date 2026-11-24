//! At a project open, re-read what the project says about its media.
//!
//! A received project's cache carries each asset's probe (kind, sound, size,
//! frame rate, decode check) and a timeline-strip pointer; they are whatever
//! the sender wrote. In the open's `proxy_check` job: a probe this engine
//! recorded for the unchanged file counts at once; every other source that is
//! still the imported file (same content hash) is probed again after the
//! proxies are given back, and the fresh probe replaces the cached one (cut_core
//! probe_facts). A missing or changed source keeps its cached probe for display
//! only, unknown to every decision. A strip this engine did not make is dropped
//! and drawn again. A person sees nothing.

use super::{
    asset_source_path, decode_check, mark_probe_verified, media_filmstrip, persist_probe_file,
    update_asset,
};
use crate::cache_lifecycle::SourceCheck;
use crate::state::AppState;
use std::path::{Path, PathBuf};

/// Fast pass at open: a probe this engine recorded for the file as it is now
/// (size and modification time, cache_trust) counts at once. Returns the asset
/// ids whose probe is still unknown (a received or older project, or a changed
/// file): [`reread_probes`] reads them from their files after the proxies.
pub(super) async fn recognise_probes(state: &AppState, dir: &Path) -> Vec<String> {
    let candidates: Vec<(String, PathBuf, Option<String>)> = {
        let guard = state.project.read().await;
        let Some(store) = guard.as_ref().filter(|store| store.dir == dir) else {
            return Vec::new();
        };
        store
            .project
            .assets
            .iter()
            .filter(|(_, asset)| asset.probe.is_some())
            .map(|(id, asset)| {
                (
                    id.clone(),
                    asset_source_path(dir, asset),
                    store.project.probe_binding(id),
                )
            })
            .collect()
    };
    let known = tokio::task::spawn_blocking(move || {
        candidates
            .into_iter()
            .map(|(id, source, binding)| {
                let known = binding
                    .is_some_and(|b| cut_media::cache_trust::source_unchanged_for(&source, &b));
                (id, known)
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    let mut unknown = Vec::new();
    let mut guard = state.project.write().await;
    let Some(store) = guard.as_mut().filter(|store| store.dir == dir) else {
        return Vec::new();
    };
    for (id, known) in known {
        if known {
            store.project.mark_probe_verified(&id);
        } else {
            unknown.push(id);
        }
    }
    unknown
}

/// Slow pass, after the proxies are given back: probe these assets again from
/// their files when each is still the imported file (same content hash); the
/// fresh probe replaces the cached one, counts as established and is recorded
/// for the next open. Returns the ids whose probe stays unknown (missing,
/// changed or unreadable file).
pub(super) async fn reread_probes(state: &AppState, dir: &Path, ids: &[String]) -> Vec<String> {
    let candidates: Vec<(String, PathBuf, String)> = {
        let guard = state.project.read().await;
        let Some(store) = guard.as_ref().filter(|store| store.dir == dir) else {
            return Vec::new();
        };
        ids.iter()
            .filter_map(|id| {
                let asset = store.project.assets.get(id)?;
                Some((
                    id.clone(),
                    asset_source_path(dir, asset),
                    asset.hash.clone(),
                ))
            })
            .collect()
    };
    let fresh = tokio::task::spawn_blocking(move || {
        candidates
            .into_iter()
            .map(|(id, source, hash)| {
                let probe = (source.is_file() && SourceCheck::verify(&source, &hash).matches)
                    .then(|| cut_media::probe_source(&source).ok())
                    .flatten()
                    .and_then(|probe| serde_json::to_value(&probe).ok());
                (id, probe)
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    let mut unknown = Vec::new();
    for (id, probe) in fresh {
        let Some(probe) = probe else {
            unknown.push(id);
            continue;
        };
        let stored = probe.clone();
        if update_asset(state, &id, |asset| asset.probe = Some(stored))
            .await
            .is_ok()
        {
            mark_probe_verified(state, &id).await;
            persist_probe_file(state, &id, &probe).await;
        }
    }
    unknown
}

/// After the proxies are given back: their decode checks are measured again
/// from the trusted proxy (the cached ones were dropped with the cached
/// probe), and every timeline strip dropped at the hold (one this engine did
/// not make) is drawn again where its source or proxy is ready.
pub(super) async fn recheck_derived(
    state: &AppState,
    dir: &Path,
    given_back: &[String],
    strips: &[String],
) {
    for id in given_back {
        let proxy = dir.join(format!("proxies/{id}.mp4"));
        if let Err(error) = decode_check::record_decode_check_or_error(state, id, &proxy).await {
            tracing::warn!(asset = %id, error = %error.message, "decode check not measured again");
        }
    }
    let still_open = state
        .project
        .read()
        .await
        .as_ref()
        .is_some_and(|store| store.dir == dir);
    for id in strips.iter().filter(|_| still_open) {
        // Drawn again from the source (images) or the given-back proxy; a clip
        // whose proxy is being made again gets its strip from that proxy job.
        if let Err(error) = media_filmstrip(state, serde_json::json!({ "asset": id })).await {
            tracing::info!(asset = %id, error = %error.message, "strip not drawn again yet");
        }
    }
}

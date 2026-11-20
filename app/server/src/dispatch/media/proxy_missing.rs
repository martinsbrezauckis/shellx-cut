//! What an opened project's footage lacks (proxy_trust.rs holds the proxies it
//! has): footage with no proxy at all (an import cancelled by a project switch
//! during its proxy, a failed proxy job) and imports stopped before their probe.
//! A person's `media.import {proxy:false}` is kept: no proxy is made for it.

use super::asset_source_path;
use crate::cache_lifecycle::SourceCheck;
use crate::state::AppState;
use cut_core::ProjectStore;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// (footage to give a proxy, imports to resume: (id, source, hash, make a proxy)).
pub(super) type Lacking = (Vec<String>, Vec<(String, PathBuf, String, bool)>);

/// What the opened project lacks, given the assets whose proxies are `held`.
pub(super) fn lacking(store: &ProjectStore, dir: &Path, held: &[(String, String)]) -> Lacking {
    let held: HashSet<&str> = held.iter().map(|(id, _)| id.as_str()).collect();
    let opted_out = proxy_opt_outs(store);
    let wants = |id: &str| opted_out.as_ref().is_some_and(|out| !out.contains(id));
    let mut missing = Vec::new();
    let mut unprobed = Vec::new();
    for (id, asset) in &store.project.assets {
        if crate::dispatch::edit_tools::is_drawn_overlay(dir, &asset.path) {
            continue; // a title or shape movie plays as drawn
        }
        let source = asset_source_path(dir, asset);
        if !source.is_file() {
            continue;
        }
        if asset.probe.is_none() {
            unprobed.push((id.clone(), source, asset.hash.clone(), wants(id)));
        } else if !held.contains(id.as_str()) && wants(id) && is_footage(asset) {
            missing.push(id.clone());
        }
    }
    (missing, unprobed)
}

/// Assets the person imported with `proxy:false`, or `None` when the history
/// cannot be read (then nothing is made: a choice is never guessed).
fn proxy_opt_outs(store: &ProjectStore) -> Option<HashSet<String>> {
    let ops = store.log.read_all().ok()?;
    Some(
        ops.iter()
            .filter(|op| {
                op.verb == "media.import" && op.args.get("proxy") == Some(&serde_json::json!(false))
            })
            .filter_map(|op| {
                op.effects
                    .iter()
                    .find_map(|e| e.detail.get("asset_id")?.as_str().map(str::to_string))
            })
            .collect(),
    )
}

/// A video asset with a picture: plays from a proxy.
fn is_footage(asset: &cut_core::Asset) -> bool {
    let probe = asset.probe.as_ref();
    let kind = probe.and_then(|p| p.get("kind")).and_then(|k| k.as_str());
    let has_video = probe
        .and_then(|p| p.get("has_video"))
        .and_then(|v| v.as_bool())
        != Some(false);
    kind == Some("video") && has_video
}

/// (id, source, recorded content hash) of these assets while `dir` is open.
pub(super) async fn sources(
    state: &AppState,
    dir: &Path,
    ids: &[String],
) -> Vec<(String, PathBuf, String)> {
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
}

/// Keep what still reads the file that was imported (same content hash, as
/// the cache rebuild requires): (verified proxy ids, imports to resume, ids
/// whose source is gone, changed or another file). Hashes; run it blocking.
pub(super) fn same_sources(
    proxies: Vec<(String, PathBuf, String)>,
    imports: Vec<(String, PathBuf, String, bool)>,
) -> (
    Vec<String>,
    Vec<(String, PathBuf, String, bool)>,
    Vec<String>,
) {
    let mut changed = Vec::new();
    let mut verified = Vec::new();
    for (id, source, hash) in proxies {
        if SourceCheck::verify(&source, &hash).matches {
            verified.push(id);
        } else {
            changed.push(id);
        }
    }
    let mut resumed = Vec::new();
    for import in imports {
        if SourceCheck::verify(&import.1, &import.2).matches {
            resumed.push(import);
        } else {
            changed.push(import.0);
        }
    }
    (verified, resumed, changed)
}

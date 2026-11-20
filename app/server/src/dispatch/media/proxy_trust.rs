//! Which proxy an opened project may use for its preview (item 171).
//!
//! A proxy is a derived file in the project's own `proxies/` folder that this
//! engine made. A project received from someone else can store any path as an
//! asset's proxy (cut_core drops one outside `proxies/` when the project
//! opens) and can carry any file under a proxy's name. The preview, the scrub
//! frame, the windowed filmstrip and the storyboard read the proxy and show or
//! hand on its frames, so a proxy is used only when `cut_media::cache_trust`
//! holds this engine's record of making it.
//!
//! Checking means hashing the proxy, which can take seconds for long footage,
//! so it is not done while the project opens: every proxy pointer is held back
//! when the project opens (the preview reads the original media meanwhile),
//! and a background check gives each one back once its record matches, or
//! makes the proxy again. A proxy file whose pointer was lost (for example
//! when the project was saved before the check finished) is found again under
//! its usual name. Nothing is lost for the person.

use super::{asset_source_path, project_paths, spawn_plain_import_chain, spawn_proxy_chain};
use crate::state::{AppState, PendingProxyCheck};
use cut_core::ProjectStore;
use std::path::{Path, PathBuf};

/// Proxies held back from an opening project until they are checked.
#[derive(Debug, Default)]
pub(crate) struct HeldProxies {
    dir: PathBuf,
    /// (asset id, proxy pointer relative to the project folder).
    assets: Vec<(String, String)>,
    /// Asset id -> recorded source content hash (what the proxy must be made from).
    hashes: std::collections::BTreeMap<String, String>,
    /// Footage with no proxy at all (an import cancelled by a project switch,
    /// a proxy job that failed or was cancelled): made at this open, unless the
    /// person imported it with `proxy:false`.
    missing: Vec<String>,
    /// Imports stopped before their probe (a project switch right after the
    /// import): (asset id, source, hash, make a proxy). Their import chain is
    /// started again (probe, then the proxy unless the person said no).
    unprobed: Vec<(String, PathBuf, String, bool)>,
    /// Assets whose strip pointer was dropped at the hold (a file this engine
    /// did not make): drawn again by the check.
    strips: Vec<String>,
}

/// Clear every proxy pointer of the opening project and remember what to check.
pub(crate) fn hold_proxies(store: &mut ProjectStore) -> HeldProxies {
    let dir = store.dir.clone();
    let mut assets = Vec::new();
    for (id, asset) in store.project.assets.iter_mut() {
        let pointer = asset.proxy.take().or_else(|| {
            let usual = format!("proxies/{id}.mp4");
            dir.join(&usual).is_file().then_some(usual)
        });
        if let Some(pointer) = pointer {
            assets.push((id.clone(), pointer));
        }
    }
    // A strip this engine did not make is never shown, not even before the
    // check (small images: hashed here, before the project is visible).
    let mut strips = Vec::new();
    for (id, asset) in store.project.assets.iter_mut() {
        if asset.filmstrip.as_deref().is_some_and(|strip| {
            let path = dir.join(strip);
            !(cut_core::matte_cache::plain_file_exists(&path).unwrap_or(false)
                && cut_media::cache_trust::trusted(&path))
        }) {
            asset.filmstrip = None;
            strips.push(id.clone());
        }
    }
    let hashes = store
        .project
        .assets
        .iter()
        .map(|(id, asset)| (id.clone(), asset.hash.clone()))
        .collect();
    let (missing, unprobed) = super::proxy_missing::lacking(store, &dir, &assets);
    HeldProxies {
        dir,
        assets,
        hashes,
        missing,
        unprobed,
        strips,
    }
}

/// What a proxy's engine-origin record is bound to: the asset and the source
/// content it was made from.
pub(crate) fn proxy_binding(asset_id: &str, source_hash: &str) -> String {
    format!("proxy\n{asset_id}\n{source_hash}")
}

/// Split the held proxies into those this engine made (unchanged since) and
/// those to make again. Hashes the files; run it off the async threads.
pub(crate) fn sort_held(held: &HeldProxies) -> (Vec<(String, String)>, Vec<String>) {
    let mut trusted = Vec::new();
    let mut remake = Vec::new();
    for (id, pointer) in &held.assets {
        // Only this asset's own name, a plain file with one name (no link to
        // another asset's proxy), made by this engine for this asset's source.
        let path = held.dir.join(pointer);
        let hash = held.hashes.get(id).map(String::as_str).unwrap_or("");
        if *pointer == format!("proxies/{id}.mp4")
            && cut_core::matte_cache::plain_file_exists(&path).unwrap_or(false)
            && cut_media::cache_trust::trusted_bound(&path, &proxy_binding(id, hash))
        {
            trusted.push((id.clone(), pointer.clone()));
        } else {
            tracing::info!(
                asset = %id,
                "proxy not made by this engine on this computer; the preview uses the original until it is made again"
            );
            remake.push(id.clone());
        }
    }
    (trusted, remake)
}

/// Check the held proxies in the background, give back the trusted ones and
/// make the others again (one at a time, like an import).
///
/// The check is a `proxy_check` job: while it runs, `jobs.list` shows it and
/// `project.state` lists the assets in `proxy_check`; when it ends, its
/// `job_progress` 1.0 is the one event every client re-reads the project on
/// (the UI's Assets panel and preview then use the proxies). One job and one
/// event per opened project, however many assets it has.
pub(crate) fn check_held_proxies(state: &AppState, held: HeldProxies) {
    // Every opened project with media: its probes are read again too.
    if held.hashes.is_empty() {
        return;
    }
    let job = state.jobs.create("proxy_check");
    let job_id = job.job_id.clone();
    let dir = held.dir.clone();
    if let Ok(mut pending) = state.proxy_check.lock() {
        *pending = Some(PendingProxyCheck {
            dir: dir.clone(),
            job_id: job_id.clone(),
            assets: held.assets.iter().map(|(id, _)| id.clone()).collect(),
        });
    }
    let count = held.assets.len();
    let missing = held.missing.clone();
    let unprobed = held.unprobed.clone();
    let strips = held.strips.clone();
    let jid = job_id.clone();
    let task_state = state.clone();
    state.jobs.spawn(&job_id, async move {
        let state = task_state;
        state.jobs.progress(
            &jid,
            0.05,
            Some(if missing.is_empty() {
                format!("checking {count} editing proxies made on this computer")
            } else {
                format!(
                    "checking {count} editing proxies; {} clips without one get one",
                    missing.len()
                )
            }),
        );
        let probes_unknown = super::probe_recheck::recognise_probes(&state, &dir).await;
        let checked = tokio::task::spawn_blocking(move || sort_held(&held)).await;
        let (trusted, remake) = match checked {
            Ok(sorted) => sorted,
            Err(error) => {
                clear_pending(&state, &dir);
                state.jobs.fail(
                    &jid,
                    cut_core::CutError::new(
                        cut_core::error_codes::IO,
                        "the editing proxies could not be checked",
                        error.to_string(),
                    )
                    .with_suggested_action("reopen the project to check its proxies again"),
                );
                return;
            }
        };
        let given_back: Vec<String> = trusted.iter().map(|(id, _)| id.clone()).collect();
        {
            let mut guard = state.project.write().await;
            let Some(store) = guard.as_mut().filter(|store| store.dir == dir) else {
                clear_pending(&state, &dir);
                return; // another project is open now
            };
            for (id, pointer) in trusted {
                if let Some(asset) = store.project.assets.get_mut(&id) {
                    asset.proxy.get_or_insert(pointer);
                }
            }
            if let Err(error) = store.save() {
                tracing::warn!(error = %error.message, "checked proxies were not saved");
            }
        }
        clear_pending(&state, &dir);
        // Only the file that was imported (same content hash) is read to make a
        // proxy or resume an import: a received project's source path is
        // untrusted text and must not make Cut read some other local file.
        let wanted: Vec<String> = remake.iter().chain(&missing).cloned().collect();
        let candidates = super::proxy_missing::sources(&state, &dir, &wanted).await;
        let (verified, unprobed, source_changed) = match tokio::task::spawn_blocking(move || {
            super::proxy_missing::same_sources(candidates, unprobed)
        })
        .await
        {
            Ok(split) => split,
            Err(_) => (Vec::new(), Vec::new(), wanted),
        };
        let remake: Vec<String> = remake.into_iter().filter(|id| verified.contains(id)).collect();
        let missing: Vec<String> = missing.into_iter().filter(|id| verified.contains(id)).collect();
        // `finish` publishes the job_progress 1.0 ("done"): the one refresh.
        state.jobs.finish(
            &jid,
            serde_json::json!({
                "proxy_check": {"given_back": given_back, "made_again": remake.clone(), "made": missing.clone(), "imports_resumed": unprobed.iter().map(|(id, ..)| id.clone()).collect::<Vec<_>>(), "source_changed": source_changed, "probes_to_read": probes_unknown.clone()},
                "readiness": {"source": "ready", "edit": "ready", "proxy": "ready"},
            }),
        );
        for (id, src, hash, make_proxy) in unprobed {
            spawn_plain_import_chain(state.clone(), id, src, hash, make_proxy);
        }
        // Probes this engine has no record of are read from their files now,
        // and the decode checks of given-back proxies measured on those.
        let still_unknown =
            super::probe_recheck::reread_probes(&state, &dir, &probes_unknown).await;
        let reread: Vec<String> = given_back
            .iter()
            .filter(|id| probes_unknown.contains(id) && !still_unknown.contains(id))
            .cloned()
            .collect();
        // After the check's end: a big project's proxies are not held up by
        // these (one probe or strip drawing per asset).
        super::probe_recheck::recheck_derived(&state, &dir, &reread, &strips).await;
        if !still_unknown.is_empty() {
            tracing::info!(assets = ?still_unknown, "probes stay unknown: file missing or changed");
        }
        // One proxy job per clip, queued: PROXY_MAX_RUNNING (1) encode at a time.
        remake_proxies(&state, &dir, remake.into_iter().chain(missing).collect()).await;
    });
}

fn clear_pending(state: &AppState, dir: &Path) {
    if let Ok(mut pending) = state.proxy_check.lock() {
        if pending.as_ref().is_some_and(|check| check.dir == dir) {
            *pending = None;
        }
    }
}

/// `project.state`'s `proxy_check` while the open project's proxies are being
/// checked: those assets have a proxy that is not used yet (not "no proxy").
pub(crate) fn proxy_check_status(state: &AppState, dir: &Path) -> Option<serde_json::Value> {
    let pending = state.proxy_check.lock().ok()?;
    let check = pending.as_ref().filter(|check| check.dir == dir)?;
    Some(serde_json::json!({
        "state": "checking",
        "job_id": check.job_id,
        "assets": check.assets,
    }))
}

async fn remake_proxies(state: &AppState, dir: &Path, assets: Vec<String>) {
    if assets.is_empty() {
        return;
    }
    let Ok((open_dir, _receipts, proxies)) = project_paths(state).await else {
        return;
    };
    if open_dir != dir {
        return;
    }
    for id in assets {
        let source = {
            let guard = state.project.read().await;
            let Some(store) = guard.as_ref() else { return };
            store.project.assets.get(&id).map(|asset| {
                let probe = asset.probe.as_ref();
                (
                    asset_source_path(&store.dir, asset),
                    probe
                        .and_then(|p| p.get("kind"))
                        .and_then(|k| k.as_str())
                        .unwrap_or("")
                        .to_string(),
                    probe
                        .and_then(|p| p.get("duration_ms"))
                        .and_then(|d| d.as_u64()),
                )
            })
        };
        let Some((src, kind, duration_ms)) = source else {
            continue;
        };
        if !src.is_file() {
            continue;
        }
        spawn_proxy_chain(
            state.clone(),
            id,
            src,
            proxies.clone(),
            open_dir.clone(),
            kind,
            duration_ms,
        );
    }
}

#[cfg(test)]
#[path = "proxy_trust_tests.rs"]
mod tests;

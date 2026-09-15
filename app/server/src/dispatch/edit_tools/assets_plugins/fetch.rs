//! Assets.fetch ownership, import admission, and its deterministic transition gate.

use super::*;
use crate::project_materialization::ProjectMaterialization;

/// assets.fetch — import a provider hit as a normal project asset. Needs
/// an open project (the asset lands in it). RE-RESOLVES the hit by id through the
/// provider (no caller-supplied URL — no SSRF surface), then: local_folder
/// imports the file in place only when the id is still under the search `dir`;
/// openverse downloads it (size-capped) into the
/// project's assets/providers/ dir. Import goes through core's record_import +
/// the import chain (receipts/replay intact). The license + attribution are
/// recorded on the op rationale and returned so the caller can credit the source.
pub(in crate::dispatch) async fn assets_fetch(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    // An import has one project owner from source resolution through durable
    // admission. Hold the same ownership-transition lock used by project.open
    // / create / close / delete across provider I/O: otherwise an A request
    // could resolve while A is open, then record the result into B after a
    // project switch. This intentionally makes a workspace transition wait
    // for the bounded fetch instead of silently retargeting or losing it.
    // RAII releases the guard on every provider/file/project error below.
    let _project_import_transition = state.project_transition.lock().await;
    assets_fetch_admitted(state, args, actor).await
}

/// B-roll already owns a verified project materialization while it admits the
/// imported asset and later places it. Re-taking `project_transition` here would
/// deadlock the non-reentrant mutex, so this private entry point reuses the same
/// fetch body under that caller-owned proof.
pub(in crate::dispatch) async fn assets_fetch_under_materialization(
    state: &AppState,
    args: Value,
    actor: Actor,
    _materialization: &ProjectMaterialization<'_>,
) -> Result<VerbResult, CutError> {
    assets_fetch_admitted(state, args, actor).await
}

async fn assets_fetch_admitted(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        provider: String,
        id: String,
        kind: Option<String>,
        dir: Option<String>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    let kind = a.kind.clone().unwrap_or_else(|| "audio".to_string());
    let local_scoped_path = if a.provider == "local_folder" {
        Some(resolve_local_folder_fetch_path(&a.id, a.dir.as_deref())?)
    } else {
        None
    };

    let proj_dir = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        store.dir.clone()
    };
    #[cfg(test)]
    wait_for_assets_fetch_project_transition_gate_after_pin(&a.id).await;

    // Resolve the authoritative hit (download URL + license) — blocking.
    let (provider, id, kind_c) = (
        a.provider.clone(),
        local_scoped_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| a.id.clone()),
        kind.clone(),
    );
    let hit =
        tokio::task::spawn_blocking(move || crate::providers::resolve(&provider, &id, &kind_c))
            .await
            .map_err(|e| {
                CutError::new(error_codes::IO, "resolve task panicked", e.to_string())
            })??;

    // Determine the local source path: local_folder + stickers import in place (the
    // sticker is rendered to a local PNG at resolve time); a network provider
    // downloads into the project's assets/providers/<provider>/ dir.
    let src_path: PathBuf = if hit.provider == "local_folder" {
        local_scoped_path.unwrap_or_else(|| PathBuf::from(&hit.download_url))
    } else if hit.provider == "stickers" {
        PathBuf::from(&hit.download_url)
    } else {
        let ext = hit
            .filetype
            .clone()
            .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()) && !e.is_empty())
            .unwrap_or_else(|| "bin".to_string());
        let safe_id: String = hit
            .id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let dest = proj_dir
            .join("assets")
            .join("providers")
            .join(&hit.provider)
            .join(format!("{safe_id}.{ext}"));
        let url = hit.download_url.clone();
        let dest_c = dest.clone();
        let target = crate::providers::prepare_download_target(url).await?;
        let n = tokio::task::spawn_blocking(move || {
            crate::providers::download_vetted_to(target, &dest_c)
        })
        .await
        .map_err(|e| CutError::new(error_codes::IO, "download task panicked", e.to_string()))??;
        tracing::info!("assets.fetch downloaded {n} bytes for {}", hit.id);
        dest
    };

    if !src_path.is_file() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("fetched asset not found: {}", src_path.display()),
            "the provider returned a path/url that did not yield a readable file",
        ));
    }
    let src = src_path.canonicalize()?;
    let hash = cut_core::hash_file(&src)?;
    // Record the credit on the op rationale so the timeline history carries it.
    let rationale = a
        .rationale
        .clone()
        .unwrap_or_else(|| format!("fetch {} — {}", hit.provider, hit.attribution));
    let asset = cut_core::Asset {
        path: src.display().to_string(),
        hash: hash.clone(),
        probe: None,
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    };
    let (asset_id, op) = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().ok_or_else(no_project)?;
        guard_call("assets.fetch", || {
            store.record_import(None, asset, actor, Some(rationale.clone()))
        })?
    };
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    let job = spawn_plain_import_chain(state.clone(), asset_id.clone(), src, hash, true);
    #[cfg(test)]
    wait_for_assets_fetch_project_transition_gate_after_admission(&a.id).await;
    Ok(VerbResult::ok_with_ops(
        json!({
            "asset_id": asset_id,
            "job_id": job,
            "provider": hit.provider,
            "title": hit.title,
            "license": hit.license,
            "license_url": hit.license_url,
            "attribution": hit.attribution,
            "requires_attribution": hit.requires_attribution,
            "source_url": hit.source_url,
            "op": op_for_result(&op, wants_legacy_inverse(&args)),
        }),
        vec![op_id],
    ))
}

/// Test-only deterministic barrier for the project-owner race regression.
/// Production builds contain neither the barrier nor its global registration.
#[cfg(test)]
#[derive(Clone)]
pub(in crate::dispatch) struct AssetsFetchProjectTransitionGate {
    fetch_id: String,
    pub project_pinned: std::sync::Arc<tokio::sync::Notify>,
    pub continue_after_pin: std::sync::Arc<tokio::sync::Notify>,
    pub import_admitted: std::sync::Arc<tokio::sync::Notify>,
    pub continue_after_admission: std::sync::Arc<tokio::sync::Notify>,
}

#[cfg(test)]
impl AssetsFetchProjectTransitionGate {
    pub(in crate::dispatch) fn new(fetch_id: impl Into<String>) -> Self {
        Self {
            fetch_id: fetch_id.into(),
            project_pinned: std::sync::Arc::new(tokio::sync::Notify::new()),
            continue_after_pin: std::sync::Arc::new(tokio::sync::Notify::new()),
            import_admitted: std::sync::Arc::new(tokio::sync::Notify::new()),
            continue_after_admission: std::sync::Arc::new(tokio::sync::Notify::new()),
        }
    }
}

#[cfg(test)]
static ASSETS_FETCH_PROJECT_TRANSITION_GATE: std::sync::OnceLock<
    std::sync::Mutex<Option<AssetsFetchProjectTransitionGate>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
fn assets_fetch_project_transition_gate(
) -> &'static std::sync::Mutex<Option<AssetsFetchProjectTransitionGate>> {
    ASSETS_FETCH_PROJECT_TRANSITION_GATE.get_or_init(|| std::sync::Mutex::new(None))
}

#[cfg(test)]
pub(in crate::dispatch) fn install_assets_fetch_project_transition_gate(
    gate: Option<AssetsFetchProjectTransitionGate>,
) {
    *assets_fetch_project_transition_gate()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = gate;
}

#[cfg(test)]
fn current_assets_fetch_project_transition_gate(
    fetch_id: &str,
) -> Option<AssetsFetchProjectTransitionGate> {
    assets_fetch_project_transition_gate()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .filter(|gate| gate.fetch_id == fetch_id)
        .cloned()
}

#[cfg(test)]
async fn wait_for_assets_fetch_project_transition_gate_after_pin(fetch_id: &str) {
    let Some(gate) = current_assets_fetch_project_transition_gate(fetch_id) else {
        return;
    };
    // notify_one retains a permit when the test has not polled its waiter yet.
    gate.project_pinned.notify_one();
    gate.continue_after_pin.notified().await;
}

#[cfg(test)]
async fn wait_for_assets_fetch_project_transition_gate_after_admission(fetch_id: &str) {
    let Some(gate) = current_assets_fetch_project_transition_gate(fetch_id) else {
        return;
    };
    // Keep this milestone observable even if the test has not polled yet.
    gate.import_admitted.notify_one();
    gate.continue_after_admission.notified().await;
}

fn resolve_local_folder_fetch_path(id: &str, dir: Option<&str>) -> Result<PathBuf, CutError> {
    let Some(dir) = dir.map(str::trim).filter(|d| !d.is_empty()) else {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "local_folder fetch needs the original search dir",
            "pass the same `dir` used for assets.search so the local hit can be fenced",
        ));
    };
    let root = PathBuf::from(dir).canonicalize().map_err(|e| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("local_folder search dir not found: {dir}"),
            e.to_string(),
        )
    })?;
    if !root.is_dir() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!(
                "local_folder search dir is not a folder: {}",
                root.display()
            ),
            "pass the folder originally used for assets.search",
        ));
    }
    let path = PathBuf::from(id).canonicalize().map_err(|e| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("local_folder hit not found: {id}"),
            e.to_string(),
        )
    })?;
    if !path.is_file() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("local_folder hit is not a file: {}", path.display()),
            "pass an id returned by assets.search",
        ));
    }
    if !path.starts_with(&root) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "local_folder hit is outside the searched folder",
            format!("{} is not under {}", path.display(), root.display()),
        ));
    }
    Ok(path)
}

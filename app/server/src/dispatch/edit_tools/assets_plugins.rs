//! Effects catalogs, media search/index, plugin, asset-provider, generation, and agent-chat handlers.

use super::*;
use crate::dispatch::generated_assets::{
    copy_generation_references, generation_family_id, generation_id, normalize_variation,
    read_generation_provenance, resolve_generation_references, validate_generation_references,
    GenerationReference,
};
use crate::jobs::{run_owned, ProcessControl, ProcessTermination};

/// effects.list — the effects-as-data CATALOG: every `edit.effect` effect
/// with its track (video/audio), description, overlay-only flag, and parameter
/// schema (name/type/range/default). A pure read (no project needed) so a UI /
/// agent can DISCOVER effects + their params without hardcoding. Generated from
/// cut-core's `effect_specs()` (drift-guarded against ClipEffect), so it never
/// drifts from what `edit.effect` accepts.
pub(in crate::dispatch) async fn effects_list(
    _state: &AppState,
    _args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    let effects = cut_core::effect_specs();
    Ok(VerbResult::ok(json!({
        "count": effects.len(),
        "effects": serde_json::to_value(&effects)?,
    })))
}

/// transitions.list — the transitions-as-data CATALOG: every `edit.crossfade`
/// VIDEO-transition style (ffmpeg `xfade`) with its family + direction + a one-line
/// description, so a UI/agent can DISCOVER and pick a transition by look/direction
/// without hardcoding the names. A pure read (no project needed). Generated from
/// cut-core's `transition_specs()` (drift-guarded against the canonical TRANSITIONS
/// set), so it never drifts from what `edit.crossfade {transition}` accepts. Also
/// returns `categories` (the distinct families) for grouped UI.
pub(in crate::dispatch) async fn transitions_list(
    _state: &AppState,
    _args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    let specs = cut_core::transition_specs();
    let mut categories: Vec<&str> = specs.iter().map(|s| s.category).collect();
    categories.dedup(); // specs are grouped by family, so dedup gives the order
    Ok(VerbResult::ok(json!({
        "count": specs.len(),
        "categories": categories,
        "transitions": serde_json::to_value(&specs)?,
    })))
}

/// media.search — ON-DEVICE VISUAL SEARCH: find the MOMENTS in indexed
/// clips that match a query, by matching a query embedding against per-frame
/// SigLIP2 image embeddings and merging the best adjacent frames into time
/// ranges. Pass `query_vector` (an already-embedded query — the advanced/agent
/// path, and the one usable without the model) OR `query` text (which needs the
/// SigLIP2 TEXT encoder to embed). `asset` narrows to one clip; omitted searches
/// every indexed asset. Returns ranked {asset, start_ms, end_ms, peak_ms, score}.
pub(in crate::dispatch) async fn media_search(
    state: &AppState,
    args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        query: Option<String>,
        query_vector: Option<Vec<f32>>,
        asset: Option<String>,
        top_k: Option<usize>,
        max_gap_ms: Option<u64>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let top_k = a.top_k.unwrap_or(8).clamp(1, 50);
    let max_gap = a.max_gap_ms.unwrap_or(2000);

    let (proj_dir, all_assets) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        (
            store.dir.clone(),
            store.project.assets.keys().cloned().collect::<Vec<_>>(),
        )
    };

    // Resolve the query vector: a raw vector (advanced / no model needed) wins;
    // a text query needs the SigLIP2 text encoder (not available without the
    // indexer model) — fail with a clear, honest pointer instead of guessing.
    let qvec: Vec<f32> = if let Some(v) = a.query_vector {
        v
    } else if let Some(text) = a.query.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        // Embed the TEXT query with the SigLIP2 text tower (the same encoder the
        // index used) via the indexer's --embed-text mode. Needs the perception
        // runtime; absent → an honest setup pointer (or pass query_vector).
        let rt = crate::vissearch::runtime().ok_or_else(|| {
            CutError::new(
                error_codes::UNIMPLEMENTED,
                "a TEXT query needs the SigLIP2 text encoder",
                "set up perception (its venv carries the encoder), or pass `query_vector` for an already-embedded query",
            )
        })?;
        let (py, script, model, q) = (
            rt.python.clone(),
            rt.script.clone(),
            rt.model.clone(),
            text.to_string(),
        );
        let outp = tokio::task::spawn_blocking(move || {
            let mut command = std::process::Command::new(&py);
            command
                .arg(&script)
                .arg("--model")
                .arg(&model)
                .arg("--embed-text")
                .arg(&q);
            crate::dispatch::run_bounded_foreground_command(&mut command, "SigLIP2 text encoder")
        })
        .await
        .map_err(|e| {
            CutError::new(
                error_codes::SIDECAR,
                "text-embed task panicked",
                e.to_string(),
            )
        })?
        .map_err(|e| {
            CutError::new(
                error_codes::SIDECAR,
                "could not run the text encoder",
                e.to_string(),
            )
        })?;
        if !outp.status.success() {
            return Err(CutError::new(
                error_codes::SIDECAR,
                "the SigLIP2 text encoder failed",
                String::from_utf8_lossy(&outp.stderr)
                    .lines()
                    .last()
                    .unwrap_or("")
                    .to_string(),
            ));
        }
        // Wire discipline: the JSON is the LAST {…} line on stdout.
        let stdout = String::from_utf8_lossy(&outp.stdout);
        let line = stdout
            .lines()
            .rev()
            .find(|l| l.trim_start().starts_with('{'))
            .unwrap_or("");
        let parsed: Value = serde_json::from_str(line).map_err(|e| {
            CutError::new(
                error_codes::SIDECAR,
                "text encoder returned non-JSON",
                e.to_string(),
            )
        })?;
        parsed
            .get("v")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|n| n.as_f64().map(|f| f as f32))
                    .collect::<Vec<f32>>()
            })
            .filter(|vec| !vec.is_empty())
            .ok_or_else(|| {
                CutError::new(
                    error_codes::SIDECAR,
                    "text encoder returned no vector",
                    "expected {\"v\":[...]}",
                )
            })?
    } else {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "no query",
            "pass `query` (text) or `query_vector` (a raw embedding)",
        ));
    };

    // When an asset is named, it MUST be a real project asset — both so the
    // result is meaningful and so `asset` can't traverse paths via the embeddings
    // filename (load_index joins it into <proj>/embeddings/<asset>.json).
    let targets: Vec<String> = match a.asset {
        Some(x) => {
            if !all_assets.iter().any(|id| id == &x) {
                return Err(CutError::new(
                    error_codes::NOT_FOUND,
                    format!("unknown asset '{x}'"),
                    "pass an imported asset id (see project.state), or omit `asset` to search all",
                ));
            }
            vec![x]
        }
        None => all_assets,
    };
    let mut hits: Vec<Value> = Vec::new();
    let mut indexed_any = false;
    for aid in &targets {
        let Some(index) = crate::vissearch::load_index(&proj_dir, aid) else {
            continue;
        };
        indexed_any = true;
        let found = crate::vissearch::search(&index, &qvec, top_k, max_gap)
            .map_err(|e| CutError::new(error_codes::INVALID_ARGS, "visual search failed", e))?;
        for h in found {
            hits.push(json!({
                "asset": aid, "start_ms": h.start_ms, "end_ms": h.end_ms,
                "peak_ms": h.peak_ms, "score": h.score,
            }));
        }
    }
    if !indexed_any {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            "no visual-search index for the requested asset(s)",
            "run media.index{asset} first — it builds the per-frame SigLIP2 embeddings",
        ));
    }
    // Rank across assets by score desc, keep top_k.
    hits.sort_by(|x, y| {
        let sx = x["score"].as_f64().unwrap_or(0.0);
        let sy = y["score"].as_f64().unwrap_or(0.0);
        sy.partial_cmp(&sx).unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.truncate(top_k);
    Ok(VerbResult::ok(json!({ "count": hits.len(), "hits": hits })))
}

/// media.index_status — read which project assets already have persisted
/// visual-search indexes. This is intentionally model-free: reopening a project
/// should show prior indexes as searchable without reinstalling/rerunning the
/// SigLIP2 indexer.
pub(in crate::dispatch) async fn media_index_status(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        asset: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let (proj_dir, all_assets) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        (
            store.dir.clone(),
            store.project.assets.keys().cloned().collect::<Vec<_>>(),
        )
    };
    let targets: Vec<String> = match a.asset {
        Some(x) => {
            if !all_assets.iter().any(|id| id == &x) {
                return Err(CutError::new(
                    error_codes::NOT_FOUND,
                    format!("unknown asset '{x}'"),
                    "pass an imported asset id (see project.state), or omit `asset` to inspect all indexes",
                ));
            }
            vec![x]
        }
        None => all_assets,
    };
    let mut assets = Vec::new();
    for aid in targets {
        let Some(index) = crate::vissearch::load_index(&proj_dir, &aid) else {
            continue;
        };
        assets.push(json!({
            "asset": aid,
            "indexed_frames": index.frames.len(),
            "dim": index.dim,
            "model": index.model,
            "path": crate::vissearch::index_path(&proj_dir, &aid).display().to_string(),
        }));
    }
    Ok(VerbResult::ok(json!({
        "count": assets.len(),
        "assets": assets,
    })))
}

/// media.index — build the visual-search index for an asset: sample its
/// frames + embed each with the SigLIP2 image encoder → `embeddings/<asset>.json`
/// (then media.search can find moments by content). The indexer is OPTIONAL +
/// fetch-on-consent (the matte pattern): it needs the perception venv +
/// onnxruntime + a SigLIP2 ONNX model. Absent → an actionable setup error (core
/// editing + search of an existing index work without it). Requires an open
/// project.
pub(in crate::dispatch) async fn media_index(
    state: &AppState,
    args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        asset: String,
        /// Frames per second to sample for embedding (default ~1.0).
        fps: Option<f64>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let fps = a.fps.unwrap_or(1.0).clamp(0.1, 8.0);
    let (proj_dir, src_path) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let asset = store.project.assets.get(&a.asset).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("unknown asset '{}'", a.asset),
                "import the media first (media.import)",
            )
        })?;
        (store.dir.clone(), PathBuf::from(&asset.path))
    };
    let rt = crate::vissearch::runtime().ok_or_else(|| {
        CutError::new(
            error_codes::SIDECAR,
            "the visual-search indexer is not installed",
            "media.index needs the local captions/search runtime (onnxruntime) + a SigLIP2 ONNX model. Choose Install captions and fetch the SigLIP2 model, then re-run. Core editing and search of an already-built index work without it.",
        )
    })?;
    let out = crate::vissearch::index_path(&proj_dir, &a.asset);
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| CutError::new(error_codes::IO, "create embeddings dir", e.to_string()))?;
    }
    // One-shot indexer: python siglip_index.py <in> <out.json> --model M --fps F
    // --asset ID. Runs off the async runtime (frame extract + ONNX inference).
    let (py, script, model) = (rt.python.clone(), rt.script.clone(), rt.model.clone());
    let (in_s, out_s, asset_id) = (src_path.clone(), out.clone(), a.asset.clone());
    let status = tokio::task::spawn_blocking(move || {
        let mut command = std::process::Command::new(&py);
        command
            .arg(&script)
            .arg(&in_s)
            .arg(&out_s)
            .arg("--model")
            .arg(&model)
            .arg("--fps")
            .arg(format!("{fps}"))
            .arg("--asset")
            .arg(&asset_id);
        crate::dispatch::run_bounded_foreground_command(&mut command, "SigLIP2 indexer")
    })
    .await
    .map_err(|e| CutError::new(error_codes::SIDECAR, "indexer task panicked", e.to_string()))?
    .map_err(|e| {
        CutError::new(
            error_codes::SIDECAR,
            "could not run the indexer",
            e.to_string(),
        )
    })?;
    if !status.status.success() {
        return Err(CutError::new(
            error_codes::SIDECAR,
            "the visual-search indexer failed",
            format!("siglip_index.py exited with {}", status.status),
        ));
    }
    let index = crate::vissearch::load_index(&proj_dir, &a.asset).ok_or_else(|| {
        CutError::new(
            error_codes::SIDECAR,
            "the indexer did not write a valid index",
            "check the perception venv has onnxruntime + the SigLIP2 model",
        )
    })?;
    Ok(VerbResult::ok(json!({
        "asset": a.asset,
        "indexed_frames": index.frames.len(),
        "dim": index.dim,
        "model": index.model,
        "path": out.display().to_string(),
    })))
}

/// plugins.list — list the registered PLUGINS: each is a permission-
/// scoped subset of the verb registry {name, version, description, provides,
/// consumes, enabled}. A pure read. The asset providers + matte runtime are the
/// first capabilities re-expressed as plugins.
pub(in crate::dispatch) async fn plugins_list(
    _state: &AppState,
    _args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    Ok(VerbResult::ok(crate::plugins::list_json()))
}

/// plugins.enable — enable or disable a plugin, persisted. A disabled
/// plugin fails closed: every plugins.call under its name is rejected.
pub(in crate::dispatch) async fn plugins_enable(
    _state: &AppState,
    args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        name: String,
        /// true = enable (default), false = disable.
        enabled: Option<bool>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    if crate::plugins::find(&a.name).is_none() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("unknown plugin '{}'", a.name),
            "see plugins.list",
        ));
    }
    let enabled = a.enabled.unwrap_or(true);
    let update = crate::plugins::set_enabled(&a.name, enabled).map_err(|e| {
        CutError::new(
            error_codes::IO,
            "could not persist the plugin state",
            e.to_string(),
        )
    })?;
    Ok(VerbResult::ok(json!({
        "name": a.name,
        "enabled": enabled,
        "permission_state": if update.recovered { "recovered" } else { "ready" },
    })))
}

/// plugins.call — the SCOPED-DISPATCH gateway: run `verb` with `args`
/// UNDER plugin `plugin`'s identity, ONLY if the plugin is enabled AND the verb
/// is within its declared scope (provides ∪ consumes). Out-of-scope or disabled
/// → rejected (guardrail). This is how "a plugin is callable only within its
/// scope" is enforced — the host re-dispatches through the SAME verb registry
/// (no parallel API), so the inner verb behaves identically, just permission-
/// fenced. Returns the inner verb's envelope under {plugin, verb, result}.
pub(in crate::dispatch) async fn plugins_call(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        plugin: String,
        verb: String,
        args: Option<Value>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let manifest = crate::plugins::find(&a.plugin).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("unknown plugin '{}'", a.plugin),
            "see plugins.list",
        )
    })?;
    match crate::plugins::access(&a.plugin) {
        crate::plugins::PluginAccess::Enabled => {}
        crate::plugins::PluginAccess::Disabled => {
            return Err(CutError::new(
                error_codes::GUARDRAIL,
                format!("plugin '{}' is disabled", a.plugin),
                "enable it with plugins.enable first",
            ));
        }
        crate::plugins::PluginAccess::Blocked(problem) => {
            let state = match problem {
                crate::plugins::PermissionStateProblem::Corrupt => "corrupt",
                crate::plugins::PermissionStateProblem::Unavailable => "unavailable",
            };
            return Err(CutError::new(
                error_codes::GUARDRAIL,
                format!("plugin permission state is {state}; all plugins are blocked"),
                format!(
                    "inspect plugins.list, then repair the explicit '{}' grant with plugins.enable {{\"name\":\"{}\",\"enabled\":true}}",
                    a.plugin, a.plugin
                ),
            ));
        }
    }
    // A plugin can never reach the plugin-control surface (no privilege games).
    if a.verb.starts_with("plugins.") {
        return Err(CutError::new(
            error_codes::GUARDRAIL,
            "plugins.* cannot be invoked through plugins.call",
            "call the inner capability verb, not the plugin controls",
        ));
    }
    if !crate::plugins::in_scope(manifest, &a.verb) {
        return Err(CutError::new(
            error_codes::GUARDRAIL,
            format!("verb '{}' is outside plugin '{}' scope", a.verb, a.plugin),
            format!(
                "plugin '{}' provides {:?} and consumes {:?}",
                a.plugin, manifest.provides, manifest.consumes
            ),
        ));
    }
    // In scope + enabled → re-dispatch through the SAME registry (the inner verb
    // is fully arg-checked + routed exactly as a direct call). Box::pin breaks
    // the async-fn recursion (plugins.call → dispatch → … → plugins.call is
    // already blocked above, so this is bounded one level).
    let inner = Box::pin(dispatch(
        state,
        &a.verb,
        a.args.unwrap_or_else(|| json!({})),
        Actor {
            kind: cut_core::ActorKind::Agent,
            name: format!("plugin:{}", a.plugin),
            via: format!("plugins.call/{}", actor.via),
            request: actor.request.clone(),
        },
    ))
    .await;
    if !inner.ok {
        return Ok(inner);
    }
    Ok(VerbResult::ok(json!({
        "plugin": a.plugin,
        "verb": a.verb,
        "result": serde_json::to_value(&inner)?,
    })))
}

/// assets.providers — list the pluggable ASSET PROVIDERS: each entry is
/// {name, kinds, needs_key, network, note}. A pure read (no project needed) so a
/// UI / agent can discover where to search before calling assets.search.
pub(in crate::dispatch) async fn assets_providers(
    _state: &AppState,
    _args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    Ok(VerbResult::ok(
        json!({ "providers": crate::providers::provider_info() }),
    ))
}

/// assets.search — search a provider for media. Stateless (no project
/// needed). `kind` ∈ {audio,image,video} (default audio); `dir` is required for
/// local_folder. Returns normalized hits {provider,id,title,kind,creator,license,
/// license_url,source_url,download_url,filetype,duration_ms,filesize,attribution,
/// requires_attribution}. The blocking network/fs work runs off the async runtime.
pub(in crate::dispatch) async fn assets_search(
    state: &AppState,
    args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    let _ = state;
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        provider: String,
        q: String,
        kind: Option<String>,
        limit: Option<usize>,
        /// local_folder only: the folder to search.
        dir: Option<String>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let kind = a.kind.clone().unwrap_or_else(|| "audio".to_string());
    let limit = a.limit.unwrap_or(12);
    let (provider, q, kind_c, dir) = (a.provider.clone(), a.q.clone(), kind.clone(), a.dir.clone());
    let hits = tokio::task::spawn_blocking(move || {
        crate::providers::search(&provider, &q, &kind_c, limit, dir.as_deref())
    })
    .await
    .map_err(|e| CutError::new(error_codes::IO, "search task panicked", e.to_string()))??;
    let hits_json: Vec<Value> = hits.iter().map(|h| h.to_json()).collect();
    Ok(VerbResult::ok(json!({
        "provider": a.provider,
        "kind": kind,
        "count": hits.len(),
        "hits": hits_json,
    })))
}

/// Partition the op-log tail for one Agent Chat turn and compute its review
/// artifact. Only ops carrying this turn's unique proxy actor are claimed as
/// actions. Any other op in the tail makes whole-turn revert unsafe because
/// `project.revert{to}` deliberately restores the complete timeline prefix.
async fn agent_chat_turn_review(
    state: &AppState,
    ops_before: usize,
    baseline: &str,
    turn_actor_name: &str,
    turn_id: &str,
    checkpoint: Option<&str>,
) -> Result<(Vec<Value>, Value), CutError> {
    let (project, all) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        (store.project.clone(), store.log.read_all()?)
    };
    let tail = all.iter().skip(ops_before).collect::<Vec<_>>();
    let is_turn_op = |op: &&OpRecord| {
        op.actor.kind == cut_core::ActorKind::Agent
            && op.actor.name == turn_actor_name
            && op.actor.via == "agent.chat"
    };
    let actions = tail
        .iter()
        .filter(|op| is_turn_op(op))
        .map(|op| json!({"op_id": op.op_id, "verb": op.verb}))
        .collect::<Vec<_>>();
    let concurrent_actions = tail
        .iter()
        .filter(|op| !is_turn_op(op))
        .map(|op| {
            json!({
                "op_id": op.op_id,
                "verb": op.verb,
                "actor": op.actor,
            })
        })
        .collect::<Vec<_>>();
    let tip = all.last().map(|op| op.op_id.clone());
    let baseline_ref = baseline.to_string();
    let (diff, diff_error) = if !actions.is_empty() {
        match tip.as_ref() {
            Some(tip) => match cut_core::diff(&project, &all, &baseline_ref, tip) {
                Ok(summary) => (serde_json::to_value(summary).ok(), None),
                Err(error) => (None, Some(error.message)),
            },
            None => (None, Some("the turn has no history tip".to_string())),
        }
    } else {
        (None, None)
    };
    let revert_safe = !actions.is_empty() && concurrent_actions.is_empty() && diff_error.is_none();
    Ok((
        actions,
        json!({
            "turn_id": turn_id,
            "baseline": baseline,
            "checkpoint": checkpoint,
            "tip": tip,
            "diff": diff,
            "diff_error": diff_error,
            "revert_safe": revert_safe,
            "concurrent_actions": concurrent_actions,
        }),
    ))
}

/// agent.chat{message, attachments?, agent?, model?, timeout_ms?} — natural-language timeline
/// editing (the headline agent-chat feature). Claude uses its pinned contained
/// contract; Codex uses the user's normal native CLI configuration and sandbox.
/// Both run from a fresh disposable cwd with Cut's MCP server connected to THIS
/// running serve (the same open project the UI shows). Grok remains detectable
/// but returns `not_available` until the next release. The op-log is the receipt
/// for every reversible Cut verb the selected agent applies.
/// NO model is hosted; the CLI's logged-in subscription does the reasoning. The
/// handler holds NO project lock during the spawn (the agent's verbs acquire it per
/// call over the proxy — holding it would deadlock).
///
/// Error transparency: agent.chat must never fail silently. Success means the
/// op-log grew because an edit landed, producing `ok:true`.
/// Every path echoes the validated attachment IDs. Every failure returns `ok:false` with a
/// structured `error` (machine category) + `reason` (human) the UI renders:
///   not_available (no supported installed CLI or a deferred provider) ·
///   unsupported_capability (required provider flags are absent) ·
///   spawn (resolved but failed to launch) · timeout · blocked (a CLI cancelled a
///   Cut MCP call) · auth (login/expired session) · cli_error (stderr surfaced) ·
///   no_change (ran but edited nothing — carries the agent's own final message).
pub(in crate::dispatch) async fn agent_chat(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        message: String,
        #[serde(default)]
        attachments: Vec<String>,
        agent: Option<String>,
        model: Option<String>,
        timeout_ms: Option<u64>,
    }
    let a: Args = parse_args(args)?;
    let msg = a.message.trim().to_string();
    if msg.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "empty chat message",
            "type what you want changed, e.g. \"add a marker at 1 second\"",
        ));
    }
    // Resolve attachment membership while briefly holding the project read lock,
    // then drop it before the agent spawn so its MCP edits cannot deadlock.
    let attachments = {
        let project = state.project.read().await;
        let store = project.as_ref().ok_or_else(no_project)?;
        crate::chat::validate_attachment_ids(&a.attachments, |id| {
            store.project.assets.contains_key(id)
        })
        .map_err(|detail| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "invalid chat attachments",
                detail,
            )
        })?
    };
    let plan = json!({
        "request": &msg,
        "reference_ids": &attachments,
        "policy": ["inspect the open project", "apply only reversible editing verbs", "return op-log receipts"],
    });
    // Every non-success path returns a structured error the UI
    // renders inline, so the user always knows WHY a task did not execute. The verb
    // itself succeeds (HTTP 200) with `ok:false` + `error` (machine category) +
    // `reason` (human) + optional `agent_message` (the agent's OWN words) / `detail`
    // (raw CLI tail). `reply` mirrors `reason` for back-compat with the v1 UI.
    let fail = |error: &str,
                reason: String,
                agent: Option<String>,
                actions: Vec<Value>,
                agent_message: Option<String>,
                detail: Option<String>,
                review: Option<Value>|
     -> Result<VerbResult, CutError> {
        Ok(VerbResult::ok(json!({
            "ok": false,
            "agent": agent,
            "reply": reason.clone(),
            "reason": reason,
            "error": error,
            "actions": actions,
            "attachments": &attachments,
            "agent_message": agent_message,
            "detail": detail,
            "plan": &plan,
            "review": review,
            "cost_usd": Value::Null,
        })))
    };
    // Do not quietly fall through from an explicitly requested provider whose
    // Agent Chat route is not enabled in this release.
    if let Some(requested) = a.agent.as_deref() {
        if let Some(reason) = crate::chat::broker::unavailable_reason(requested) {
            return fail(
                "not_available",
                reason.into(),
                Some(requested.into()),
                vec![],
                None,
                None,
                None,
            );
        }
    }
    // Pick the first installed provider with an implemented Agent Chat route.
    let Some(agent) = crate::chat::pick_agent(a.agent.as_deref()) else {
        // Detection is resolve_agent-based (process PATH first, THEN the explicit
        // install-dir ladder incl. grok's off-PATH ~/.grok/bin / a Finder-stripped
        // .app's Homebrew dirs) — so a detected-but-off-PATH agent is NOT
        // mis-reported as missing here.
        let installed: Vec<&str> = crate::chat::CHAT_AGENTS
            .iter()
            .copied()
            .filter(|x| crate::chat::detect(x))
            .collect();
        let reason = if let Some(req) = a.agent.as_deref() {
            if crate::chat::CHAT_AGENTS.contains(&req) {
                format!(
                    "{req} is not available on this machine — install it and sign in to use it for \
                     chat (looked on PATH and in the standard install dirs). Detected agents: {installed:?}."
                )
            } else {
                format!(
                    "'{req}' is not a supported Agent Chat provider — choose Claude or Codex. \
                     Detected agents: {installed:?}."
                )
            }
        } else if installed.is_empty() {
            "no supported coding-agent CLI was found (looked beyond PATH too: ~/.local/bin, \
             Homebrew, ~/.grok/bin). Install Claude Code or Codex and sign in to use Agent Chat."
                .into()
        } else {
            format!(
                "no ready Claude or Codex Agent Chat route is available. Detected CLIs: \
                 {installed:?}."
            )
        };
        return fail(
            "not_available",
            reason,
            a.agent.clone(),
            vec![],
            None,
            None,
            None,
        );
    };
    // The cutd binary that will serve `cutd mcp` (same build as this serve).
    let cutd_exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "cutd".into());
    // The addr the spawned `cutd mcp` must proxy verbs to = THIS serve (so the
    // agent edits the project the UI shows, not whichever serve last wrote the
    // shared discovery file). The provider launch policy passes these values to
    // the Cut MCP child without changing the user's CLI credentials.
    let proxy_addr = state
        .addr
        .read()
        .await
        .clone()
        .unwrap_or_else(crate::httpc::server_addr);
    // Create a new, empty, disposable cwd for every turn. The CLI never runs in
    // the project or user cwd; edits reach the project through Cut's MCP verbs.
    let suffix = {
        use std::sync::atomic::{AtomicU64, Ordering};
        static C: AtomicU64 = AtomicU64::new(0);
        C.fetch_add(1, Ordering::Relaxed)
    };
    let turn_id = format!("chat-{}-{suffix}", std::process::id());
    let turn_actor_name = turn_id.clone();
    let proxy_actor = format!("agent:{turn_actor_name}:agent.chat");
    let workspace = match crate::chat::broker::IsolatedWorkspace::create() {
        Ok(workspace) => workspace,
        Err(reason) => {
            return fail(
                "not_available",
                reason,
                Some(agent.into()),
                vec![],
                None,
                None,
                None,
            );
        }
    };
    let ws = workspace.path().to_path_buf();
    let launch_env = match agent {
        "claude" => match crate::chat::broker::sanitized_environment(&proxy_addr, &proxy_actor) {
            Ok(environment) => environment,
            Err(reason) => {
                return fail(
                    "not_available",
                    reason,
                    Some(agent.into()),
                    vec![],
                    None,
                    None,
                    None,
                );
            }
        },
        "codex" => crate::chat::broker::native_environment(&proxy_addr, &proxy_actor),
        _ => {
            return fail(
                "not_available",
                format!("agent '{agent}' has no launch environment"),
                Some(agent.into()),
                vec![],
                None,
                None,
                None,
            );
        }
    };
    let mcp_path = ws.join("mcp.json");
    if let Err(e) = std::fs::write(
        &mcp_path,
        serde_json::to_vec(&crate::chat::mcp_config(&cutd_exe)).unwrap_or_default(),
    ) {
        let _ = std::fs::remove_dir_all(&ws);
        return Err(CutError::new(
            error_codes::IO,
            "write mcp config",
            e.to_string(),
        ));
    }
    // Resolve the agent to a runnable path (process PATH first, then the explicit
    // install-dir ladder). A resolved absolute CLI path is spawned directly.
    // pick_agent already proved detection, so resolution is Some here; fall back to
    // the bare name defensively rather than aborting the turn.
    let agent_path = crate::gen::resolve_agent(agent)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| agent.to_string());
    if let Err(reason) = crate::chat::broker::verify_installed_agent(
        agent,
        std::path::Path::new(&agent_path),
        &launch_env,
        &ws,
    )
    .await
    {
        return fail(
            "unsupported_capability",
            reason,
            Some(agent.into()),
            vec![],
            None,
            None,
            None,
        );
    }
    let Some(cmd) = crate::chat::build_command(
        agent,
        &agent_path,
        &mcp_path.to_string_lossy(),
        &cutd_exe,
        &proxy_addr,
        &proxy_actor,
        a.model.as_deref(),
    ) else {
        let _ = std::fs::remove_dir_all(&ws);
        return fail(
            "not_available",
            format!("agent '{agent}' has no chat command construction"),
            Some(agent.into()),
            vec![],
            None,
            None,
            None,
        );
    };
    // A future provider may declare an additional workspace-local config file.
    if let Some((rel, contents)) = &cmd.config_file {
        let cfg_path = ws.join(rel);
        if let Some(parent) = cfg_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&cfg_path, contents) {
            let _ = std::fs::remove_dir_all(&ws);
            return Err(CutError::new(
                error_codes::IO,
                "write agent MCP config",
                e.to_string(),
            ));
        }
    }
    let prompt = crate::chat::build_prompt(&msg, &attachments);
    let timeout =
        std::time::Duration::from_millis(a.timeout_ms.unwrap_or(180_000).clamp(10_000, 600_000));
    // Claude and Codex receive the prompt on stdin. The placeholder branch is
    // retained for a future provider that needs a prompt file.
    let resolved_args: Vec<String> = if cmd.via_stdin {
        cmd.args.clone()
    } else {
        let pf = ws.join("prompt.txt");
        if let Err(e) = std::fs::write(&pf, &prompt) {
            let _ = std::fs::remove_dir_all(&ws);
            return Err(CutError::new(
                error_codes::IO,
                "write prompt file",
                e.to_string(),
            ));
        }
        let pfs = pf.to_string_lossy().into_owned();
        cmd.args
            .iter()
            .map(|x| {
                if x == "__PROMPT_FILE__" {
                    pfs.clone()
                } else {
                    x.clone()
                }
            })
            .collect()
    };
    let mut command =
        crate::gen::agent_tokio_command(std::path::Path::new(&cmd.cmd), &resolved_args).map_err(
            |e| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("cannot launch the {agent} CLI safely: {e}"),
                    "the resolved Windows batch shim received an unsafe path or argument",
                )
            },
        )?;
    launch_env.apply(&mut command);
    command
        .current_dir(&ws)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // A raw op id is a stable history reference for project.diff/revert, so the
    // normal case needs no synthetic checkpoint op. Only an empty log needs one
    // to create a valid baseline before the agent can edit.
    let existing_baseline = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        store.log.read_all()?.last().map(|op| op.op_id.clone())
    };
    let (baseline, checkpoint) = if let Some(op_id) = existing_baseline {
        (op_id, None)
    } else {
        let checkpoint_result = Box::pin(dispatch(
            state,
            "project.checkpoint",
            json!({
                "name": format!("before-{turn_id}"),
                "rationale": "auto: establish an empty-project baseline for Agent Chat",
            }),
            actor.clone(),
        ))
        .await;
        if !checkpoint_result.ok {
            let _ = std::fs::remove_dir_all(&ws);
            return Ok(checkpoint_result);
        }
        let checkpoint = checkpoint_result
            .result
            .as_ref()
            .and_then(|result| result["checkpoint"]["id"].as_str())
            .unwrap_or_default()
            .to_string();
        let checkpoint_op = checkpoint_result
            .result
            .as_ref()
            .and_then(|result| result["checkpoint"]["at_op"].as_str())
            .unwrap_or_default()
            .to_string();
        (checkpoint_op, Some(checkpoint))
    };
    // Derive the tail boundary from the exact baseline op, never from a later
    // log length. A human edit can land between selecting the baseline and
    // spawning the agent; it must remain in the tail so revert_safe is false.
    let ops_before = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        store
            .log
            .read_all()?
            .iter()
            .position(|op| op.op_id == baseline)
            .map(|index| index + 1)
            .ok_or_else(|| {
                CutError::new(
                    error_codes::CONFLICT,
                    "the project history changed before Agent Chat started",
                    format!("the selected baseline {baseline} is no longer in the open project"),
                )
                .with_suggested_action("retry the Agent Chat turn against the current project")
            })?
    };
    let control = ProcessControl::for_operation(timeout);
    let out = run_owned(
        &mut command,
        cmd.via_stdin.then_some(prompt.as_bytes()),
        &control,
    )
    .await;
    let _ = std::fs::remove_dir_all(&ws);

    let out = match out {
        Ok(output) => output,
        Err(error) => {
            let (actions, review) = agent_chat_turn_review(
                state,
                ops_before,
                &baseline,
                &turn_actor_name,
                &turn_id,
                checkpoint.as_deref(),
            )
            .await?;
            let (kind, reason) = match error.termination() {
                Some(ProcessTermination::DeadlineExceeded) => (
                    "timeout",
                    format!(
                        "the {agent} agent timed out after {}s — it may have been waiting for an \
                         approval or login that can't be given headlessly (a non-interactive run has \
                         no one to answer a prompt). Try a simpler request, raise timeout_ms, or \
                         confirm the agent is signed in.",
                        timeout.as_secs()
                    ),
                ),
                Some(ProcessTermination::Cancelled(reason)) => (
                    "cancelled",
                    format!("the {agent} agent was cancelled ({})", reason.label()),
                ),
                None => (
                    "cli_error",
                    format!("the {} CLI could not be read to completion: {error}", cmd.cmd),
                ),
            };
            return fail(
                kind,
                reason,
                Some(agent.into()),
                actions,
                None,
                None,
                Some(review),
            );
        }
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let result = crate::chat::parse_result(&stdout);
    let (actions, review) = agent_chat_turn_review(
        state,
        ops_before,
        &baseline,
        &turn_actor_name,
        &turn_id,
        checkpoint.as_deref(),
    )
    .await?;

    // SUCCESS = the turn LANDED at least one edit (the op-log grew). This is the
    // proven path (claude split/fade/speed/title/fade-out, each landing + undoable);
    // Edits are receipts — if they landed, the turn worked, regardless of a noisy
    // exit code. The additive plan/review fields make the turn inspectable and
    // group-revertible without changing the established reply/actions contract.
    if !actions.is_empty() {
        return Ok(VerbResult::ok(json!({
            "ok": true,
            "agent": agent,
            "reply": result.reply,
            "actions": actions,
            "attachments": &attachments,
            "plan": &plan,
            "review": review,
            "cost_usd": result.cost_usd,
        })));
    }

    // NO edit landed → never report a silent success. Classify WHY (blocked / auth /
    // cli_error / ran-but-no-change) and return a structured error the UI renders,
    // carrying the agent's OWN final message on the no-change path so the user sees
    // the reason (a refusal, "couldn't find a clip at 2s", an answer to a question…).
    let (error_kind, reason) =
        crate::chat::classify_failure(agent, &stdout, &stderr, out.status.success(), &result.reply);
    let agent_message = {
        let r = result.reply.trim();
        if r.is_empty()
            || r == "(the agent returned no message)"
            || r == "(no output from the agent CLI)"
        {
            None
        } else {
            Some(result.reply.clone())
        }
    };
    let detail = {
        let t = stderr.trim();
        if t.is_empty() {
            None
        } else {
            let n = t.chars().count();
            Some(t.chars().skip(n.saturating_sub(400)).collect::<String>())
        }
    };
    fail(
        error_kind,
        reason,
        Some(agent.into()),
        actions,
        agent_message,
        detail,
        Some(review),
    )
}

struct GenerationScratch(PathBuf);

impl Drop for GenerationScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn generation_metadata(
    id: &str,
    family_id: &str,
    provider: &str,
    kind: &str,
    model: Option<&str>,
    prompt: &str,
    variation: Option<&str>,
    references: &[GenerationReference],
    created_at_ms: Option<u64>,
    reused: bool,
    provenance_path: &Path,
) -> Value {
    let references = references
        .iter()
        .map(|reference| {
            json!({
                "asset_id": reference.asset_id,
                "content_hash": reference.content_hash,
                "kind": reference.kind,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schema": "shellx-cut/generated-asset/2",
        "generation_id": id,
        "family_id": family_id,
        "provider": provider,
        "kind": kind,
        "model": model,
        "prompt": prompt,
        "variation": variation,
        "references": references,
        "created_at_ms": created_at_ms,
        "reused": reused,
        "cost_usd": null,
        "cost_note": if reused { "reused existing generated media; provider CLI was not run" } else { "provider price is not reported by this CLI; check the provider account" },
        "provenance_path": provenance_path.display().to_string(),
    })
}

fn write_generation_provenance(path: &Path, metadata: &Value, hash: &str) -> Result<(), CutError> {
    let mut document = metadata.clone();
    document["content_hash"] = json!(hash);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&document)?).map_err(|e| {
        CutError::new(
            error_codes::IO,
            "write generated-asset provenance",
            e.to_string(),
        )
    })?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        CutError::new(
            error_codes::IO,
            "publish generated-asset provenance",
            e.to_string(),
        )
    })
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
enum GenerationPlacementArgs {
    Insert {
        track: String,
        at_ms: u64,
        duration_ms: u64,
    },
    Replace {
        target_clip: String,
    },
}

#[derive(Clone)]
enum PreparedGenerationPlacement {
    Insert {
        target_clip: String,
        track: String,
        at_ms: u64,
        duration_ms: u64,
        placeholder_asset: String,
        placeholder_path: PathBuf,
    },
    Replace {
        target_clip: String,
        track: String,
        duration_ms: u64,
    },
}

impl PreparedGenerationPlacement {
    fn target_clip(&self) -> &str {
        match self {
            Self::Insert { target_clip, .. } | Self::Replace { target_clip, .. } => target_clip,
        }
    }

    fn duration_ms(&self) -> u64 {
        match self {
            Self::Insert { duration_ms, .. } | Self::Replace { duration_ms, .. } => *duration_ms,
        }
    }

    fn public_json(&self, state: &str) -> Value {
        match self {
            Self::Insert {
                target_clip,
                track,
                at_ms,
                duration_ms,
                ..
            } => json!({
                "mode": "insert",
                "target_clip": target_clip,
                "track": track,
                "at_ms": at_ms,
                "duration_ms": duration_ms,
                "state": state,
            }),
            Self::Replace {
                target_clip,
                track,
                duration_ms,
            } => json!({
                "mode": "replace",
                "target_clip": target_clip,
                "track": track,
                "duration_ms": duration_ms,
                "state": state,
            }),
        }
    }
}

#[derive(serde::Deserialize)]
struct AssetsGenerateArgs {
    prompt: String,
    kind: Option<String>,
    provider: String,
    model: Option<String>,
    #[serde(default)]
    references: Vec<String>,
    variation: Option<String>,
    placement: Option<GenerationPlacementArgs>,
    timeout_ms: Option<u64>,
    #[allow(dead_code)]
    rationale: Option<String>,
}

async fn wait_for_internal_import(state: &AppState, job_id: &str) -> Result<(), CutError> {
    for _ in 0..200 {
        let Some(job) = state.jobs.get(job_id) else {
            return Err(CutError::new(
                error_codes::NOT_FOUND,
                "pending placeholder import job disappeared",
                job_id.to_string(),
            ));
        };
        match job.state {
            crate::jobs::JobState::Done => return Ok(()),
            crate::jobs::JobState::Failed => {
                return Err(job.error.unwrap_or_else(|| {
                    CutError::new(
                        error_codes::IO,
                        "pending placeholder could not be prepared",
                        "the placeholder import failed without an error record",
                    )
                }))
            }
            crate::jobs::JobState::Queued | crate::jobs::JobState::Running => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    }
    let _ = state.jobs.abort(job_id).await;
    Err(CutError::new(
        error_codes::IO,
        "pending placeholder import timed out",
        "the local placeholder did not become editable within 10 seconds",
    ))
}

fn unlink_generation_placeholder(project_dir: &Path, path: &Path) -> Result<bool, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err("placeholder source is no longer a regular file".into());
    }
    let root = project_dir
        .join("assets/placeholders")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let source = path.canonicalize().map_err(|error| error.to_string())?;
    if source.parent() != Some(root.as_path()) {
        return Err("placeholder source escaped the project placeholder directory".into());
    }
    std::fs::remove_file(&source).map_err(|error| error.to_string())?;
    Ok(true)
}

async fn remove_generation_placeholder(
    state: &AppState,
    project_dir: &Path,
    asset: &str,
    path: &Path,
    actor: Actor,
) -> (Value, Vec<String>) {
    match media_remove(
        state,
        json!({
            "asset": asset,
            "rationale": "remove replaced generated-media placeholder",
        }),
        actor,
    )
    .await
    {
        Ok(removed) if removed.ok => {
            let op_ids = removed.op_ids.unwrap_or_default();
            match unlink_generation_placeholder(project_dir, path) {
                Ok(true) => (json!({"removed": true, "source_deleted": true}), op_ids),
                Ok(false) => (json!({"removed": true, "source_deleted": false}), op_ids),
                Err(error) => (
                    json!({"removed": true, "source_deleted": false, "warning": error}),
                    op_ids,
                ),
            }
        }
        Ok(removed) => (
            json!({
                "removed": false,
                "warning": removed.error.map(|error| error.message).unwrap_or_else(|| "placeholder asset cleanup failed".into()),
            }),
            vec![],
        ),
        Err(error) => (json!({"removed": false, "warning": error.message}), vec![]),
    }
}

async fn cleanup_abandoned_generation_placeholder(
    state: &AppState,
    project_dir: &Path,
    placement: &PreparedGenerationPlacement,
    actor: Actor,
) -> (Value, Vec<String>) {
    let PreparedGenerationPlacement::Insert {
        target_clip,
        placeholder_asset,
        placeholder_path,
        ..
    } = placement
    else {
        return (Value::Null, vec![]);
    };
    let still_reserved = {
        let guard = state.project.read().await;
        guard.as_ref().is_some_and(|store| {
            store
                .project
                .find_clip(target_clip)
                .and_then(|(track_id, index)| store.project.track(track_id).map(|track| &track.clips[index]))
                .is_some_and(|clip| {
                    matches!(clip, cut_core::Clip::Media(media) if media.asset == *placeholder_asset)
                })
        })
    };
    if still_reserved {
        return (
            json!({"removed": false, "retained_for_retry": true}),
            vec![],
        );
    }
    remove_generation_placeholder(
        state,
        project_dir,
        placeholder_asset,
        placeholder_path,
        actor,
    )
    .await
}

async fn prepare_generation_placement(
    state: &AppState,
    requested: Option<&GenerationPlacementArgs>,
    generation_id: &str,
    actor: Actor,
) -> Result<Option<PreparedGenerationPlacement>, CutError> {
    let Some(requested) = requested else {
        return Ok(None);
    };
    let (project, _, project_dir, _) = snapshot(state).await?;
    match requested {
        GenerationPlacementArgs::Replace { target_clip } => {
            let (track_id, index) = project.find_clip(target_clip).ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no replacement target clip '{target_clip}'"),
                    "select an existing video media clip and retry",
                )
                .with_clip(target_clip)
            })?;
            let track = project.track(track_id).expect("find_clip track exists");
            if track.kind != cut_core::TrackKind::Video {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("replacement target '{target_clip}' is not on a video track"),
                    "generated image/video placement requires a video media clip",
                )
                .with_clip(target_clip));
            }
            if track.locked {
                return Err(CutError::new(
                    error_codes::CONFLICT,
                    format!("track '{}' is locked", track.id),
                    "unlock the track before replacing its selected clip",
                )
                .with_clip(target_clip));
            }
            let cut_core::Clip::Media(clip) = &track.clips[index] else {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("replacement target '{target_clip}' is not a media clip"),
                    "select a video or image clip, not a gap or caption",
                )
                .with_clip(target_clip));
            };
            Ok(Some(PreparedGenerationPlacement::Replace {
                target_clip: target_clip.clone(),
                track: track.id.clone(),
                duration_ms: cut_core::Clip::Media(clip.clone()).timeline_duration_ms(),
            }))
        }
        GenerationPlacementArgs::Insert {
            track,
            at_ms,
            duration_ms,
        } => {
            if *duration_ms == 0 || *duration_ms > 3_600_000 {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "generated-media insertion duration must be between 1ms and 1 hour",
                    format!("duration_ms was {duration_ms}"),
                ));
            }
            let target = project.track(track).ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no target track '{track}'"),
                    "choose an existing video track from project.state",
                )
            })?;
            if target.kind != cut_core::TrackKind::Video {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("track '{track}' is not a video track"),
                    "generated image/video placement requires a video track",
                ));
            }
            if target.locked {
                return Err(CutError::new(
                    error_codes::CONFLICT,
                    format!("track '{track}' is locked"),
                    "unlock the track before inserting generated media",
                ));
            }

            let placeholder_dir = project_dir.join("assets/placeholders");
            std::fs::create_dir_all(&placeholder_dir).map_err(|error| {
                CutError::new(
                    error_codes::IO,
                    "create generated-media placeholder directory",
                    error.to_string(),
                )
            })?;
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let placeholder_path =
                placeholder_dir.join(format!("pending-{generation_id}-{nonce}.png"));
            let scale = (1920.0 / f64::from(project.settings.width.max(1)))
                .min(1080.0 / f64::from(project.settings.height.max(1)))
                .min(1.0);
            let width = (f64::from(project.settings.width) * scale).round().max(1.0) as u32;
            let height = (f64::from(project.settings.height) * scale)
                .round()
                .max(1.0) as u32;
            let cx = width / 2;
            let cy = height / 2;
            let ring = width.min(height).saturating_div(9).max(28);
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">
<rect width="{width}" height="{height}" fill="#17191d"/>
<path d="M0 {cy}H{width}M{cx} 0V{height}" stroke="#2b3038" stroke-width="2"/>
<rect x="3" y="3" width="{}" height="{}" rx="8" fill="none" stroke="#4b5563" stroke-width="6"/>
<circle cx="{cx}" cy="{cy}" r="{ring}" fill="#20242a" stroke="#8b97a8" stroke-width="8" stroke-dasharray="18 14"/>
<path d="M{cx} {}V{cy}L{} {}" fill="none" stroke="#d9e0ea" stroke-width="10" stroke-linecap="round" stroke-linejoin="round"/>
</svg>"##,
                width.saturating_sub(6),
                height.saturating_sub(6),
                cy.saturating_sub(ring / 2),
                cx.saturating_add(ring / 3),
                cy.saturating_add(ring / 5),
            );
            if let Err(error) =
                cut_media::mask::render_svg_png(&svg, width, height, &placeholder_path)
            {
                let _ = std::fs::remove_file(&placeholder_path);
                return Err(error);
            }

            let imported: VerbResult = media_import(
                state,
                json!({
                    "path": placeholder_path.display().to_string(),
                    "proxy": false,
                    "expected_project_dir": project_dir.display().to_string(),
                    "rationale": format!("pending generated media {generation_id}"),
                }),
                actor.clone(),
            )
            .await
            .into();
            if !imported.ok {
                let _ = std::fs::remove_file(&placeholder_path);
                return Err(imported.error.unwrap_or_else(|| {
                    CutError::new(
                        error_codes::IO,
                        "pending placeholder import failed",
                        "media.import returned no error detail",
                    )
                }));
            }
            let imported_result = imported.result.unwrap_or_default();
            let placeholder_asset = imported_result
                .get("asset_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    CutError::new(
                        error_codes::IO,
                        "pending placeholder import returned no asset id",
                        "the media.import result was incomplete",
                    )
                })?
                .to_string();
            let import_job = imported_result
                .get("job_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    CutError::new(
                        error_codes::IO,
                        "pending placeholder import returned no job id",
                        "the media.import result was incomplete",
                    )
                })?;
            if let Err(error) = wait_for_internal_import(state, import_job).await {
                let _ =
                    media_remove(state, json!({"asset": placeholder_asset}), actor.clone()).await;
                let _ = unlink_generation_placeholder(&project_dir, &placeholder_path);
                return Err(error);
            }

            let inserted = edit_insert(
                state,
                json!({
                    "asset": placeholder_asset,
                    "track": track,
                    "at_ms": at_ms,
                    "duration_ms": duration_ms,
                    "rationale": format!("reserve timeline slot for generated media {generation_id}"),
                }),
                actor.clone(),
            )
            .await;
            let inserted = match inserted {
                Ok(result) if result.ok => result,
                Ok(result) => {
                    let _ = media_remove(state, json!({"asset": placeholder_asset}), actor).await;
                    let _ = unlink_generation_placeholder(&project_dir, &placeholder_path);
                    return Err(result.error.unwrap_or_else(|| {
                        CutError::new(
                            error_codes::IO,
                            "pending placeholder could not be inserted",
                            "edit.insert returned no error detail",
                        )
                    }));
                }
                Err(error) => {
                    let _ = media_remove(state, json!({"asset": placeholder_asset}), actor).await;
                    let _ = unlink_generation_placeholder(&project_dir, &placeholder_path);
                    return Err(error);
                }
            };
            let target_clip = inserted
                .result
                .as_ref()
                .and_then(|result| result.get("clip_id"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    CutError::new(
                        error_codes::IO,
                        "pending placeholder insert returned no clip id",
                        "the edit.insert result was incomplete",
                    )
                })?
                .to_string();
            Ok(Some(PreparedGenerationPlacement::Insert {
                target_clip,
                track: track.clone(),
                at_ms: *at_ms,
                duration_ms: *duration_ms,
                placeholder_asset,
                placeholder_path,
            }))
        }
    }
}

async fn apply_generated_placement(
    state: &AppState,
    mut outcome: VerbResult,
    placement: Option<&PreparedGenerationPlacement>,
    kind: &str,
    expected_project_dir: &Path,
    actor: Actor,
) -> VerbResult {
    let Some(placement) = placement else {
        return outcome;
    };
    let Some(asset_id) = outcome
        .result
        .as_ref()
        .and_then(|result| result.get("asset_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return outcome;
    };

    let placement_result = async {
        require_generation_project(state, expected_project_dir).await?;
        if kind == "video" {
            media_probe(state, json!({"asset": asset_id})).await?;
        }
        let mut replace_args = json!({
            "target_clip": placement.target_clip(),
            "asset": asset_id,
            "link_audio": false,
            "rationale": "place completed generated media into its reserved timeline slot",
        });
        if kind == "image" {
            replace_args["source_out_ms"] = json!(placement.duration_ms());
        }
        edit_replace(state, replace_args, actor.clone()).await
    }
    .await;

    match placement_result {
        Ok(applied) if applied.ok => {
            let mut op_ids = outcome.op_ids.take().unwrap_or_default();
            op_ids.extend(applied.op_ids.clone().unwrap_or_default());
            let cleanup = if let PreparedGenerationPlacement::Insert {
                placeholder_asset,
                placeholder_path,
                ..
            } = placement
            {
                let (cleanup, cleanup_ops) = remove_generation_placeholder(
                    state,
                    expected_project_dir,
                    placeholder_asset,
                    placeholder_path,
                    actor,
                )
                .await;
                op_ids.extend(cleanup_ops);
                cleanup
            } else {
                Value::Null
            };
            if let Some(result) = outcome.result.as_mut() {
                let mut placed = placement.public_json("applied");
                placed["edit"] = applied.result.unwrap_or(Value::Null);
                placed["cleanup"] = cleanup;
                result["placement"] = placed;
            }
            if !op_ids.is_empty() {
                outcome.op_ids = Some(op_ids);
            }
        }
        Ok(applied) => {
            let (cleanup, cleanup_ops) = cleanup_abandoned_generation_placeholder(
                state,
                expected_project_dir,
                placement,
                actor,
            )
            .await;
            if !cleanup_ops.is_empty() {
                outcome
                    .op_ids
                    .get_or_insert_with(Vec::new)
                    .extend(cleanup_ops);
            }
            if let Some(result) = outcome.result.as_mut() {
                let mut placed = placement.public_json("failed");
                placed["error"] = serde_json::to_value(applied.error).unwrap_or(Value::Null);
                placed["cleanup"] = cleanup;
                result["placement"] = placed;
            }
        }
        Err(error) => {
            let (cleanup, cleanup_ops) = cleanup_abandoned_generation_placeholder(
                state,
                expected_project_dir,
                placement,
                actor,
            )
            .await;
            if !cleanup_ops.is_empty() {
                outcome
                    .op_ids
                    .get_or_insert_with(Vec::new)
                    .extend(cleanup_ops);
            }
            if let Some(result) = outcome.result.as_mut() {
                let mut placed = placement.public_json("failed");
                placed["error"] = serde_json::to_value(error).unwrap_or(Value::Null);
                placed["cleanup"] = cleanup;
                result["placement"] = placed;
            }
        }
    }
    outcome
}

async fn require_generation_project(state: &AppState, expected_dir: &Path) -> Result<(), CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    if store.dir != expected_dir {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "the project changed while generated media was queued or running",
            format!(
                "generation belongs to {}; the open project is {}",
                expected_dir.display(),
                store.dir.display()
            ),
        )
        .with_suggested_action("return to the original project and submit the generation again"));
    }
    Ok(())
}

/// assets.generate{prompt, kind? = "image", provider, model?, timeout_ms?,
/// rationale?} — enqueue image/video generation through the user's OWN agent CLI
/// Provider work is serialized and cancellable through the shared job system;
/// the completed job imports validated output like a normal upload.
pub(in crate::dispatch) async fn assets_generate(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let a: AssetsGenerateArgs = parse_args(args)?;
    let kind = a.kind.clone().unwrap_or_else(|| "image".to_string());
    let provider = a.provider.clone();
    let variation = normalize_variation(a.variation.as_deref())?;
    let (project, _, project_dir, _) = snapshot(state).await?;
    let references = resolve_generation_references(&project, &a.references)?;
    let family_id = generation_family_id(
        &a.provider,
        &kind,
        a.model.as_deref(),
        a.prompt.trim(),
        &references,
    );
    let generation_id = generation_id(&family_id, variation.as_deref());
    let placement =
        prepare_generation_placement(state, a.placement.as_ref(), &generation_id, actor.clone())
            .await?;
    let queued_placement = placement
        .as_ref()
        .map(|placement| placement.public_json("pending"));
    let task_references = references.clone();
    let task_variation = variation.clone();
    let task_placement = placement.clone();
    let job = state.jobs.create("asset_generate");
    let job_id = job.job_id.clone();
    let task_job_id = job_id.clone();
    let task_state = state.clone();
    state
        .jobs
        .spawn_limited(&job_id, "asset_generate", 1, async move {
            task_state.jobs.progress(
                &task_job_id,
                0.05,
                Some("starting generated-media provider".into()),
            );
            match assets_generate_run(
                &task_state,
                a,
                actor,
                project_dir,
                task_references,
                task_variation,
                task_placement,
                &task_job_id,
            )
            .await
            {
                Ok(outcome) if outcome.ok => {
                    let result = outcome.result.unwrap_or_else(|| json!({"ok": false}));
                    if result.get("ok").and_then(Value::as_bool) == Some(false) {
                        let reason = result
                            .get("reason")
                            .and_then(Value::as_str)
                            .unwrap_or("generation did not produce an asset");
                        task_state.jobs.fail(
                            &task_job_id,
                            CutError::new(
                                "generation_failed",
                                reason,
                                "the provider did not produce reusable media",
                            ),
                        );
                    } else {
                        task_state.jobs.finish(&task_job_id, result);
                    }
                }
                Ok(outcome) => task_state.jobs.fail(
                    &task_job_id,
                    outcome.error.unwrap_or_else(|| {
                        CutError::new(
                            "generation_failed",
                            "generation failed",
                            "the generation worker returned no error detail",
                        )
                    }),
                ),
                Err(error) => task_state.jobs.fail(&task_job_id, error),
            }
        });
    Ok(VerbResult::ok(json!({
        "job_id": job_id,
        "generation_id": generation_id,
        "family_id": family_id,
        "provider": provider,
        "kind": kind,
        "variation": variation,
        "placement": queued_placement,
        "state": "queued",
    })))
}

async fn assets_generate_run(
    state: &AppState,
    a: AssetsGenerateArgs,
    actor: Actor,
    expected_project_dir: PathBuf,
    references: Vec<GenerationReference>,
    variation: Option<String>,
    placement: Option<PreparedGenerationPlacement>,
    generation_job_id: &str,
) -> Result<VerbResult, CutError> {
    let kind = a.kind.clone().unwrap_or_else(|| "image".to_string());

    // --- cost-free guards (NO spawn) ------------------------------------------
    let degrade = |reason: String| {
        Ok(VerbResult::ok(json!({
            "ok": false, "provider": a.provider, "kind": kind, "reason": reason,
        })))
    };
    if a.prompt.trim().is_empty() {
        return degrade("describe what to generate (prompt is empty)".into());
    }
    if crate::gen::cli_for(&a.provider).is_none() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown generation provider '{}'", a.provider),
            "provider is codex (gpt-image) | grok (grok-imagine)",
        ));
    }
    if !crate::gen::supports_kind(&a.provider, &kind) {
        return degrade(format!(
            "provider '{}' does not generate {kind}",
            a.provider
        ));
    }
    if !crate::gen::detect(&a.provider) {
        return degrade(format!(
            "the '{}' CLI is not installed/on PATH — install it (and sign in) to generate",
            crate::gen::cli_for(&a.provider).unwrap_or("?")
        ));
    }

    // --- scratch workspace + exact output path under the project --------------
    let (project, _edl, dir, _at) = snapshot(state).await?;
    require_generation_project(state, &expected_project_dir).await?;
    validate_generation_references(&project, &references, &dir)?;
    let family_id = generation_family_id(
        &a.provider,
        &kind,
        a.model.as_deref(),
        a.prompt.trim(),
        &references,
    );
    let generation_id = generation_id(&family_id, variation.as_deref());
    let durable_dir = dir.join("assets/generated");
    std::fs::create_dir_all(&durable_dir).map_err(|e| {
        CutError::new(
            error_codes::IO,
            "create generated-assets directory",
            e.to_string(),
        )
    })?;
    let extension = if kind == "video" { "mp4" } else { "png" };
    let durable_output = durable_dir.join(format!("{generation_id}.{extension}"));
    let provenance_path = durable_dir.join(format!("{generation_id}.json"));
    if durable_output.is_file() {
        let path_is_symlink = match std::fs::symlink_metadata(&durable_output) {
            Ok(metadata) => metadata.file_type().is_symlink(),
            Err(error) => {
                return degrade(format!(
                    "generated media {generation_id} metadata could not be verified: {error}"
                ));
            }
        };
        if path_is_symlink {
            return degrade(format!(
                "generated media {generation_id} is not reusable because its immutable path is a symlink"
            ));
        }
        let existing_hash = cut_core::hash_file(&durable_output)?;
        let same_path_asset = project
            .assets
            .iter()
            .find(|(_, asset)| Path::new(&asset.path) == durable_output);
        if let Some((_, asset)) = same_path_asset {
            if asset.hash != existing_hash {
                return degrade(format!(
                    "generated media {generation_id} changed after import; restore the original content or remove the old asset before generating again"
                ));
            }
        }
        let provenance = match read_generation_provenance(&provenance_path) {
            Ok(provenance)
                if provenance.generation_id == generation_id
                    && provenance.content_hash == existing_hash =>
            {
                provenance
            }
            Ok(_) => {
                return degrade(format!(
                    "generated media {generation_id} is not reusable because its provenance does not match the immutable file"
                ));
            }
            Err(issue) => {
                return degrade(format!(
                    "generated media {generation_id} is not reusable because its provenance is {}",
                    issue.description()
                ));
            }
        };
        let metadata = generation_metadata(
            &generation_id,
            &family_id,
            &a.provider,
            &kind,
            a.model.as_deref(),
            a.prompt.trim(),
            variation.as_deref(),
            &references,
            provenance.created_at_ms,
            true,
            &provenance_path,
        );
        if let Some((asset_id, _)) = same_path_asset {
            state.jobs.progress(
                generation_job_id,
                0.9,
                Some("reusing generated media".into()),
            );
            let reused = VerbResult::ok(json!({
                "ok": true,
                "asset_id": asset_id,
                "job_id": null,
                "generated": metadata,
            }));
            return Ok(apply_generated_placement(
                state,
                reused,
                placement.as_ref(),
                &kind,
                &expected_project_dir,
                actor,
            )
            .await);
        }

        require_generation_project(state, &expected_project_dir).await?;
        state.jobs.progress(
            generation_job_id,
            0.85,
            Some("importing reusable generated media".into()),
        );
        let import: VerbResult = media_import(
            state,
            json!({
                "path": durable_output.display().to_string(),
                "expected_project_dir": expected_project_dir.display().to_string(),
                "rationale": format!("reuse generated media {generation_id} via {}", a.provider),
            }),
            actor.clone(),
        )
        .await
        .into();
        if import.ok {
            let mut reused = import;
            if let Some(result) = reused.result.as_mut() {
                result["ok"] = json!(true);
                result["generated"] = metadata;
            }
            return Ok(apply_generated_placement(
                state,
                reused,
                placement.as_ref(),
                &kind,
                &expected_project_dir,
                actor,
            )
            .await);
        }
        return degrade(format!(
            "existing generated media {generation_id} is not reusable: {}",
            import
                .error
                .as_ref()
                .map(|e| e.message.as_str())
                .unwrap_or("import failed")
        ));
    }

    let run_nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let ws = dir
        .join("cache/gen/runs")
        .join(format!("{generation_id}-{run_nonce}"));
    std::fs::create_dir_all(&ws)
        .map_err(|e| CutError::new(error_codes::IO, "create gen workspace", e.to_string()))?;
    let _scratch = GenerationScratch(ws.clone());
    let output = ws.join(crate::gen::output_filename(&kind));
    let ws_str = ws.to_string_lossy().into_owned();
    let out_str = output.to_string_lossy().into_owned();
    let reference_paths = copy_generation_references(&project, &references, &dir, &ws)?;

    let cmd =
        crate::gen::build_command(&a.provider, &ws_str, a.model.as_deref()).ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "no command for provider",
                "codex|grok",
            )
        })?;
    let prompt =
        crate::gen::build_prompt(&a.provider, &kind, &a.prompt, &out_str, &reference_paths);
    let timeout =
        std::time::Duration::from_millis(a.timeout_ms.unwrap_or(240_000).clamp(10_000, 1_800_000));

    state.jobs.progress(
        generation_job_id,
        0.15,
        Some(format!("running the {} provider", a.provider)),
    );

    // grok takes the prompt via a file (substitute the placeholder); codex via stdin.
    let mut prompt_file: Option<PathBuf> = None;
    let resolved_args: Vec<String> = if cmd.via_stdin {
        cmd.args.clone()
    } else {
        let pf = ws.join("prompt.txt");
        std::fs::write(&pf, &prompt)
            .map_err(|e| CutError::new(error_codes::IO, "write prompt file", e.to_string()))?;
        let pfs = pf.to_string_lossy().into_owned();
        prompt_file = Some(pf);
        cmd.args
            .iter()
            .map(|x| {
                if x == "__PROMPT_FILE__" {
                    pfs.clone()
                } else {
                    x.clone()
                }
            })
            .collect()
    };

    // --- spawn the agent CLI (bounded) ----------------------------------------
    // Spawn the RESOLVED path, not the bare provider name: gen::detect now uses the
    // full resolve_agent ladder (process PATH first, THEN the off-PATH install dirs
    // incl. grok's self-managed ~/.grok/bin), so a detected-but-off-PATH grok must be
    // launched BY its resolved absolute path or `Command::new("grok")` would ENOENT.
    // detect already proved it resolves ⇒ Some here; fall back to the bare name
    // defensively. An on-PATH codex/grok resolves to itself ⇒ behavior is unchanged.
    let agent_path = crate::gen::resolve_agent(&cmd.cmd)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| cmd.cmd.clone());
    let mut command =
        crate::gen::agent_tokio_command(std::path::Path::new(&agent_path), &resolved_args)
            .map_err(|e| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("cannot launch the {} CLI safely: {e}", cmd.cmd),
                    "the resolved Windows batch shim received an unsafe path or argument",
                )
            })?;
    command.current_dir(&ws);
    let control = ProcessControl::for_operation(timeout);
    let out = run_owned(
        &mut command,
        cmd.via_stdin.then_some(prompt.as_bytes()),
        &control,
    )
    .await;
    if let Some(pf) = &prompt_file {
        let _ = std::fs::remove_file(pf);
    }
    let out = match out {
        Ok(output) => output,
        Err(error) => match error.termination() {
            Some(ProcessTermination::DeadlineExceeded) => {
                return degrade(format!(
                    "generation timed out after {}ms",
                    timeout.as_millis()
                ))
            }
            Some(ProcessTermination::Cancelled(reason)) => {
                return Err(CutError::new(
                    "job_cancelled",
                    format!("generation cancelled ({})", reason.label()),
                    "the owning background job stopped this external worker",
                ))
            }
            None => return degrade(format!("the {} CLI errored: {error}", cmd.cmd)),
        },
    };
    let stdout = String::from_utf8_lossy(&out.stdout);

    // --- parse the CLI's result + validate the file ---------------------------
    if let Some(parsed) = crate::gen::parse_output_json(&stdout) {
        if !parsed.ok {
            return degrade(
                parsed
                    .reason
                    .unwrap_or_else(|| "the generator reported failure".into()),
            );
        }
    }
    if !output.exists() {
        let tail: String = stdout
            .chars()
            .rev()
            .take(200)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        return degrade(format!(
            "the generator did not write the expected file ({out_str}); CLI tail: {tail}"
        ));
    }

    let probe_path = output.clone();
    let generated_probe = match run_blocking("assets.generate.probe", move || {
        cut_media::probe(&probe_path)
    })
    .await
    {
        Ok(probe) => probe,
        Err(error) => {
            return degrade(format!(
                "the generator output is not valid {kind} media: {}",
                error.message
            ))
        }
    };
    if generated_probe.kind != kind {
        return degrade(format!(
            "the generator returned {} media when {kind} was requested",
            generated_probe.kind
        ));
    }

    state.jobs.progress(
        generation_job_id,
        0.7,
        Some("validating generated media".into()),
    );
    require_generation_project(state, &expected_project_dir).await?;
    let (current_project, _current_edl, current_dir, _current_at) = snapshot(state).await?;
    if current_dir != expected_project_dir {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "the project changed while generated media was running",
            "return to the original project and submit the generation again",
        ));
    }
    validate_generation_references(&current_project, &references, &current_dir)?;

    std::fs::rename(&output, &durable_output).map_err(|e| {
        CutError::new(
            error_codes::IO,
            "publish immutable generated media",
            e.to_string(),
        )
    })?;

    let content_hash = cut_core::hash_file(&durable_output)?;
    let metadata = generation_metadata(
        &generation_id,
        &family_id,
        &a.provider,
        &kind,
        a.model.as_deref(),
        a.prompt.trim(),
        variation.as_deref(),
        &references,
        Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        ),
        false,
        &provenance_path,
    );
    if let Err(error) = write_generation_provenance(&provenance_path, &metadata, &content_hash) {
        let _ = std::fs::remove_file(&durable_output);
        return Err(error);
    }

    // --- import the synchronously validated file; the import chain enriches it --
    require_generation_project(state, &expected_project_dir).await?;
    state.jobs.progress(
        generation_job_id,
        0.85,
        Some("importing generated media".into()),
    );
    let import: VerbResult = media_import(
        state,
        json!({
            "path": durable_output.display().to_string(),
            "expected_project_dir": expected_project_dir.display().to_string(),
            "rationale": format!("generated media {generation_id} via {}", a.provider),
        }),
        actor.clone(),
    )
    .await
    .into();
    if !import.ok {
        let _ = std::fs::remove_file(&durable_output);
        let _ = std::fs::remove_file(&provenance_path);
        return degrade(format!(
            "the generated file failed to import/probe (not valid {kind}): {}",
            import
                .error
                .as_ref()
                .map(|e| e.message.clone())
                .unwrap_or_default()
        ));
    }
    let mut res = import;
    if let Some(r) = res.result.as_mut() {
        r["ok"] = json!(true);
        r["generated"] = metadata;
    }
    Ok(apply_generated_placement(
        state,
        res,
        placement.as_ref(),
        &kind,
        &expected_project_dir,
        actor,
    )
    .await)
}

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

    // The project dir (download target for network providers) — and the project
    // must be open to import into.
    let proj_dir = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        store.dir.clone()
    };

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
        let n = tokio::task::spawn_blocking(move || crate::providers::download_to(&url, &dest_c))
            .await
            .map_err(|e| {
                CutError::new(error_codes::IO, "download task panicked", e.to_string())
            })??;
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

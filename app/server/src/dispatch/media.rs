//! Media/import-oriented dispatch handlers.
//!
//! Kept as a child module of `dispatch` so this extraction is behavior-preserving:
//! handlers still share the same project commit, job, event, and error helpers.

use super::*;
use std::io::Read;
use std::sync::Arc;

mod cache_cleanup;
mod decode_check;
mod format_adoption;
mod picture_check;
mod placement_turn;
mod probe_recheck;
mod proxy_missing;
mod proxy_trust;
pub(crate) use proxy_trust::{check_held_proxies, hold_proxies, proxy_binding, proxy_check_status};
#[cfg(test)]
mod picture_check_tests;
mod unprobed_import;
use cache_cleanup::{remove_asset_receipts, remove_owned_cache_outputs_locked};
pub(super) use picture_check::media_check;
pub(crate) use placement_turn::PlacementTurns;

fn plain_generated_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return false;
    }
    // Only a name-surrogate reparse point (symbolic link, junction) names
    // another entry, and `is_symlink` above already reports exactly those;
    // cloud placeholders and other reparse points are the file itself.
    true
}

/// Capture the request's open project before waiting for a concurrent switch,
/// then verify that owner while holding the transition gate. The returned
/// guard keeps subsequent source selection and write-back on that project.
async fn pin_media_project<'a>(
    state: &'a AppState,
) -> Result<tokio::sync::MutexGuard<'a, ()>, CutError> {
    let (dir, identity) = {
        let project = state.project.read().await;
        let store = project.as_ref().ok_or_else(no_project)?;
        (store.dir.clone(), path_free_project_identity(store)?)
    };
    let transition = state.project_transition.lock().await;
    let project = state.project.read().await;
    let store = project.as_ref().ok_or_else(no_project)?;
    if store.dir != dir || path_free_project_identity(store)? != identity {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "the open project changed before media work started",
            "retry the media request against the current project",
        ));
    }
    drop(project);
    Ok(transition)
}

fn pending_rebuild_cleanup_outputs(
    project_dir: &std::path::Path,
    asset: &str,
    warnings: &mut Vec<cut_core::VerbWarning>,
) -> Vec<(crate::cache_lifecycle::CacheKind, Option<String>)> {
    match crate::cache_lifecycle::pending_rebuild_outputs_for_asset(project_dir, asset) {
        Ok(outputs) => outputs
            .into_iter()
            .map(|(kind, relative)| (kind, Some(relative)))
            .collect(),
        Err(error) => {
            warnings.push(cut_core::VerbWarning {
                code: "cache_ownership_cleanup_pending".into(),
                message: format!(
                    "kept pending cache ownership for asset '{asset}' because it could not be reconciled: {} ({})",
                    error.message, error.cause
                ),
                detail: Default::default(),
            });
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------------------
// media.* handlers — import chain + sidecar jobs
// ---------------------------------------------------------------------------

/// Snapshot the bits of the open project the job tasks need (jobs must not
/// hold the project lock while ffmpeg/python run).
pub(crate) async fn project_paths(
    state: &AppState,
) -> Result<(PathBuf, PathBuf, PathBuf), CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let receipts =
        crate::output_paths::ensure_plain_project_relative_dir(&store.dir, Path::new("receipts"))?;
    let proxies =
        crate::output_paths::ensure_plain_project_relative_dir(&store.dir, Path::new("proxies"))?;
    Ok((store.dir.clone(), receipts, proxies))
}

/// Look up an asset's source path + hash (job inputs). The path is absolute.
pub(crate) async fn asset_info(
    state: &AppState,
    asset_id: &str,
) -> Result<(PathBuf, String), CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let asset = store.project.assets.get(asset_id).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("no asset '{asset_id}'"),
            "asset ids come from media.import results / project.state assets",
        )
        .with_suggested_action("call project.state and use an existing asset id")
    })?;
    Ok((asset_source_path(&store.dir, asset), asset.hash.clone()))
}

/// Record that this engine has just read `asset_id`'s probe from its own
/// source, so decisions may use it (cut_core probe_facts).
pub(crate) async fn mark_probe_verified(state: &AppState, asset_id: &str) {
    let recorded = {
        let mut guard = state.project.write().await;
        let Some(store) = guard.as_mut() else {
            return;
        };
        store.project.mark_probe_verified(asset_id);
        let binding = store.project.probe_binding(asset_id);
        let source = store
            .project
            .assets
            .get(asset_id)
            .map(|asset| asset_source_path(&store.dir, asset));
        source.zip(binding)
    };
    // The next open takes this probe as read while the file is unchanged.
    if let Some((source, binding)) = recorded {
        let _ = run_blocking("probe.record", move || {
            cut_media::cache_trust::remember_source(&source, &binding);
            Ok(())
        })
        .await;
    }
}

/// Whether `asset_id`'s current probe is one this engine established.
pub(crate) async fn probe_verified(state: &AppState, asset_id: &str) -> bool {
    let guard = state.project.read().await;
    guard
        .as_ref()
        .is_some_and(|store| store.project.probe_is_verified(asset_id))
}

/// The stored probe of `asset_id`, when the asset exists and has one.
async fn asset_probe(state: &AppState, asset_id: &str) -> Option<Value> {
    let guard = state.project.read().await;
    guard.as_ref()?.project.assets.get(asset_id)?.probe.clone()
}

/// The asset's stored decode check record, for a job result.
async fn current_decode_check(state: &AppState, asset_id: &str) -> Option<Value> {
    asset_probe(state, asset_id)
        .await
        .and_then(|p| p.get("decode_check").cloned())
}

/// Write back one enrichment field on an asset after a job step completes.
/// This is cache enrichment riding on the media.import op (not a new op):
/// probe/transcript/perception paths are derived facts, not timeline edits.
pub(crate) async fn update_asset(
    state: &AppState,
    asset_id: &str,
    f: impl FnOnce(&mut cut_core::Asset),
) -> Result<(), CutError> {
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let asset = store.project.assets.get_mut(asset_id).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("no asset '{asset_id}'"),
            "asset disappeared before enrichment write-back could update project state",
        )
        .with_suggested_action("re-run project.state and retry the operation with a live asset id")
    })?;
    f(asset);
    store.save()
}

/// Persist an asset's probe JSON to `receipts/<id>.probe.json`. Probe is an
/// INLINE cache field (not a file pointer), so unlike proxy/transcript/perception
/// it would be unrecoverable after a log rebuild — writing it out here lets
/// `ProjectStore::open`'s reconcile pass re-point it from disk like the others.
/// Best-effort (a write failure must not fail the import/probe verb).
async fn persist_probe_file(state: &AppState, asset_id: &str, probe: &Value) {
    if asset_id.is_empty()
        || asset_id.len() > 128
        || !asset_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return;
    }
    let dir = {
        let guard = state.project.read().await;
        match guard.as_ref() {
            Some(s) => s.dir.clone(),
            None => return,
        }
    };
    let Ok(receipts) =
        crate::output_paths::ensure_plain_project_relative_dir(&dir, Path::new("receipts"))
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

// ---------------------------------------------------------------------------
// Capture-manifest ingestion — media.import{capture_manifest?}. The OBS-driving
// agent (or any recorder) hands its clock-mapped event log to the
// import; events become namespaced `capture:<type>` markers with the FULL
// event payload (incl. its confidence tag) preserved in the marker note.
//
// HONEST V1 SCOPE: markers land as confidence-tagged claims. The future
// "verify, don't trust" pass (pixel cross-check against detected scene cuts,
// snap-to-frame, clock-skew re-anchor) is v2; until it lands, manifest claims
// never enter RenderReceipt facts and a sloppy capture agent can only poison
// markers, not receipts. Media-time == timeline-time is assumed, which holds
// because ingestion only fires when THIS import was just auto-placed at 0
// full-length (the dominant record→edit flow); otherwise markers are skipped
// with an in-band note (markers are timeline-absolute).
// ---------------------------------------------------------------------------

/// Schema tag a capture manifest must carry.
const CAPTURE_MANIFEST_SCHEMA: &str = "shellx-cut/capture-manifest/1";
const MAX_CAPTURE_MANIFEST_JSON_BYTES: u64 = 32 * 1024 * 1024;

/// Parsed capture manifest. Events stay RAW JSON — the event type set is
/// open by contract (unknown types map to plain markers, forward compatible).
#[derive(Debug, Clone, serde::Deserialize)]
pub(super) struct CaptureManifest {
    schema: String,
    #[serde(default)]
    recording: Option<CaptureRecording>,
    #[serde(default)]
    events: Vec<Value>,
}

/// recording{} subset we consume: the UTC→media-time clock map.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct CaptureRecording {
    #[serde(default)]
    clock_map: Vec<ClockSample>,
}

/// One clock-map sample: wall clock (RFC3339) ↔ media time (ms).
#[derive(Debug, Clone, serde::Deserialize)]
struct ClockSample {
    utc: String,
    media_ms: u64,
}

/// Load + validate a capture manifest file (schema tag checked — a wrong
/// schema is a wrong file, not a degraded one).
pub(super) fn load_capture_manifest(path: &Path) -> Result<CaptureManifest, CutError> {
    load_capture_manifest_with_limit(path, MAX_CAPTURE_MANIFEST_JSON_BYTES)
}

pub(super) fn load_capture_manifest_with_limit(
    path: &Path,
    max_bytes: u64,
) -> Result<CaptureManifest, CutError> {
    let file = std::fs::File::open(path).map_err(|e| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("capture manifest not readable: {}", path.display()),
            e.to_string(),
        )
    })?;
    let mut bytes = Vec::new();
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| {
            CutError::new(
                error_codes::IO,
                format!("capture manifest could not be read: {}", path.display()),
                e.to_string(),
            )
        })?;
    if bytes.len() as u64 > max_bytes {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "capture manifest exceeds the {} MiB limit: {}",
                max_bytes / (1024 * 1024),
                path.display()
            ),
            "capture manifest JSON is too large to process safely",
        ));
    }
    let m: CaptureManifest = serde_json::from_slice(&bytes).map_err(|e| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("capture manifest is not valid JSON: {}", path.display()),
            e.to_string(),
        )
    })?;
    if m.schema != CAPTURE_MANIFEST_SCHEMA {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "capture manifest schema '{}' is not '{CAPTURE_MANIFEST_SCHEMA}'",
                m.schema
            ),
            format!("file: {}", path.display()),
        )
        .with_suggested_action("the recorder must write the documented capture-manifest shape"));
    }
    Ok(m)
}

/// Sorted (utc_epoch_ms, media_ms) anchors from the manifest clock map.
/// Unparseable timestamps are dropped (each sample is independent).
fn clock_anchors(m: &CaptureManifest) -> Vec<(i64, u64)> {
    let mut anchors: Vec<(i64, u64)> = m
        .recording
        .as_ref()
        .map(|r| r.clock_map.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|s| {
            let t = chrono::DateTime::parse_from_rfc3339(&s.utc).ok()?;
            Some((t.timestamp_millis(), s.media_ms))
        })
        .collect();
    anchors.sort_unstable();
    anchors
}

/// Piecewise-linear UTC-to-media-time mapping over the clock anchors. The
/// sample series absorbs pauses and drift. Outside the anchored span
/// the map CLAMPS to the nearest anchor — extrapolating past a pause would
/// invent time; a clamped estimate is honest about its limit.
pub(super) fn utc_to_media_ms(anchors: &[(i64, u64)], utc_ms: i64) -> Option<u64> {
    let (first, last) = (anchors.first()?, anchors.last()?);
    if utc_ms <= first.0 {
        return Some(first.1);
    }
    if utc_ms >= last.0 {
        return Some(last.1);
    }
    let i = anchors.partition_point(|(t, _)| *t <= utc_ms);
    let (t0, m0) = anchors[i - 1];
    let (t1, m1) = anchors[i];
    if t1 == t0 {
        return Some(m0);
    }
    let frac = (utc_ms - t0) as f64 / (t1 - t0) as f64;
    Some((m0 as f64 + frac * (m1 as f64 - m0 as f64)).round() as u64)
}

/// Map one manifest event to edit.add_marker args. Time resolution order:
/// `at_ms` (media time, the contract default) → `range_ms[0]` (spans mark
/// their start; the span survives in the note payload) → `utc` through the
/// clock map. Events with no usable time are skipped (reason returned).
pub(super) fn manifest_event_to_marker(
    ev: &Value,
    anchors: &[(i64, u64)],
) -> Result<Value, String> {
    let ev_type = ev.get("type").and_then(|t| t.as_str()).unwrap_or("event");
    let at_ms = ev
        .get("at_ms")
        .and_then(|v| v.as_f64())
        .map(|v| v.max(0.0).round() as u64)
        .or_else(|| {
            ev.get("range_ms")
                .and_then(|r| r.as_array())
                .and_then(|r| r.first())
                .and_then(|v| v.as_u64())
        })
        .or_else(|| {
            let utc = ev.get("utc").and_then(|v| v.as_str())?;
            let t = chrono::DateTime::parse_from_rfc3339(utc).ok()?;
            utc_to_media_ms(anchors, t.timestamp_millis())
        })
        .ok_or_else(|| {
            format!("event '{ev_type}' has no at_ms/range_ms and no clock-mappable utc")
        })?;
    // Note = the FULL event JSON — payload, confidence tag and provenance
    // survive verbatim (the earlier hand-translation lost all three).
    let note = serde_json::to_string(ev).unwrap_or_default();
    Ok(json!({"at_ms": at_ms, "label": format!("capture:{ev_type}"), "note": note}))
}

/// Resolve the manifest for an import: explicit arg (errors are HARD — you
/// asked for this file) or `<stem>.capture.json` auto-discovery beside the
/// media (used with a warning so the agent knows; a malformed
/// SIDECAR degrades to a warning — you didn't ask for it, the import must
/// not fail on it).
pub(super) fn resolve_capture_manifest(
    src: &Path,
    explicit: Option<&str>,
) -> Result<
    (
        Option<(PathBuf, CaptureManifest)>,
        Vec<cut_core::VerbWarning>,
    ),
    CutError,
> {
    let mut warnings = Vec::new();
    if let Some(p) = explicit {
        let src_dir = src.parent().unwrap_or_else(|| Path::new("."));
        let raw = Path::new(p);
        let candidate = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            src_dir.join(raw)
        };
        let path = fenced_existing_file_under_dir(
            src_dir,
            &candidate,
            "capture manifest",
            "place the capture manifest beside the media file, or omit it for sidecar auto-discovery",
        )?;
        let m = load_capture_manifest(&path)?;
        return Ok((Some((path, m)), warnings));
    }
    let sidecar = src.with_extension("capture.json");
    if sidecar.is_file() {
        match load_capture_manifest(&sidecar) {
            Ok(m) => {
                warnings.push(cut_core::VerbWarning {
                    code: "capture_manifest_auto_discovered".into(),
                    message: format!(
                        "using capture manifest auto-discovered at {} ({} events) — pass capture_manifest explicitly to pin it",
                        sidecar.display(),
                        m.events.len()
                    ),
                    detail: Default::default(),
                });
                return Ok((Some((sidecar, m)), warnings));
            }
            Err(e) => {
                warnings.push(cut_core::VerbWarning {
                    code: "capture_manifest_sidecar_unusable".into(),
                    message: format!(
                        "sidecar manifest at {} exists but is unusable ({}) — importing without it",
                        sidecar.display(),
                        e.message
                    ),
                    detail: Default::default(),
                });
            }
        }
    }
    Ok((None, warnings))
}

/// media.import{path, capture_manifest?, rationale?} — registers the asset as
/// ONE op (the append-only operation-log contract), then kicks the async probe→proxy→transcribe→perception
/// chain as a job. A capture manifest (explicit or `<stem>.capture.json`
/// auto-discovered) is parsed NOW (fail fast) and its events become
/// `capture:<type>` markers inside the chain, right after auto-place (the capture-manifest contract).
pub(crate) async fn media_import(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    // Keep project selection stable from source admission through the import
    // op and its tracked background job registration.
    let expected_project = {
        let project = state.project.read().await;
        match project.as_ref() {
            Some(store) => Some((store.dir.clone(), path_free_project_identity(store)?)),
            None => None,
        }
    };
    let had_project = expected_project.is_some();
    let _project_transition = state.project_transition.lock().await;
    if let Some((dir, identity)) = expected_project {
        let project = state.project.read().await;
        let store = project.as_ref().ok_or_else(no_project)?;
        if store.dir != dir || path_free_project_identity(store)? != identity {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "the open project changed before media import started",
                "retry the import against the current project",
            ));
        }
    }
    media_import_admitted(state, args, actor, had_project).await
}

/// Internal callers that already own the project transition use the same
/// admission body without recursively locking the non-reentrant gate.
pub(super) async fn media_import_under_transition(
    state: &AppState,
    args: Value,
    actor: Actor,
    _transition: &tokio::sync::MutexGuard<'_, ()>,
) -> Result<VerbResult, CutError> {
    media_import_admitted(state, args, actor, true).await
}

async fn media_import_admitted(
    state: &AppState,
    args: Value,
    actor: Actor,
    had_project: bool,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        path: String,
        capture_manifest: Option<String>,
        /// Generate the editing proxy in the import chain (default true). Set false
        /// for heavy files (large FHD / multi-GB raw) to import INSTANTLY — the asset
        /// stays usable (transcript + composed-frame scrub from source) and the final
        /// render uses the source, so output quality is unaffected.
        proxy: Option<bool>,
        /// Internal caller guard. Public dispatch rejects this field through the
        /// schema; background orchestration uses it to bind a delayed import to
        /// the project that submitted the work.
        expected_project_dir: Option<String>,
        /// Internal caller guard for attested connector artifacts. Public
        /// dispatch rejects this field through the schema; trusted orchestration
        /// calls this handler directly after verifying an artifact descriptor.
        expected_sha256: Option<String>,
        expected_byte_length: Option<u64>,
    }
    let a: Args = parse_args(args.clone())?;
    let src = PathBuf::from(&a.path);
    if src.is_dir() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("{} is a folder, not a media file", src.display()),
            "media.import takes one file",
        )
        .with_suggested_action(
            "import the video, audio or image files inside the folder one by one",
        ));
    }
    if !src.is_file() {
        return Err(media_path_error(&src));
    }
    let src = src.canonicalize()?;
    // An empty file can never be media: refuse it now, before anything is
    // registered. Everything else is judged by the probe in the import job.
    if std::fs::metadata(&src).is_ok_and(|m| m.len() == 0) {
        return Err(cut_media::probe::empty_media_error(&src));
    }
    let (manifest, warnings) = resolve_capture_manifest(&src, a.capture_manifest.as_deref())?;
    let hash = if let Some(expected_sha256) = a.expected_sha256.as_deref() {
        let expected_byte_length = a.expected_byte_length.ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "internal expected_byte_length guard is missing",
                "attested imports require both SHA-256 and byte-length evidence",
            )
        })?;
        verify_attested_media_source(&src, expected_sha256, expected_byte_length)?.1
    } else {
        cut_core::hash_file(&src)? // sha256/sample hash — cache key for normal imports
    };
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    // Core's record_import is the ONLY valid import path: replay requires
    // its exact {asset_id, asset} effect payload (a hand-rolled op here once
    // made every diff/replay fail with "import effect payload missing").
    let asset = cut_core::Asset {
        path: src.display().to_string(),
        hash: hash.clone(),
        probe: None,
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    };
    if !had_project {
        return Err(no_project());
    }
    {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        if let Some(expected) = a.expected_project_dir.as_deref() {
            if store.dir != Path::new(expected) {
                return Err(CutError::new(
                    error_codes::CONFLICT,
                    "the project changed before generated media could be imported",
                    format!(
                        "import belongs to {}; the open project is {}",
                        expected,
                        store.dir.display()
                    ),
                ));
            }
        }
    }
    // Attestation and project-owner guards keep their existing refusals. Decode
    // off the project lock, but before an import op can make the picture visible.
    let picture = src.clone();
    run_blocking("media.import.picture", move || {
        cut_media::picture_decode::check_if_picture(&picture)
    })
    .await?;
    let (asset_id, op) = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().ok_or_else(no_project)?;
        // A person who chose no proxy keeps that choice: a later open does not make one.
        let choice = if a.proxy == Some(false) {
            json!({"proxy": false})
        } else {
            Value::Null
        };
        guard_call("media.import", || {
            store.record_import_with(None, asset, actor, rationale, choice)
        })?
    };
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    #[cfg(test)]
    wait_for_media_import_transition_gate(&src).await;
    // Kick the chain job. Result contract (schema/verbs.json):
    // {asset_id, job_id}; the op record rides along for the UI.
    let manifest_meta = manifest
        .as_ref()
        .map(|(p, m)| json!({"path": p.display().to_string(), "events": m.events.len()}));
    let job = spawn_import_chain(
        state.clone(),
        asset_id.clone(),
        src.clone(),
        hash,
        manifest,
        a.proxy.unwrap_or(true),
        true,
    );
    #[cfg(test)]
    wait_for_media_import_admission_gate(&src).await;
    // Keep the shared result shape. New import records have no legacy inverse.
    let mut result = json!({"asset_id": asset_id, "job_id": job, "op": op_for_result(&op, wants_legacy_inverse(&args))});
    if let Some(meta) = manifest_meta {
        result["capture_manifest"] = meta; // markers land async — see job result
    }
    Ok(VerbResult::ok_with_ops(result, vec![op_id]).with_warnings(warnings))
}

pub(crate) fn verify_attested_media_source(
    path: &Path,
    expected_sha256: &str,
    expected_byte_length: u64,
) -> Result<(PathBuf, String), CutError> {
    if expected_sha256.len() != 64
        || !expected_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "internal expected_sha256 guard is invalid",
            "expected a lowercase 64-character SHA-256 digest",
        ));
    }
    let canonical = path.canonicalize()?;
    let (actual_len, actual_sha256) = hash_file_exact(&canonical)?;
    if expected_byte_length != actual_len || actual_sha256 != expected_sha256 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "attested media changed before Cut imported it",
            format!(
                "expected {} bytes / {}, got {} bytes / {}",
                expected_byte_length, expected_sha256, actual_len, actual_sha256
            ),
        )
        .with_suggested_action(
            "Re-render the connector handoff and apply its unchanged artifact handle",
        ));
    }
    Ok((canonical, format!("sha256:{actual_sha256}")))
}

fn hash_file_exact(path: &Path) -> Result<(u64, String), CutError> {
    use sha2::{Digest, Sha256};

    let path_before = std::fs::symlink_metadata(path)?;
    if !path_before.file_type().is_file() || path_before.file_type().is_symlink() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "attested media is not a regular file",
            path.display().to_string(),
        ));
    }
    let mut file = std::fs::File::open(path)?;
    let before = file.metadata()?;
    if !same_media_file_identity(&path_before, &before) {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "attested media changed before Cut opened it",
            path.display().to_string(),
        ));
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    let mut total = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        hasher.update(&buffer[..read]);
    }
    let after = file.metadata()?;
    let path_after = std::fs::symlink_metadata(path)?;
    if total != before.len()
        || !same_media_file_identity(&before, &after)
        || !same_media_file_identity(&before, &path_after)
    {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "media changed while Cut was hashing it",
            path.display().to_string(),
        )
        .with_suggested_action("Wait for the file writer to finish, then retry the import"));
    }
    Ok((total, format!("{:x}", hasher.finalize())))
}

fn same_media_file_identity(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    if !left.file_type().is_file()
        || !right.file_type().is_file()
        || left.len() != right.len()
        || left.modified().ok() != right.modified().ok()
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev() && left.ino() == right.ino()
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// media.remove{asset} — drop an imported asset from the OPEN project: remove its
/// `assets[<id>]` record (replay-safe, via a recorded media.remove op so it stays
/// removed across revert/rebuild) and unlink its derived proxy / filmstrip /
/// transcript / perception files + probe receipt under the .cutproj (all
/// regenerable). The linked SOURCE media file on disk is NEVER touched.
///
/// SAFE DEFAULT: refuses (CONFLICT) while any timeline clip still references the
/// asset — the caller deletes those clips first (that timeline delete IS undoable;
/// asset removal is NOT — re-import to restore). This is the "delete files" half
/// of the cleanup workflow (project.delete is the "delete projects" half). Requires an
/// open project. User-imported sources are NEVER touched. Project-owned generated
/// sources under assets/generated are deleted with their provenance sidecar after
/// the last asset reference is removed. Result: {removed, asset, source_kept,
/// source_deleted, freed[]}.
pub(super) async fn media_remove(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        asset: String,
    }
    let a: Args = parse_args(args.clone())?;
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    // Source-changing/removing mutations take this lease before their project
    // write. A deterministic cache rebuild holds it exclusively from durable
    // reservation through metadata publication, so it can never publish an
    // old source after this mutation commits.
    let _cache_lease = state.cache_lifecycle_lease.write().await;
    let (removed, op, dir, source_deleted, mut freed) = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().ok_or_else(no_project)?;
        if !store.project.assets.contains_key(&a.asset) {
            return Err(CutError::new(
                error_codes::NOT_FOUND,
                format!("no asset '{}'", a.asset),
                "list assets via project.state (project.assets)".to_string(),
            ));
        }
        // SAFE DEFAULT: refuse while any timeline clip still references this asset —
        // removing it would orphan those clips (and the rebuild would have no asset to
        // resolve them against). The UI's linked-delete clears the clips cleanly first.
        let used_by = store
            .project
            .all_sequence_tracks()
            .flat_map(|t| &t.clips)
            .filter(|c| matches!(c, cut_core::Clip::Media(m) if m.asset == a.asset))
            .count();
        if used_by > 0 {
            let scope = if store.project.sequences.is_empty() {
                "timeline"
            } else {
                "project sequence"
            };
            return Err(CutError::new(
                error_codes::CONFLICT,
                format!("asset '{}' is still used by {used_by} {scope} clip(s)", a.asset),
                format!("{used_by} clip(s) reference it; removing the asset would orphan them"),
            )
            .with_suggested_action(
                "delete those clips from every sequence first (switch sequences from the topbar; clip deletion is undoable), then media.remove",
            ));
        }
        // Capture the derived-file roots before the mutating borrow.
        let dir = store.dir.clone();
        // record_remove_asset drops the record AND commits the replay-safe op (which
        // appends to ops.jsonl + saves project.json). It returns the removed Asset so
        // we can unlink its derived files; it NEVER deletes `removed.path` (the source).
        let (removed, op) = guard_call("media.remove", || {
            store.record_remove_asset(&a.asset, actor, rationale)
        })?;
        // Generated media is project-owned, unlike a normal user import. Delete only
        // a DIRECT child of the canonical assets/generated directory and only when no
        // remaining asset points at the same source. This containment fence prevents a
        // crafted project path/symlink from turning media.remove into arbitrary unlink.
        let mut source_deleted = false;
        let mut freed: Vec<String> = Vec::new();
        let source = PathBuf::from(&removed.path);
        let generated_root = crate::output_paths::existing_plain_project_relative_dir(
            &dir,
            Path::new("assets/generated"),
        );
        let still_referenced = store
            .project
            .assets
            .values()
            .any(|asset| Path::new(&asset.path) == source);
        if !still_referenced {
            if let Ok(root) = generated_root {
                if source.parent() == Some(root.as_path()) && plain_generated_file(&source) {
                    let sidecar = source.with_extension("json");
                    if std::fs::remove_file(&source).is_ok() {
                        source_deleted = true;
                        freed.push(
                            source
                                .strip_prefix(&dir)
                                .unwrap_or(&source)
                                .display()
                                .to_string(),
                        );
                        if plain_generated_file(&sidecar) && std::fs::remove_file(&sidecar).is_ok()
                        {
                            freed.push(
                                sidecar
                                    .strip_prefix(&dir)
                                    .unwrap_or(&sidecar)
                                    .display()
                                    .to_string(),
                            );
                        }
                    }
                }
            }
        }
        (removed, op, dir, source_deleted, freed)
    };
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    // We keep the mutation lease through exact output retirement. The project
    // guard above is dropped before filesystem work, while no producer can
    // publish into these names between the committed mutation and cleanup.
    let mut warnings = Vec::new();
    remove_owned_cache_outputs_locked(
        &dir,
        &a.asset,
        [
            (
                crate::cache_lifecycle::CacheKind::Proxies,
                removed.proxy.clone(),
            ),
            (
                crate::cache_lifecycle::CacheKind::Thumbnails,
                removed.filmstrip.clone(),
            ),
        ]
        .into_iter()
        .chain(pending_rebuild_cleanup_outputs(
            &dir,
            &a.asset,
            &mut warnings,
        )),
        &mut freed,
        &mut warnings,
    );
    remove_asset_receipts(
        &dir,
        &a.asset,
        removed.transcript.as_deref(),
        removed.perception.as_deref(),
        &mut freed,
        &mut warnings,
    );
    Ok(VerbResult::ok_with_ops(
        json!({
            "removed": true,
            "asset": a.asset,
            "source_kept": if source_deleted { Value::Null } else { json!(removed.path) },
            "source_deleted": source_deleted,
            "freed": freed,
            "op": op_for_result(&op, wants_legacy_inverse(&args)),
        }),
        vec![op_id],
    )
    .with_warnings(warnings))
}

/// media.relink{asset, path} — repoint an imported asset at a new source file,
/// the recovery verb for OFFLINE media (moved/renamed/restored-elsewhere files).
/// Two modes decided by content hash:
///   * SAME hash  ⇒ the file merely moved — pure repath; probe/proxy/filmstrip/
///     transcript/perception all still describe the content and are KEPT.
///   * NEW hash   ⇒ different content — derived state is cleared and regenerated
///     via the same import chain a fresh media.import runs (probe → proxy →
///     filmstrip …), so the asset behaves exactly as if imported anew while every
///     timeline clip keeps referencing the same asset id (no re-editing needed).
/// Refuses a KIND mismatch (relinking a video asset to an audio file would break
/// every clip's geometry); WARNS (not refuses) when the new file is shorter than
/// the furthest source point used by clips — the agent can then re-trim.
/// Replay-safe metadata op (store::record_relink_asset), off the undo cursor.
/// Result: {asset, path, old_path, hash_changed, derived_cleared, freed[], job_id?}.
pub(super) async fn media_relink(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    // Hash/probe may yield before the relink op, and a changed hash starts an
    // import job after the op. Both must stay attached to this project.
    let _project_transition = pin_media_project(state).await?;
    #[derive(serde::Deserialize)]
    struct Args {
        asset: String,
        path: String,
    }
    let a: Args = parse_args(args.clone())?;
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let src = PathBuf::from(&a.path);
    if !src.is_file() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("relink target not found: {}", src.display()),
            "path must point at an existing readable file",
        )
        .with_suggested_action("pass an absolute path to the new source media file"));
    }
    let src = src.canonicalize()?;
    let new_hash = cut_core::hash_file(&src)?;
    // Probe the NEW file up front (off the lock): kind guard + duration warning.
    let s = src.clone();
    // A target that is not readable media is refused with the same plain
    // `unreadable_media` error as an import; the asset keeps its old source.
    let new_probe = run_blocking("media.relink.probe", move || cut_media::probe_source(&s)).await?;
    // See media.remove: acquire before committing the source change so a cache
    // rebuild cannot race from old input bytes into a new asset revision.
    let _cache_lease = state.cache_lifecycle_lease.write().await;
    let (old, op, mut warnings, dir) = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().ok_or_else(no_project)?;
        let old = store.project.assets.get(&a.asset).cloned().ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("no asset '{}'", a.asset),
                "list assets via project.state (project.assets)".to_string(),
            )
        })?;
        // KIND GUARD: only when the old kind is known (probed). video→video,
        // audio→audio, image→image; anything else breaks clip geometry/audio.
        if let Some(old_kind) = old
            .probe
            .as_ref()
            .and_then(|p| p.get("kind"))
            .and_then(|k| k.as_str())
        {
            if old_kind != new_probe.kind {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!(
                        "kind mismatch: asset '{}' is {old_kind}, relink target is {}",
                        a.asset, new_probe.kind
                    ),
                    "clips referencing the asset assume its kind",
                )
                .with_suggested_action(
                    "import the file as a NEW asset (media.import) and swap clips via edit.replace",
                ));
            }
        }
        // DURATION WARNING: find the furthest source point any clip uses.
        let max_used = store
            .project
            .all_sequence_tracks()
            .flat_map(|t| &t.clips)
            .filter_map(|c| match c {
                cut_core::Clip::Media(m) if m.asset == a.asset => {
                    Some((m.id.clone(), m.src_out_ms))
                }
                _ => None,
            })
            .max_by_key(|(_, out)| *out);
        let mut warnings: Vec<cut_core::VerbWarning> = Vec::new();
        if let (Some((clip_id, used_ms)), Some(new_dur)) = (&max_used, new_probe.duration_ms) {
            if new_dur < *used_ms {
                warnings.push(cut_core::VerbWarning {
                    code: "relink_shorter_than_used".into(),
                    message: format!(
                        "new file is {}ms long but clip '{}' uses source up to {}ms — \
                         re-trim affected clips or renders will come up short",
                        new_dur, clip_id, used_ms
                    ),
                    detail: Default::default(),
                });
            }
        }
        let hash_changed = new_hash != old.hash;
        let (_, op) = guard_call("media.relink", || {
            store.record_relink_asset(
                &a.asset,
                &src.display().to_string(),
                &new_hash,
                hash_changed,
                actor,
                rationale,
            )
        })?;
        (old, op, warnings, store.dir.clone())
    };
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    let hash_changed = new_hash != old.hash;
    let mut freed: Vec<String> = Vec::new();
    let mut job_id: Option<String> = None;
    if hash_changed {
        // The project mutation committed above and released its write guard.
        // Cache cleanup now takes the shared lifecycle lease, never in the
        // inverse order. Only exact ledger-owned proxy/filmstrip files may be
        // removed; a stale ledger stays visible as a warning and blocks later
        // cache inventory rather than deleting another asset's output.
        remove_owned_cache_outputs_locked(
            &dir,
            &a.asset,
            [
                (
                    crate::cache_lifecycle::CacheKind::Proxies,
                    old.proxy.clone(),
                ),
                (
                    crate::cache_lifecycle::CacheKind::Thumbnails,
                    old.filmstrip.clone(),
                ),
            ]
            .into_iter()
            .chain(pending_rebuild_cleanup_outputs(
                &dir,
                &a.asset,
                &mut warnings,
            )),
            &mut freed,
            &mut warnings,
        );
        remove_asset_receipts(
            &dir,
            &a.asset,
            old.transcript.as_deref(),
            old.perception.as_deref(),
            &mut freed,
            &mut warnings,
        );
        job_id = Some(spawn_import_chain(
            state.clone(),
            a.asset.clone(),
            src.clone(),
            new_hash.clone(),
            None,
            old.proxy.is_some(), // rebuild a proxy only if the asset had one
            false,               // relinking an existing asset must never auto-place a new clip
        ));
    }
    let mut result = json!({
        "asset": a.asset,
        "path": src.display().to_string(),
        "old_path": old.path,
        "hash_changed": hash_changed,
        "derived_cleared": hash_changed,
        "freed": freed,
        "op": op_for_result(&op, wants_legacy_inverse(&args)),
    });
    if let Some(j) = &job_id {
        result["job_id"] = json!(j);
    }
    Ok(VerbResult::ok_with_ops(result, vec![op_id]).with_warnings(warnings))
}

/// An asset's stored path as an absolute path. A relative path in a project
/// is relative to the project folder (the renderer reads it that way too); it
/// is never handed to a tool as is, where a name such as `-report` would be
/// read as an option and other names would resolve against the engine's own
/// working folder.
pub(crate) fn asset_source_path(project_dir: &Path, asset: &cut_core::Asset) -> PathBuf {
    let p = PathBuf::from(&asset.path);
    if p.is_relative() {
        project_dir.join(p)
    } else {
        p
    }
}

fn source_modified_ms(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let duration = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(duration.as_millis()).ok()
}

/// Does `asset` match a smart bin's query? AND-combined: kind (probe kind),
/// text (case-insensitive substring on the source basename), unused (timeline
/// reference count == 0), source resolution, offline/online state, and source
/// file modification time. An asset with no probe yet fails kind/resolution
/// filters (unknown ≠ match — honest until probed).
fn smart_bin_matches(
    bin: &cut_core::SmartBin,
    asset: &cut_core::Asset,
    referenced: usize,
    exists: bool,
    modified_ms: Option<i64>,
) -> bool {
    if let Some(want) = &bin.kind {
        let kind = asset
            .probe
            .as_ref()
            .and_then(|p| p.get("kind"))
            .and_then(|k| k.as_str());
        if kind != Some(want.as_str()) {
            return false;
        }
    }
    if let Some(text) = &bin.text {
        let base = asset
            .path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(asset.path.as_str())
            .to_lowercase();
        if !base.contains(&text.to_lowercase()) {
            return false;
        }
    }
    if let Some(unused) = bin.unused {
        if unused != (referenced == 0) {
            return false;
        }
    }
    if let Some(min_width) = bin.min_width {
        let width = asset
            .probe
            .as_ref()
            .and_then(|p| p.get("width"))
            .and_then(|w| w.as_u64());
        match width {
            Some(width) if width >= u64::from(min_width) => {}
            _ => return false,
        }
    }
    if let Some(min_height) = bin.min_height {
        let height = asset
            .probe
            .as_ref()
            .and_then(|p| p.get("height"))
            .and_then(|h| h.as_u64());
        match height {
            Some(height) if height >= u64::from(min_height) => {}
            _ => return false,
        }
    }
    if let Some(offline) = bin.offline {
        let is_offline = !exists;
        if offline != is_offline {
            return false;
        }
    }
    if let Some(after) = bin.modified_after_ms {
        match modified_ms {
            Some(ms) if ms >= after => {}
            _ => return false,
        }
    }
    if let Some(before) = bin.modified_before_ms {
        match modified_ms {
            Some(ms) if ms <= before => {}
            _ => return false,
        }
    }
    true
}

/// media.bin_save{name, kind?, text?, unused?, min_width?, min_height?,
/// offline?, modified_after_ms?, modified_before_ms?} — save (upsert) a SMART
/// BIN, a named saved search over the asset tray (the
/// saved-bin convention). Membership is computed at list time
/// (media.bin_list), never stored. At least one criterion is required (an
/// everything-bin is refused). Name-keyed: re-save REPLACES. Replay-safe
/// metadata op (grade.save pattern).
pub(super) async fn media_bin_save(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        name: String,
        kind: Option<String>,
        text: Option<String>,
        unused: Option<bool>,
        min_width: Option<u32>,
        min_height: Option<u32>,
        offline: Option<bool>,
        modified_after_ms: Option<i64>,
        modified_before_ms: Option<i64>,
    }
    let a: Args = parse_args(args.clone())?;
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let name = a.name.trim().to_string();
    if name.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "smart bin needs a non-empty name",
            "the name is the bin's key (re-save replaces)",
        ));
    }
    if a.kind.is_none()
        && a.text.is_none()
        && a.unused.is_none()
        && a.min_width.is_none()
        && a.min_height.is_none()
        && a.offline.is_none()
        && a.modified_after_ms.is_none()
        && a.modified_before_ms.is_none()
    {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "smart bin needs at least one criterion",
            "a bin matching everything is just the tray",
        ));
    }
    if let Some(k) = &a.kind {
        if !matches!(k.as_str(), "video" | "audio" | "image") {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown kind '{k}'"),
                "kind must be video | audio | image",
            ));
        }
    }
    if matches!(a.min_width, Some(0)) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "min_width must be greater than zero",
            "omit min_width or pass a positive pixel width",
        ));
    }
    if matches!(a.min_height, Some(0)) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "min_height must be greater than zero",
            "omit min_height or pass a positive pixel height",
        ));
    }
    if let (Some(after), Some(before)) = (a.modified_after_ms, a.modified_before_ms) {
        if after > before {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "modified_after_ms must be before modified_before_ms",
                "pass a valid source-file date range",
            ));
        }
    }
    let bin = cut_core::SmartBin {
        name,
        kind: a.kind,
        text: a.text,
        unused: a.unused,
        min_width: a.min_width,
        min_height: a.min_height,
        offline: a.offline,
        modified_after_ms: a.modified_after_ms,
        modified_before_ms: a.modified_before_ms,
    };
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let (replaced, op) = guard_call("media.bin_save", || {
        store.save_smart_bin(bin.clone(), actor, rationale)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    Ok(VerbResult::ok_with_ops(
        json!({
            "bin": bin,
            "replaced": replaced,
            "op": op_for_result(&op, wants_legacy_inverse(&args)),
        }),
        vec![op_id],
    ))
}

/// media.bin_delete{name} — delete a smart bin (NOT_FOUND if unknown).
pub(super) async fn media_bin_delete(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        name: String,
    }
    let a: Args = parse_args(args.clone())?;
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let op = guard_call("media.bin_delete", || {
        store.delete_smart_bin(&a.name, actor, rationale)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    Ok(VerbResult::ok_with_ops(
        json!({
            "deleted": a.name,
            "op": op_for_result(&op, wants_legacy_inverse(&args)),
        }),
        vec![op_id],
    ))
}

/// media.bin_list{} — read-only: every smart bin with its LIVE membership
/// (matching asset ids, computed against the current tray + timeline usage —
/// never cached, so it can't go stale).
pub(super) async fn media_bin_list(state: &AppState, _args: Value) -> Result<VerbResult, CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let dir = store.dir.clone();
    // asset id → timeline reference count (for the `unused` criterion).
    let mut refs: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for t in store.project.all_sequence_tracks() {
        for c in &t.clips {
            if let cut_core::Clip::Media(m) = c {
                *refs.entry(m.asset.as_str()).or_default() += 1;
            }
        }
    }
    let bins: Vec<Value> = store
        .project
        .smart_bins
        .iter()
        .map(|bin| {
            let matches: Vec<&String> = store
                .project
                .assets
                .iter()
                .filter(|(id, a)| {
                    let source = asset_source_path(&dir, a);
                    let exists = source.is_file();
                    let modified_ms = if exists {
                        source_modified_ms(&source)
                    } else {
                        None
                    };
                    smart_bin_matches(
                        bin,
                        a,
                        refs.get(id.as_str()).copied().unwrap_or(0),
                        exists,
                        modified_ms,
                    )
                })
                .map(|(id, _)| id)
                .collect();
            json!({
                "name": bin.name,
                "kind": bin.kind,
                "text": bin.text,
                "unused": bin.unused,
                "min_width": bin.min_width,
                "min_height": bin.min_height,
                "offline": bin.offline,
                "modified_after_ms": bin.modified_after_ms,
                "modified_before_ms": bin.modified_before_ms,
                "matches": matches,
                "match_count": matches.len(),
            })
        })
        .collect();
    Ok(VerbResult::ok(json!({
        "count": bins.len(),
        "bins": bins,
    })))
}

/// media.filmstrip{asset} — (re)build the timeline thumbnail strip for a video
/// asset from its proxy, the "frames in the time bar" layer. Idempotent (returns
/// the existing strip if present). For assets imported before the feature, or to
/// build one on demand. Video assets WITH a proxy only; returns {filmstrip}.
pub(super) async fn media_filmstrip(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    // The generated file and asset field must belong to the same project as
    // the source/proxy selected before the blocking thumbnail operation.
    let _project_transition = pin_media_project(state).await?;
    #[derive(serde::Deserialize)]
    struct Args {
        asset: String,
        /// WINDOWED (zoom) mode: `[start_ms, end_ms)` in the asset's own SOURCE
        /// time. When present, sample just this window (the visible clip range at
        /// the current zoom) instead of the whole asset — the per-zoom density
        /// path. Absent = the whole-asset base strip (stored on the asset).
        range_ms: Option<[u64; 2]>,
        /// Windowed mode: how many frames to tile across the window (≈1 per fixed
        /// pixel width of the visible clip). Clamped server-side. Default 12.
        count: Option<u32>,
        /// Thumbnail height in px (windowed mode). Default 80.
        h: Option<u32>,
    }
    let a: Args = parse_args(args)?;
    let (dir, _receipts, _proxies) = project_paths(state).await?;
    // Read the asset's kind/duration/path (from its probe) under a short read lock.
    let (kind, dur, has_proxy) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let asset = store.project.assets.get(&a.asset).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("no asset '{}'", a.asset),
                "asset ids come from project.state",
            )
        })?;
        let probe = asset.probe.as_ref();
        let kind = probe
            .and_then(|p| p.get("kind"))
            .and_then(|k| k.as_str())
            .unwrap_or("")
            .to_string();
        let dur = probe
            .and_then(|p| p.get("duration_ms"))
            .and_then(|d| d.as_u64())
            .unwrap_or(0);
        (kind, dur, asset.proxy.is_some())
    };
    let film_dir = dir.join("filmstrip");
    // A cleanup job takes this same lease exclusively. Hold it across output
    // publication, durable ownership recording, and asset write-back so a
    // cache plan can never race a cooperating filmstrip producer.
    let _cache_write = state.cache_lifecycle_lease.read().await;
    let identity =
        crate::cache_lifecycle::CacheSource::capture(state, &dir, &a.asset, None).await?;
    let binding = identity.binding(crate::cache_lifecycle::CacheKind::Thumbnails);

    // WINDOWED (zoom) mode — sample just the requested source window. Video only
    // (an image is one frame at every zoom); ephemeral (NOT stored on the asset),
    // returns `{thumbs, range_ms, count, h}`. The tile lives in the same
    // `filmstrip/` dir → served by the existing `/filmstrip/:file` route.
    if let Some([mut t0, mut t1]) = a.range_ms {
        if kind != "video" {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "windowed thumbnails (range_ms) are for video assets only",
                format!(
                    "asset '{}' is kind={kind} — a still image is one frame at any zoom",
                    a.asset
                ),
            ));
        }
        if !has_proxy || dur == 0 {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "asset has no proxy/duration yet",
                "wait for the import chain's proxy step, then retry",
            ));
        }
        // Clamp the window into the asset's real source span; keep it non-empty.
        t1 = t1.min(dur);
        t0 = t0.min(t1.saturating_sub(1));
        if t1 <= t0 {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("empty window after clamping to asset duration {dur}ms"),
                "pass range_ms within [0, asset duration) with end > start",
            ));
        }
        let count = a.count.unwrap_or(12);
        let h = a.h.unwrap_or(80);
        let aid = a.asset.clone();
        let proxy_path = identity.proxy_input(&dir).await?;
        let path = run_blocking("media.filmstrip.window", move || {
            cut_media::filmstrip::make_window_thumbs_bound(
                &proxy_path,
                &film_dir,
                &aid,
                t0,
                t1,
                count,
                h,
                &binding,
            )
        })
        .await?;
        let rel = format!(
            "filmstrip/{}",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        identity
            .record(
                state,
                &dir,
                crate::cache_lifecycle::CacheKind::Thumbnails,
                &path,
            )
            .await?;
        return Ok(VerbResult::ok(json!({
            "thumbs": rel,
            "range_ms": [t0, t1],
            "count": count.clamp(12, 160),
            "h": h.clamp(24, 240),
        })));
    }

    let aid = a.asset.clone();
    // Images: a single-frame thumbnail from the source (no proxy/duration needed).
    let path = if kind == "image" {
        let src = identity.source.clone();
        run_blocking("media.image_thumb", move || {
            cut_media::filmstrip::make_image_thumb_bound(&src, &film_dir, &aid, &binding)
        })
        .await?
    } else if kind == "video" {
        if !has_proxy || dur == 0 {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "asset has no proxy/duration yet",
                "wait for the import chain's proxy step, then retry",
            ));
        }
        let proxy_path = identity.proxy_input(&dir).await?;
        run_blocking("media.filmstrip", move || {
            cut_media::filmstrip::make_filmstrip_bound(&proxy_path, &film_dir, &aid, dur, &binding)
        })
        .await?
    } else {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "filmstrip is for video or image assets",
            format!("asset '{}' is kind={kind}", a.asset),
        ));
    };
    let rel = format!(
        "filmstrip/{}",
        path.file_name().unwrap_or_default().to_string_lossy()
    );
    identity
        .record(
            state,
            &dir,
            crate::cache_lifecycle::CacheKind::Thumbnails,
            &path,
        )
        .await?;
    update_asset(state, &a.asset, |asset| asset.filmstrip = Some(rel.clone())).await?;
    Ok(VerbResult::ok(json!({ "filmstrip": rel })))
}

/// Spawn the probe→proxy→transcribe→perception chain (server contract). Each step is
/// todo-tolerant: a step failure fails the JOB with the step named, but keeps
/// every earlier step's persisted result (partial enrichment is still value).
/// `manifest` = the resolved capture manifest (the capture-manifest contract): its events become
/// `capture:<type>` marker ops right after auto-place (markers are timeline-
/// absolute, so they are only created when THIS import became the timeline).
pub(super) fn spawn_import_chain(
    state: AppState,
    asset_id: String,
    src: PathBuf,
    hash: String,
    manifest: Option<(PathBuf, CaptureManifest)>,
    make_proxy: bool,
    allow_auto_place: bool,
) -> String {
    let job = state.jobs.create("import_chain");
    let job_id = job.job_id.clone();
    if let Some(error) = job.persistence_error {
        state.jobs.fail(
            &job_id,
            CutError::new(
                error_codes::IO,
                "Media analysis did not start",
                format!("The analysis job could not be saved: {error}"),
            )
            .with_suggested_action(
                "Free space or restore write access to the project folder, then reopen the project to retry analysis.",
            ),
        );
        return job_id;
    }
    let jid = job_id.clone();
    let jobs = state.jobs.clone();
    // Take this import's place in line NOW, in the order imports were requested:
    // the placement decision below is made in that order whatever order the
    // probes finish in, so the first import requested is the one that becomes the
    // timeline.
    let mut placement_turn = allow_auto_place.then(|| state.placement_turns.take());
    jobs.spawn(&job_id, async move {
        let (dir, receipts, proxies) = match project_paths(&state).await {
            Ok(p) => p,
            Err(e) => return state.jobs.fail(&jid, e),
        };
        // 1) probe — duration/geometry facts the editor needs first.
        state.jobs.progress(&jid, 0.05, Some("probe".into()));
        let s = src.clone();
        #[cfg(test)]
        placement_turn::probe_delay(&src).await;
        let probed = match run_blocking("media.probe", move || cut_media::probe_source(&s)).await {
            Ok(probe) => {
                let pv = serde_json::to_value(&probe).unwrap_or(Value::Null);
                persist_probe_file(&state, &asset_id, &pv).await; // survives rebuild
                if let Err(e) = update_asset(&state, &asset_id, |a| a.probe = Some(pv)).await {
                    return state.jobs.fail(
                        &jid,
                        e.with_suggested_action(
                            "probe succeeded but project state could not be updated; retry media.import for the asset",
                        ),
                    );
                }
                mark_probe_verified(&state, &asset_id).await;
                probe
            }
            Err(e) => {
                // No facts, so no usable asset: undo the registration whatever
                // the reason (the tool is missing, or the file is not media),
                // so nothing half-imported stays in Assets. The error keeps its
                // own plain message and next step (`tool_missing` names the
                // tool; `unreadable_media` names the file and what to do).
                let undone =
                    unprobed_import::undo_unprobed_import(&state, &dir, &asset_id, &e).await;
                let cause = if undone {
                    format!("{}; the import was undone", e.cause)
                } else {
                    e.cause.clone()
                };
                let suggested_action = e
                    .suggested_action
                    .clone()
                    .or_else(|| Some("Try the import again; if it fails again, check the file in a media player.".into()));
                return state.jobs.fail(
                    &jid,
                    CutError {
                        cause,
                        suggested_action,
                        ..e
                    },
                );
            }
        };
        // Still images: probe is the whole chain — there is no
        // audio to transcribe/analyze and a one-frame proxy is useless. The
        // job FINISHES (not fails) with the skipped steps named so the agent
        // knows the asset is ready for edit.insert{duration_ms}.
        if probed.kind == "image" {
            // A still is never auto-placed: its decision is made, so the imports
            // behind it need not wait for its thumbnail.
            if let Some(turn) = placement_turn.as_mut() {
                turn.finish();
            }
            // Thumbnail for the timeline clip so the picture is visible (not just a
            // photo icon) — reuses the `filmstrip` field/dir. Non-fatal.
            let film_dir = dir.join("filmstrip");
            let (s, fd, aid) = (src.clone(), film_dir, asset_id.clone());
            let _cache_write = state.cache_lifecycle_lease.read().await;
            let identity = match crate::cache_lifecycle::CacheSource::capture(&state, &dir, &asset_id, Some((&src, &hash))).await {
                Ok(identity) => identity,
                Err(error) => return state.jobs.fail(&jid, error),
            };
            let binding = identity.binding(crate::cache_lifecycle::CacheKind::Thumbnails);
            let thumb = run_blocking("media.image_thumb", move || {
                cut_media::filmstrip::make_image_thumb_bound(&s, &fd, &aid, &binding)
            })
            .await;
            // A thumbnail that could not be made is said, not dropped.
            let thumb_warning = thumb.as_ref().err().map(|e| {
                format!(
                    "no timeline thumbnail: {} (rebuild it with media.filmstrip)",
                    e.message
                )
            });
            if let Ok(path) = thumb {
                let rel = format!(
                    "filmstrip/{}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
                match identity.record(&state, &dir, crate::cache_lifecycle::CacheKind::Thumbnails, &path).await {
                    Ok(()) => {
                        if let Err(e) = update_asset(&state, &asset_id, |a| a.filmstrip = Some(rel)).await
                        {
                            return state.jobs.fail(
                                &jid,
                                e.with_suggested_action(
                                    "image thumbnail was created but project state could not be updated",
                                ),
                            );
                        }
                    }
                    Err(error) => {
                        return state.jobs.fail(
                            &jid,
                            error.with_suggested_action(
                                "the thumbnail was left unowned; rebuild it with media.filmstrip after the cache issue is resolved",
                            ),
                        );
                    }
                }
            }
            state
                .jobs
                .progress(&jid, 1.0, Some("done (still image)".into()));
            let mut result = json!({
                "asset": asset_id,
                "kind": "image",
                "skipped_steps": ["proxy", "transcribe", "perception"],
                "placed": false,
                "placement": {
                    "placed": false,
                    "reason": "A still image is never placed on the timeline automatically. Place it with edit.insert and a duration_ms.",
                },
                "note": "still image: no intrinsic duration/audio — place it with edit.insert{asset, track, at_ms, duration_ms}; the render loops the still for the clip duration",
            });
            if manifest.is_some() {
                // Stills are never auto-placed, and capture markers are
                // timeline-absolute — nothing sound to anchor them to.
                result["capture_manifest"] = json!({
                    "markers_created": 0,
                    "note": "manifest parsed but stills are never auto-placed — capture markers (timeline-absolute) were not created",
                });
            }
            if let Some(warning) = thumb_warning {
                result["warnings"] = json!([warning]);
                return state.jobs.finish_with_warnings(&jid, result);
            }
            return state.jobs.finish(&jid, result);
        }
        // 1b) auto-place: when the timeline holds NO media clips yet, the
        // first import BECOMES the timeline, so the documented workflow can go
        // straight from import to transcript editing (the
        // talking-head wedge). Placement is real edit.insert ops with the
        // system actor (every mutation is a verb-shaped op;
        // the review rail shows them). Later imports (b-roll) are NOT placed
        // — they wait for an explicit edit.insert. The src range is resolved
        // from the just-persisted probe and recorded EXPLICITLY on each op:
        // logged ops must be self-contained (probe write-back is cache, not
        // an op, so replay cannot consult it).
        let mut manifest_summary = Value::Null; // filled by the capture-manifest contract ingestion below
        let mut format_record = Value::Null; // the first-clip format adoption outcome
        let mut format_warning: Option<String> = None;
        let placement: Value;
        {
            // Every import requested before this one has decided first: that is
            // what makes "the first import" the first one asked for.
            if let Some(turn) = placement_turn.as_ref() {
                turn.wait().await;
            }
            // Why nothing was placed, when nothing was: the job result says so.
            let mut declined: Option<&'static str> = None;
            let targets: Vec<(String, [u64; 2])> = {
                let guard = state.project.read().await;
                match guard.as_ref() {
                    None => {
                        declined = Some("No project is open, so nothing was placed.");
                        vec![]
                    }
                    Some(store) => {
                        let any_media_clips = !allow_auto_place || store.project.tracks.iter().any(|t| {
                            matches!(
                                t.kind,
                                cut_core::TrackKind::Video | cut_core::TrackKind::Audio
                            ) && !t.clips.is_empty()
                        });
                        let probe = store
                            .project
                            .assets
                            .get(&asset_id)
                            .and_then(|a| a.probe.clone())
                            .unwrap_or(Value::Null);
                        let duration = probe.get("duration_ms").and_then(|v| v.as_u64());
                        match (any_media_clips, duration) {
                            (true, _) => {
                                declined = Some(if allow_auto_place {
                                    "The timeline already has media, so this import waits in Assets. Place it with edit.insert."
                                } else {
                                    "This kind of import is never placed automatically. Place it with edit.insert."
                                });
                                vec![]
                            }
                            (_, None) => {
                                declined = Some("The file has no readable duration, so it was not placed. Place it with edit.insert and a duration_ms.");
                                vec![]
                            }
                            (false, Some(d)) => {
                                let has_video =
                                    probe.get("width").map(|w| !w.is_null()).unwrap_or(false);
                                let has_audio = probe
                                    .get("has_audio")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false);
                                let first_track = |kind: cut_core::TrackKind| {
                                    store
                                        .project
                                        .tracks
                                        .iter()
                                        .find(|t| t.kind == kind)
                                        .map(|t| (t.id.clone(), [0, d]))
                                };
                                let mut out = Vec::new();
                                if has_video {
                                    out.extend(first_track(cut_core::TrackKind::Video));
                                }
                                if has_audio {
                                    out.extend(first_track(cut_core::TrackKind::Audio));
                                }
                                out
                            }
                        }
                    }
                }
            };
            let placed = !targets.is_empty();
            if !placed && declined.is_none() {
                declined = Some("The project has no video or audio track to place it on.");
            }
            let placed_tracks: Vec<String> = targets.iter().map(|(t, _)| t.clone()).collect();
            for (track, src_range) in targets {
                // ripple: false EXPLICITLY — auto-place puts the SAME source
                // on v1 and a1t at the same position; a base-track default
                // ripple would gap-shift the sibling before its own placement
                // lands (the ripple-sync contract semantics; both tracks start empty + aligned).
                let args = json!({
                    "asset": asset_id, "track": track, "at_ms": 0,
                    "src_range_ms": src_range, "ripple": false,
                    "rationale": "auto-place: first import becomes the timeline",
                });
                if let Err(e) = commit_core(&state, "edit.insert", args, Actor::system()).await {
                    return state.jobs.fail(
                        &jid,
                        e.with_suggested_action(format!(
                            "auto-place of '{asset_id}' onto '{track}' failed — timeline left empty"
                        )),
                    );
                }
            }
            // The decision is made (the clips are on the timeline, or this import
            // stays in Assets): the imports behind this one may decide now.
            if let Some(turn) = placement_turn.as_mut() {
                turn.finish();
            }
            placement = if placed {
                json!({"placed": true, "tracks": placed_tracks})
            } else {
                json!({"placed": false, "reason": declined.unwrap_or("Not placed.")})
            };
            // NEW-TIMELINE-FROM-FILE: when THIS import is the first clip (the
            // timeline was empty, so it was just auto-placed) and the project
            // still has the built-in default format, adopt the file's size and
            // frame rate. An explicit format is never overridden. Every outcome,
            // including a size beyond the frame limit, is reported as `format`.
            if placed && probed.kind == "video" {
                let (record, warning) =
                    format_adoption::adopt_first_clip_format(&state, &probed).await;
                format_record = record;
                format_warning = warning;
            }
            // Capture-manifest ingestion (the capture-manifest contract): events → capture:<type>
            // marker ops (system actor, visible in the review
            // rail) with the full event payload in the note. Only when THIS
            // import was just auto-placed at 0 full-length (media time ==
            // timeline time); otherwise skipped with an honest note. A
            // per-event failure is recorded and the chain continues —
            // markers are enrichment, not a reason to lose proxy/transcribe.
            if let Some((mpath, m)) = &manifest {
                let mname = mpath
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let summary = if placed {
                    state
                        .jobs
                        .progress(&jid, 0.25, Some("capture markers".into()));
                    let anchors = clock_anchors(m);
                    let mut created = 0u64;
                    let mut skipped: Vec<String> = Vec::new();
                    for ev in &m.events {
                        match manifest_event_to_marker(ev, &anchors) {
                            Ok(mut args) => {
                                args["rationale"] =
                                    json!(format!("capture-manifest ingest from {mname}"));
                                match commit_core(&state, "edit.add_marker", args, Actor::system())
                                    .await
                                {
                                    Ok(_) => created += 1,
                                    Err(e) => skipped.push(format!(
                                        "marker op failed: {} ({})",
                                        e.message, e.cause
                                    )),
                                }
                            }
                            Err(why) => skipped.push(why),
                        }
                    }
                    json!({
                        "markers_created": created,
                        "events_skipped": skipped.len(),
                        "skipped": skipped,
                        "note": "markers are confidence-tagged claims from the capture agent (full event payload in each marker note); automated pixel cross-checking is not yet implemented",
                    })
                } else {
                    json!({
                        "markers_created": 0,
                        "note": "asset was not auto-placed (timeline already has content) — capture markers are timeline-absolute and were NOT created; import the take into a fresh project or add markers manually",
                    })
                };
                manifest_summary = summary;
            }
        }
        // 2) proxy + filmstrip moved OUT of the import chain into a BACKGROUND job
        // The import finishes "ready to edit" immediately and the 960x540
        // editing proxy transcodes in the background with live progress. Editing
        // is never blocked: the Preview plays the SOURCE until the proxy lands,
        // then auto-swaps when asset.proxy updates. The final
        // render uses the SOURCE regardless, so output quality is unaffected. This
        // pairs with the transcribe/perception decouple: a multi-GB import no
        // longer sits on a frozen "proxy"/"encoding" step. SKIPPED on proxy:false.
        let proxy_job = if make_proxy {
            Some(spawn_proxy_chain(
                state.clone(),
                asset_id.clone(),
                src.clone(),
                hash.clone(),
                proxies.clone(),
                dir.clone(),
                probed.kind.clone(),
                probed.duration_ms,
            ))
        } else {
            tracing::info!(
                asset_id,
                "proxy generation skipped (media.import proxy:false)"
            );
            None
        };
        // ── READY TO EDIT. probe + auto-place are done, so the clip is on the
        // timeline and the user can cut immediately (proxy + analysis stream in
        // the background). The heavy ANALYSIS — transcribe + the perception
        // battery — is SLOW on long footage
        // and must NEVER block editing: a large file must not leave the import
        // pill FROZEN at 55% (the blocking transcribe step, no sub-progress). So
        // finish the import job HERE and run transcribe+perception as a separate
        // background ENRICH job, NON-FATAL (a missing/slow/failed sidecar degrades
        // to a warning on THAT job — "core editing works without it" — never a
        // failed import; this also closes the cold-install hard-fail gap).
        let enrich_job = spawn_enrich_chain(
            state.clone(),
            asset_id.clone(),
            src.clone(),
            hash.clone(),
            receipts.clone(),
            probed.kind.clone(),
        );
        state.jobs.progress(&jid, 1.0, Some("ready to edit".into()));
        let mut result = json!({
            "asset": asset_id,
            "ready_to_edit": true,
            "enrich_job": enrich_job,
            "proxy_job": proxy_job,
            "readiness": {
                "source": "ready",
                "edit": "ready",
                "proxy": if make_proxy { "pending" } else { "not_requested" },
                "speech": "pending",
                "perception": "pending",
                "optional_services": "not_required",
            },
            "enrichment": "transcript + perception run in the BACKGROUND (poll enrich_job); transcript-dependent verbs (captions/cut_words/remove_fillers) wait for it. The editing proxy builds in the BACKGROUND too (poll proxy_job); the Preview plays the source until it lands.",
        });
        if probed.kind == "audio" {
            result["kind"] = json!("audio");
            result["note"] = json!(
                "audio-only asset: enrichment skips video instruments (scenes/black/frozen); \
                 place it with edit.insert on an audio track"
            );
        }
        result["placed"] = placement["placed"].clone();
        result["placement"] = placement;
        if !manifest_summary.is_null() {
            result["capture_manifest"] = manifest_summary; // the capture-manifest contract ingest outcome
        }
        if !format_record.is_null() {
            result["format"] = format_record;
        }
        if probed.variable_frame_rate {
            result["variable_frame_rate"] = json!(true);
        }
        // Capture markers the manifest asked for but that could not be made
        // are a partial import: said as a warning, never a plain "done".
        let mut warnings: Vec<String> = format_warning.into_iter().collect();
        if let Some(skipped) = result["capture_manifest"]["skipped"].as_array() {
            if !skipped.is_empty() {
                warnings.push(format!(
                    "{} capture marker{} could not be created: {}",
                    skipped.len(),
                    if skipped.len() == 1 { "" } else { "s" },
                    skipped[0].as_str().unwrap_or("see capture_manifest.skipped")
                ));
            }
        }
        if warnings.is_empty() {
            state.jobs.finish(&jid, result);
        } else {
            result["warnings"] = json!(warnings);
            state.jobs.finish_with_warnings(&jid, result);
        }
    });
    job_id
}

pub(crate) fn spawn_plain_import_chain(
    state: AppState,
    asset_id: String,
    src: PathBuf,
    hash: String,
    make_proxy: bool,
) -> String {
    spawn_import_chain(state, asset_id, src, hash, None, make_proxy, false)
}

/// Background PROXY job: transcode the 960x540 editing proxy + build the
/// filmstrip, OUT of the import chain so a multi-GB import is "ready to edit"
/// instantly. NON-FATAL — editing works from the SOURCE (Preview source-fallback,
/// and the final render uses the source, so a failed/slow proxy only costs
/// smooth scrubbing, never the import. The proxy transcode streams real progress
/// (0.05..0.85) so a long encode never reads as a frozen number; the filmstrip
/// rides the finished proxy (0.85..1.0, video only). `asset.proxy`/`asset.filmstrip`
/// update on completion → the UI auto-swaps source→proxy. Returns the job id,
/// surfaced in the import result as `proxy_job`.
const PROXY_MAX_RUNNING: usize = 1;
pub(crate) const ANALYSIS_MAX_RUNNING: usize = 1;

#[cfg(all(test, unix))]
#[path = "media/proxy_missing_tests.rs"]
mod proxy_missing_tests;

fn spawn_proxy_chain(
    state: AppState,
    asset_id: String,
    src: PathBuf,
    source_hash: String,
    proxies: PathBuf,
    dir: PathBuf,
    kind: String,
    duration_ms: Option<u64>,
) -> String {
    let job = state.jobs.create("proxy");
    let job_id = job.job_id.clone();
    let jid = job_id.clone();
    let jobs = state.jobs.clone();
    jobs.spawn_limited(&job_id, "proxy", PROXY_MAX_RUNNING, async move {
        let mut warnings: Vec<String> = Vec::new();
        // Hold the shared lease from the first generated proxy through its
        // filmstrip and both ledger records. An exclusive purge either starts
        // before this chain or refuses while this producer is active.
        let _cache_write = state.cache_lifecycle_lease.read().await;
        let identity = match crate::cache_lifecycle::CacheSource::capture(
            &state,
            &dir,
            &asset_id,
            Some((&src, &source_hash)),
        )
        .await
        {
            Ok(identity) => identity,
            Err(error) => return state.jobs.fail(&jid, error),
        };
        let binding = identity.binding(crate::cache_lifecycle::CacheKind::Proxies);
        state.jobs.progress(
            &jid,
            0.05,
            Some("generating editing proxy (background; you can edit from the source now)".into()),
        );
        // An HDR source previews tone-mapped to SDR, as it will export.
        let tone_map = decode_check::proxy_tone_map(&state, &asset_id, &mut warnings).await;
        // Proxy transcode with live progress mapped into the 0.05..0.85 band.
        let total = duration_ms.unwrap_or(0);
        let (s, pd, aid) = (src.clone(), proxies.clone(), asset_id.clone());
        let (st_cb, jid_cb) = (state.clone(), jid.clone());
        let proxy_ok = match run_blocking("media.proxy", move || {
            let cb = move |f: f32| {
                st_cb
                    .jobs
                    .progress(&jid_cb, 0.05 + f * 0.80, Some("transcoding proxy".into()));
            };
            cut_media::make_proxy_with_progress_and_tone_map_bound(
                &s, &pd, &aid, total, &cb, &tone_map, &binding,
            )
        })
        .await
        {
            Ok(path) => {
                let rel = format!(
                    "proxies/{}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
                match identity
                    .record(
                        &state,
                        &dir,
                        crate::cache_lifecycle::CacheKind::Proxies,
                        &path,
                    )
                    .await
                {
                    Ok(()) => {
                        match update_asset(&state, &asset_id, |a| a.proxy = Some(rel)).await {
                            Ok(_) => true,
                            Err(e) => {
                                warnings
                                    .push(format!("proxy write-back: {} ({})", e.message, e.cause));
                                false
                            }
                        }
                    }
                    Err(e) => {
                        warnings.push(format!("proxy ownership: {} ({})", e.message, e.cause));
                        false
                    }
                }
            }
            Err(e) if e.code == cut_core::error_codes::TOOL_MISSING => {
                state.jobs.fail(&jid, e);
                return;
            }
            Err(e) => {
                warnings.push(format!("proxy: {} ({})", e.message, e.cause));
                false
            }
        };
        // Decode check (video only; rides the proxy): the proxy decoded the
        // whole source, so its length is the source's readable length. A file
        // cut off part-way gets a plain warning on the asset and on this job.
        let mut decode_check: Option<Value> = None;
        if proxy_ok && kind == "video" {
            let proxy_path = proxies.join(format!("{asset_id}.mp4"));
            match decode_check::record_decode_check(&state, &asset_id, &proxy_path).await {
                Ok(warning) => {
                    if let Some(w) = warning {
                        warnings.push(w);
                    }
                    decode_check = current_decode_check(&state, &asset_id).await;
                }
                Err(e) => warnings.push(format!(
                    "Could not check how much of the file can be read: {} ({})",
                    e.message, e.cause
                )),
            }
        }
        // Filmstrip — timeline thumbnail strip (video only; rides the proxy).
        if proxy_ok && kind == "video" {
            if let Some(d) = duration_ms.filter(|d| *d > 0) {
                state
                    .jobs
                    .progress(&jid, 0.88, Some("building filmstrip".into()));
                let proxy_path = proxies.join(format!("{asset_id}.mp4"));
                let film_dir = dir.join("filmstrip");
                let (pp, fd, aid) = (proxy_path, film_dir, asset_id.clone());
                let binding = identity.binding(crate::cache_lifecycle::CacheKind::Thumbnails);
                let strip = run_blocking("media.filmstrip", move || {
                    cut_media::filmstrip::make_filmstrip_bound(&pp, &fd, &aid, d, &binding)
                })
                .await;
                if let Err(e) = &strip {
                    // Said, not dropped: the timeline shows no frames for it.
                    warnings.push(format!(
                        "no timeline thumbnails: {} (rebuild them with media.filmstrip)",
                        e.message
                    ));
                }
                if let Ok(path) = strip {
                    let rel = format!(
                        "filmstrip/{}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    );
                    match identity
                        .record(
                            &state,
                            &dir,
                            crate::cache_lifecycle::CacheKind::Thumbnails,
                            &path,
                        )
                        .await
                    {
                        Ok(()) => {
                            if let Err(e) =
                                update_asset(&state, &asset_id, |a| a.filmstrip = Some(rel)).await
                            {
                                warnings.push(format!(
                                    "filmstrip write-back: {} ({})",
                                    e.message, e.cause
                                ));
                            }
                        }
                        Err(e) => warnings
                            .push(format!("filmstrip ownership: {} ({})", e.message, e.cause)),
                    }
                }
            }
        }
        state.jobs.progress(&jid, 1.0, Some("done".into()));
        let has_warnings = !warnings.is_empty();
        let result = json!({
            "asset": asset_id,
            "proxy": proxy_ok,
            "decode_check": decode_check,
            "warnings": warnings,
            "readiness": {
                "source": "ready",
                "edit": "ready",
                "proxy": if proxy_ok { "ready" } else { "source_fallback" },
            },
        });
        if has_warnings {
            state.jobs.finish_with_warnings(&jid, result);
        } else {
            state.jobs.finish(&jid, result);
        }
    });
    job_id
}

/// Background ENRICH job: transcribe (word timestamps) + the perception battery.
/// Split OUT of the import chain so heavy/slow analysis NEVER blocks
/// editing — the clip is already auto-placed + proxied when this starts. Each
/// step is NON-FATAL: a missing, slow, or failed sidecar degrades to a warning on
/// THIS job, never a failed import (the "core editing works without the sidecar"
/// contract, preventing a long import from appearing frozen). Transcript-
/// dependent verbs (captions / cut_words / remove_fillers) wait on this
/// job's id, returned in the import result as `enrich_job`.
fn spawn_enrich_chain(
    state: AppState,
    asset_id: String,
    src: PathBuf,
    hash: String,
    receipts: PathBuf,
    kind: String,
) -> String {
    let job = state.jobs.create("enrich");
    let job_id = job.job_id.clone();
    let jid = job_id.clone();
    let jobs = state.jobs.clone();
    jobs.spawn_limited(&job_id, "analysis", ANALYSIS_MAX_RUNNING, async move {
        let mut warnings: Vec<String> = Vec::new();
        // transcribe — word timestamps (the slow one on long footage). Real
        // sub-progress is streamed from the python sidecar's PROGRESS lines into
        // the 0.1..0.55 band so the pill never reads as a frozen number while the
        // background transcription runs. The label keeps "keep editing"
        // visible so the non-blocking nature is obvious.
        state.jobs.progress(
            &jid,
            0.1,
            Some("transcribing… (background, keep editing)".into()),
        );
        let (s, rd, aid, h) = (
            src.clone(),
            receipts.clone(),
            asset_id.clone(),
            hash.clone(),
        );
        let (st_cb, jid_cb) = (state.clone(), jid.clone());
        let cancellation = crate::jobs::current_job_cancellation();
        let sidecar_cancellation = cancellation.clone();
        let transcript = run_blocking("media.transcribe", move || {
            let cb = move |frac: f32, label: &str| {
                let clean = label.strip_prefix("transcribe:").unwrap_or(label);
                st_cb.jobs.progress(
                    &jid_cb,
                    0.1 + frac * 0.45,
                    Some(format!("transcribing… {clean} (keep editing)")),
                );
            };
            let control = owned_job_process_control(sidecar_cancellation);
            cut_perception::transcribe_owned_progress(
                &s,
                &rd,
                &aid,
                &h,
                None,
                Some(Arc::new(cb)),
                &control,
            )
        })
        .await;
        if let Some(reason) = cancellation.reason() {
            return state.jobs.cancel_from_worker(&jid, reason);
        }
        let transcript_ok = match transcript {
            Ok(_) => {
                let rel = format!("receipts/{asset_id}.words.json");
                match update_asset(&state, &asset_id, |a| a.transcript = Some(rel)).await {
                    Ok(_) => true,
                    Err(e) => {
                        warnings.push(format!(
                            "transcript write-back: {} ({})",
                            e.message, e.cause
                        ));
                        false
                    }
                }
            }
            Err(e) if cut_perception::is_captions_not_installed(&e) => {
                // The captions runtime is an optional install, not a fault of this
                // import. The perception step would stop on the same missing
                // runtime, so skip it and finish once with the reason and the
                // setup action (agents read them here; a person sees the Install
                // captions card). No warning and no failed row on every import.
                state.jobs.progress(
                    &jid,
                    1.0,
                    Some("captions and transcription are not installed".into()),
                );
                return state.jobs.finish(
                    &jid,
                    json!({
                        "asset": asset_id,
                        "transcript": false,
                        "perception": false,
                        "skipped": "captions_not_installed",
                        "skipped_reason": e.message,
                        "skipped_cause": e.cause,
                        "suggested_action": e.suggested_action,
                        "setup_needed": super::rendering::captions_setup_needed(&e),
                        "warnings": [],
                        "readiness": {
                            "speech": "unavailable",
                            "perception": "unavailable",
                            "optional_services": "not_installed",
                        },
                    }),
                );
            }
            Err(e) => {
                warnings.push(format!("transcribe: {} ({})", e.message, e.cause));
                false
            }
        };
        // perception — the instrument battery (video instruments skipped for
        // audio-only via InstrumentSet::for_kind). Non-fatal too. Streams
        // per-instrument PROGRESS into the 0.6..1.0 band so the slow scene-detect
        // pass on a big file never reads as a frozen "perception 60%".
        state.jobs.progress(
            &jid,
            0.6,
            Some("analysing… (background, keep editing)".into()),
        );
        let set = cut_perception::InstrumentSet::for_kind(&kind);
        let (s, rd, aid, h) = (
            src.clone(),
            receipts.clone(),
            asset_id.clone(),
            hash.clone(),
        );
        let (st_cb2, jid_cb2) = (state.clone(), jid.clone());
        let cancellation = crate::jobs::current_job_cancellation();
        let sidecar_cancellation = cancellation.clone();
        let perception = run_blocking("media.perception", move || {
            let cb = move |frac: f32, label: &str| {
                let clean = label.strip_prefix("perception:").unwrap_or(label);
                st_cb2.jobs.progress(
                    &jid_cb2,
                    0.6 + frac * 0.4,
                    Some(format!("analysing {clean}… (keep editing)")),
                );
            };
            let control = owned_job_process_control(sidecar_cancellation);
            cut_perception::run_instruments_owned_progress(
                &s,
                &rd,
                &aid,
                &h,
                set,
                None,
                Some(Arc::new(cb)),
                &control,
            )
        })
        .await;
        if let Some(reason) = cancellation.reason() {
            return state.jobs.cancel_from_worker(&jid, reason);
        }
        let perception_ok = match perception {
            Ok(_) => {
                let rel = format!("receipts/{asset_id}.perception.json");
                match update_asset(&state, &asset_id, |a| a.perception = Some(rel)).await {
                    Ok(_) => true,
                    Err(e) => {
                        warnings.push(format!(
                            "perception write-back: {} ({})",
                            e.message, e.cause
                        ));
                        false
                    }
                }
            }
            Err(e) => {
                warnings.push(format!("perception: {} ({})", e.message, e.cause));
                false
            }
        };
        state.jobs.progress(&jid, 1.0, Some("done".into()));
        let has_warnings = !warnings.is_empty();
        let result = json!({
            "asset": asset_id,
            "transcript": transcript_ok,
            "perception": perception_ok,
            "warnings": warnings,
            "readiness": {
                "speech": if transcript_ok { "ready" } else { "unavailable" },
                "perception": if perception_ok { "ready" } else { "unavailable" },
                "optional_services": "not_required",
            },
        });
        if has_warnings {
            state.jobs.finish_with_warnings(&jid, result);
        } else {
            state.jobs.finish(&jid, result);
        }
    });
    job_id
}

/// Why `media.import` cannot take `path`: say the true reason (a folder, access
/// denied, or really missing), never "not found" for something that exists.
fn media_path_error(path: &Path) -> CutError {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => CutError::new(
            error_codes::INVALID_ARGS,
            format!("{} is a folder, not a media file", path.display()),
            "media.import takes one media file per call; folders are not imported",
        )
        .with_suggested_action(
            "list the folder and call media.import for each media file inside it",
        ),
        Ok(_) => CutError::new(
            error_codes::INVALID_ARGS,
            format!("{} is not a regular media file", path.display()),
            "the path is not an ordinary file (for example a device or a pipe)",
        )
        .with_suggested_action("pass the path of a media file"),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => CutError::new(
            error_codes::IO,
            format!(
                "media file cannot be read: {} (access is denied)",
                path.display()
            ),
            error.to_string(),
        )
        .with_suggested_action(
            "give your user account read access to the file, or choose another file",
        ),
        Err(_) => CutError::new(
            error_codes::NOT_FOUND,
            format!("media file not found: {}", path.display()),
            "path must point at an existing readable file",
        )
        .with_suggested_action("pass an absolute path to the media file"),
    }
}

/// media.probe{asset} — synchronous probe + cache write-back (not an op).
pub(super) async fn media_probe(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    // Pin the project through both the probe receipt and asset write-back.
    let _project_transition = pin_media_project(state).await?;
    media_probe_admitted(state, args).await
}

pub(super) async fn media_probe_under_transition(
    state: &AppState,
    args: Value,
    _transition: &tokio::sync::MutexGuard<'_, ()>,
) -> Result<VerbResult, CutError> {
    media_probe_admitted(state, args).await
}

async fn media_probe_admitted(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        asset: String,
    }
    let a: Args = parse_args(args)?;
    let (src, _hash) = asset_info(state, &a.asset).await?;
    let probe = run_blocking("media.probe", move || cut_media::probe_source(&src)).await?;
    let fresh = serde_json::to_value(&probe)?;
    // A refreshed probe keeps the earlier decode check (it reads the index
    // only), when this engine made that check (a received one is dropped).
    let old = if probe_verified(state, &a.asset).await {
        asset_probe(state, &a.asset).await
    } else {
        None
    };
    let pv = decode_check::keep_decode_check(old.as_ref(), fresh);
    let pv2 = pv.clone();
    persist_probe_file(state, &a.asset, &pv).await; // survives rebuild
    update_asset(state, &a.asset, |x| x.probe = Some(pv2)).await?;
    mark_probe_verified(state, &a.asset).await;
    Ok(VerbResult::ok(pv))
}

/// media.waveform{asset, buckets?} — per-bucket audio peaks (0..1) for the
/// timeline waveform (workspace contract v1). Synchronous, deterministic, memory-bounded
/// (the decode rate is chosen from the probed duration). Errors when the asset
/// has no audio stream.
pub(super) async fn media_waveform(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        asset: String,
        buckets: Option<usize>,
    }
    let a: Args = parse_args(args)?;
    let (src, _hash) = asset_info(state, &a.asset).await?;
    let duration_ms = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let asset = store.project.assets.get(&a.asset).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("no asset '{}'", a.asset),
                "asset ids come from project.state",
            )
        })?;
        let probe = asset.probe.as_ref().ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                format!("asset '{}' is not probed yet", a.asset),
                "media.waveform needs the asset probe duration to choose a bounded decode rate",
            )
            .with_suggested_action("wait for the import chain to finish or call media.probe first")
        })?;
        let duration = probe
            .get("duration_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if duration == 0 {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("asset '{}' is not probed yet or has no timed audio duration", a.asset),
                "media.waveform needs a positive duration_ms from media.probe",
            )
            .with_suggested_action(
                "wait for media.import/media.probe to finish; still images and zero-duration assets have no waveform",
            ));
        }
        if !probe
            .get("has_audio")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("asset '{}' has no audio stream", a.asset),
                "media.waveform only extracts waveform peaks from audio-bearing media",
            )
            .with_suggested_action("choose an asset where media.probe reports has_audio=true"));
        }
        duration
    };
    let buckets = a.buckets.unwrap_or(1000);
    let wf = run_blocking("media.waveform", move || {
        cut_media::waveform::waveform(&src, duration_ms, buckets)
    })
    .await?;
    Ok(VerbResult::ok(json!({
        "asset": a.asset,
        "bucket_count": wf.bucket_count,
        "peaks": wf.peaks,
        "source_ms": wf.source_ms,
        "sample_rate": wf.sample_rate,
    })))
}

/// media.transcribe{asset, model?} — active STT word transcript as a background job
/// (the background-job contract: returns {job_id}).
pub(super) async fn media_transcribe(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    // Bind source/receipt selection to registration in the current project's
    // job manager; project replacement drains that tracked worker.
    let _project_transition = pin_media_project(state).await?;
    #[derive(serde::Deserialize)]
    struct Args {
        asset: String,
        model: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let (src, hash) = asset_info(state, &a.asset).await?;
    let (_dir, receipts, _p) = project_paths(state).await?;
    let job = state.jobs.create("transcribe");
    let job_id = job.job_id.clone();
    let st = state.clone();
    let jobs = state.jobs.clone();
    jobs.spawn_limited(&job_id, "analysis", ANALYSIS_MAX_RUNNING, async move {
        st.jobs
            .progress(&job.job_id, 0.1, Some("transcribing".into()));
        let aid = a.asset.clone();
        let (st_cb, jid_cb) = (st.clone(), job.job_id.clone());
        let cancellation = crate::jobs::current_job_cancellation();
        let sidecar_cancellation = cancellation.clone();
        let r = run_blocking("media.transcribe", move || {
            let cb = move |frac: f32, label: &str| {
                let clean = label.strip_prefix("transcribe:").unwrap_or(label);
                st_cb.jobs.progress(
                    &jid_cb,
                    0.1 + frac * 0.85,
                    Some(format!("transcribing… {clean}")),
                );
            };
            let control = owned_job_process_control(sidecar_cancellation);
            cut_perception::transcribe_owned_progress(
                &src,
                &receipts,
                &aid,
                &hash,
                a.model.as_deref(),
                Some(Arc::new(cb)),
                &control,
            )
        })
        .await;
        if let Some(reason) = cancellation.reason() {
            return st.jobs.cancel_from_worker(&job.job_id, reason);
        }
        match r {
            Ok(t) => {
                let rel = format!("receipts/{}.words.json", a.asset);
                let rel2 = rel.clone();
                if let Err(e) = update_asset(&st, &a.asset, |x| x.transcript = Some(rel2)).await
                {
                    return st.jobs.fail(
                        &job.job_id,
                        e.with_suggested_action(
                            "transcription succeeded but project state could not point at the transcript receipt",
                        ),
                    );
                }
                st.jobs.finish(
                    &job.job_id,
                    json!({"transcript": rel, "words": t.words.len()}),
                );
            }
            Err(e) => st.jobs.fail(&job.job_id, e),
        }
    });
    Ok(VerbResult::ok(json!({"job_id": job_id})))
}

/// media.perception{asset} — full instrument run as a background job.
pub(super) async fn media_perception(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    let _project_transition = pin_media_project(state).await?;
    #[derive(serde::Deserialize)]
    struct Args {
        asset: String,
    }
    let a: Args = parse_args(args)?;
    let (src, hash) = asset_info(state, &a.asset).await?;
    let (_dir, receipts, _p) = project_paths(state).await?;
    // audio-only media guard: audio-only assets get the audio battery — video
    // instruments (scenes) crash on media with no video stream. Kind comes
    // from the cached probe; unknown (never probed) keeps Full, matching
    // the pre-fix behavior for timed media.
    let kind = {
        let guard = state.project.read().await;
        guard
            .as_ref()
            .and_then(|st| st.project.assets.get(&a.asset))
            .and_then(|x| x.probe.as_ref())
            .and_then(|p| p.get("kind"))
            .and_then(|k| k.as_str())
            .unwrap_or("video")
            .to_string()
    };
    let set = cut_perception::InstrumentSet::for_kind(&kind);
    let job = state.jobs.create("perception");
    let job_id = job.job_id.clone();
    let st = state.clone();
    let jobs = state.jobs.clone();
    jobs.spawn_limited(&job_id, "analysis", ANALYSIS_MAX_RUNNING, async move {
        st.jobs
            .progress(&job.job_id, 0.1, Some("running instruments".into()));
        let aid = a.asset.clone();
        let cancellation = crate::jobs::current_job_cancellation();
        let sidecar_cancellation = cancellation.clone();
        let r = run_blocking("media.perception", move || {
            let control = owned_job_process_control(sidecar_cancellation);
            cut_perception::run_instruments_owned(
                &src,
                &receipts,
                &aid,
                &hash,
                set,
                None,
                &control,
            )
        })
        .await;
        if let Some(reason) = cancellation.reason() {
            return st.jobs.cancel_from_worker(&job.job_id, reason);
        }
        match r {
            Ok(_rep) => {
                let rel = format!("receipts/{}.perception.json", a.asset);
                let rel2 = rel.clone();
                if let Err(e) = update_asset(&st, &a.asset, |x| x.perception = Some(rel2)).await
                {
                    return st.jobs.fail(
                        &job.job_id,
                        e.with_suggested_action(
                            "perception succeeded but project state could not point at the perception receipt",
                        ),
                    );
                }
                st.jobs.finish(&job.job_id, json!({"perception": rel}));
            }
            Err(e) => st.jobs.fail(&job.job_id, e),
        }
    });
    Ok(VerbResult::ok(json!({"job_id": job_id})))
}

#[cfg(test)]
mod media_security_tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[tokio::test]
    async fn attested_import_rejects_changed_bytes_before_project_mutation() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("artifact.png");
        std::fs::write(&path, b"changed bytes").unwrap();

        let error = media_import(
            &AppState::new(),
            json!({
                "path": path,
                "expected_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                "expected_byte_length": 13,
            }),
            Actor::system(),
        )
        .await
        .expect_err("changed attested bytes must fail before media.import records an op");

        assert_eq!(error.code, error_codes::INVALID_ARGS);
        assert!(error.message.contains("changed before Cut imported"));
    }

    #[tokio::test]
    async fn attested_import_persists_the_full_sha256_asset_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().join("attested-import.cutproj");
        let state = AppState::new();
        let created = crate::dispatch::dispatch(
            &state,
            "project.create",
            json!({"name": "attested-import", "dir": project_dir}),
            Actor::system(),
        )
        .await;
        assert!(
            created.ok,
            "attested fixture project failed: {:?}",
            created.error
        );

        let bytes = b"sealed artifact bytes";
        let path = tmp.path().join("artifact.bin");
        std::fs::write(&path, bytes).unwrap();
        let expected_sha256 = format!("{:x}", Sha256::digest(bytes));
        let imported = media_import(
            &state,
            json!({
                "path": path,
                "proxy": false,
                "expected_sha256": expected_sha256,
                "expected_byte_length": bytes.len(),
            }),
            Actor::system(),
        )
        .await
        .unwrap();
        let asset_id = imported.result.unwrap()["asset_id"]
            .as_str()
            .unwrap()
            .to_string();
        let guard = state.project.read().await;
        let asset = guard
            .as_ref()
            .unwrap()
            .project
            .assets
            .get(&asset_id)
            .unwrap();
        assert_eq!(asset.hash, format!("sha256:{expected_sha256}"));
    }
}

#[cfg(test)]
#[derive(Clone)]
pub(super) struct MediaImportTransitionGate {
    pub source: PathBuf,
    pub committed: Arc<tokio::sync::Notify>,
    pub resume: Arc<tokio::sync::Notify>,
    pub admitted: Arc<tokio::sync::Notify>,
    pub resume_admitted: Arc<tokio::sync::Notify>,
}

#[cfg(test)]
fn media_import_transition_gate() -> &'static std::sync::Mutex<Option<MediaImportTransitionGate>> {
    static GATE: std::sync::OnceLock<std::sync::Mutex<Option<MediaImportTransitionGate>>> =
        std::sync::OnceLock::new();
    GATE.get_or_init(|| std::sync::Mutex::new(None))
}

#[cfg(test)]
pub(super) fn install_media_import_transition_gate(gate: Option<MediaImportTransitionGate>) {
    *media_import_transition_gate().lock().unwrap() = gate;
}

#[cfg(test)]
async fn wait_for_media_import_transition_gate(source: &Path) {
    let gate = media_import_transition_gate()
        .lock()
        .unwrap()
        .as_ref()
        .filter(|gate| gate.source == source)
        .cloned();
    if let Some(gate) = gate {
        gate.committed.notify_one();
        gate.resume.notified().await;
    }
}

#[cfg(test)]
async fn wait_for_media_import_admission_gate(source: &Path) {
    let gate = media_import_transition_gate()
        .lock()
        .unwrap()
        .as_ref()
        .filter(|gate| gate.source == source)
        .cloned();
    if let Some(gate) = gate {
        gate.admitted.notify_one();
        gate.resume_admitted.notified().await;
    }
}

#[cfg(test)]
mod media_path_error_tests {
    use super::*;

    #[test]
    fn a_folder_is_named_as_a_folder_not_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let error = media_path_error(dir.path());
        assert_eq!(error.code, error_codes::INVALID_ARGS);
        assert!(error.message.contains("is a folder, not a media file"));
        assert!(!error.message.contains("not found"));
        assert!(error
            .suggested_action
            .unwrap()
            .contains("media.import for each media file"));
        let missing = media_path_error(&dir.path().join("absent.mp4"));
        assert_eq!(missing.code, error_codes::NOT_FOUND);
    }
}

#[cfg(test)]
mod asset_info_path_tests {
    use super::*;

    /// A relative path stored in a received project reaches tools inside the
    /// project folder, never as a bare argument such as `-report`.
    #[tokio::test]
    async fn a_relative_asset_path_is_given_to_tools_inside_the_project() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::new();
        let created = crate::dispatch::dispatch(
            &state,
            "project.create",
            json!({"name":"paths", "dir": root.path().join("paths.cutproj")}),
            cut_core::Actor::system(),
        )
        .await;
        assert!(created.ok, "project create: {:?}", created.error);
        let dir = {
            let mut project = state.project.write().await;
            let store = project.as_mut().unwrap();
            store.project.assets.insert(
                "a1".into(),
                cut_core::Asset {
                    path: "-report".into(),
                    hash: "sha256:test".into(),
                    probe: None,
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                },
            );
            store.dir.clone()
        };
        let (path, _) = asset_info(&state, "a1").await.unwrap();
        assert_eq!(path, dir.join("-report"));
        assert!(path.is_absolute());
    }
}

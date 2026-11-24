//! Clip-marker ingestion for screen-recording takes (raw, polished, recovered).
//!
//! Reads `<capture>/studio-events.jsonl` (falling back to legacy `studio-events.json`),
//! verifies 1:1 source-to-baked duration correspondence on polished takes, persists
//! durable `<capture>/raw-link.json` and `<baked>.clip-markers.json` sidecars, and builds
//! single-op lowered `edit._set_clip_markers` steps for `media.import` auto-place,
//! `edit.insert`, `edit.insert_linked`, `edit.overwrite`, and `screen_record.recover`.

use std::path::{Path, PathBuf};

use cut_core::store::{read_bounded_cache, replace_project_file, BoundedCacheRead};
use cut_core::{
    edit, error_codes, hash_file, Actor, ClipMarker, CutError, InverseOp, OpRecord, Project,
    ProjectStore, TrackKind, VerbWarning,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{screen_record_cache_dir, strip_verbatim_prefix};
use crate::screen_record_studio::StudioEventLog;
use crate::screen_record_studio_journal::{read_studio_journal, STUDIO_EVENTS_FILENAME};
use crate::state::AppState;

const RAW_LINK_SCHEMA: &str = "shellx-cut/raw-recording-link@1";
const RAW_LINK_FILE: &str = "raw-link.json";
const BAKED_MARKERS_SCHEMA: &str = "shellx-cut/baked-clip-markers@1";
const MAX_SIDECAR_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CAPTURE_DIRS_SCAN: usize = 512;
const POLISH_DURATION_TOLERANCE_MS: u64 = 250;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MarkerSeed {
    pub(crate) source_ms: u64,
    pub(crate) label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) color: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RawLinkRecord {
    schema: String,
    capture_id: String,
    raw_hash: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct BakedMarkersSidecar {
    schema: String,
    baked_hash: String,
    markers: Vec<MarkerSeed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notice: Option<String>,
}

pub(super) fn baked_sidecar_path(baked: &Path) -> PathBuf {
    let mut os = baked.as_os_str().to_os_string();
    os.push(".clip-markers.json");
    PathBuf::from(os)
}

fn jsonl_or_legacy_markers(capture_dir: &Path) -> Result<Vec<MarkerSeed>, CutError> {
    let jsonl = capture_dir.join(STUDIO_EVENTS_FILENAME);
    let legacy = capture_dir.join(crate::screen_record_studio::LEGACY_STUDIO_EVENTS_FILENAME);
    let log: StudioEventLog = if jsonl.exists() {
        read_studio_journal(&jsonl)?
    } else if legacy.exists() {
        let BoundedCacheRead::Bytes(bytes) = read_bounded_cache(&legacy, MAX_SIDECAR_BYTES) else {
            return Err(CutError::new(
                error_codes::IO,
                format!(
                    "could not read legacy Studio events at {}",
                    legacy.display()
                ),
                "legacy studio-events.json is missing, linked, or oversized",
            ));
        };
        serde_json::from_slice(&bytes)?
    } else {
        return Ok(Vec::new());
    };

    let mut markers: Vec<MarkerSeed> = log
        .events
        .into_iter()
        .filter(|ev| ev.source == "recording" && ev.kind == "marker")
        .map(|ev| MarkerSeed {
            source_ms: ev.t_ms,
            label: ev
                .label
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("Marker")
                .to_string(),
            note: None,
            color: None,
        })
        .collect();
    markers.sort_by_key(|m| m.source_ms);
    Ok(markers)
}

pub(crate) fn read_capture_markers(capture_dir: &Path) -> Vec<MarkerSeed> {
    jsonl_or_legacy_markers(capture_dir).unwrap_or_default()
}

/// Link a stopped take's muxed raw recording (saved in the project's exports
/// folder, away from its capture folder) to that capture by content hash, so a
/// later add to the timeline finds the take's markers. Never fails the stop:
/// a problem comes back as the note the stop reply carries.
pub(crate) fn link_raw_take(
    capture_dir: &Path,
    capture_id: &str,
    raw_mp4: &Path,
) -> Option<String> {
    let write = || -> Result<(), CutError> {
        let body = serde_json::to_vec_pretty(&RawLinkRecord {
            schema: RAW_LINK_SCHEMA.to_string(),
            capture_id: capture_id.to_string(),
            raw_hash: hash_file(raw_mp4)?,
        })?;
        replace_project_file(&capture_dir.join(RAW_LINK_FILE), &body)?;
        Ok(())
    };
    write().err().map(|e| {
        format!(
            "The take's markers could not be linked to the raw recording, so they will not appear on its clip in Edit ({}).",
            e.message
        )
    })
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn secs(ms: Option<u64>) -> String {
    ms.map_or_else(
        || "unknown".into(),
        |v| format!("{:.1} s", v as f64 / 1000.0),
    )
}

fn write_baked_sidecar(
    baked: &Path,
    baked_hash: String,
    markers: Vec<MarkerSeed>,
    notice: Option<String>,
) -> Result<(), CutError> {
    let sidecar = BakedMarkersSidecar {
        schema: BAKED_MARKERS_SCHEMA.to_string(),
        baked_hash,
        markers,
        notice,
    };
    replace_project_file(
        &baked_sidecar_path(baked),
        &serde_json::to_vec_pretty(&sidecar)?,
    )?;
    Ok(())
}

/// Decide which take markers the polished (or stream-copied) clip carries and
/// store them beside the baked file, where the insert that places it reads
/// them. The polish keeps recorded time 1:1 (its plan has no cuts), so a marker
/// keeps its time when the polished length matches the recording; when that
/// cannot be confirmed no marker is added and the returned notice says so.
/// Never fails the polish: a problem becomes the notice.
/// Returns `(markers_added, notice)`.
pub(crate) fn for_polish(
    source_dir: &Path,
    source: &Path,
    baked: &Path,
    plan_path: &Path,
    raw: bool,
) -> (usize, Option<String>) {
    let markers = match jsonl_or_legacy_markers(source_dir) {
        Ok(m) => m,
        Err(e) => {
            return (
                0,
                Some(format!(
                    "The take's markers could not be read, so none were added to the polished clip ({}).",
                    e.message
                )),
            )
        }
    };
    let baked_hash = match hash_file(baked) {
        Ok(h) => h,
        Err(e) if !markers.is_empty() => {
            return (
                0,
                Some(format!(
                    "{} take marker{} could not be added to the polished clip ({}).",
                    markers.len(),
                    plural(markers.len()),
                    e.message
                )),
            )
        }
        Err(_) => return (0, None),
    };
    if markers.is_empty() {
        let _ = write_baked_sidecar(baked, baked_hash, Vec::new(), None);
        return (0, None);
    }

    let source_dur = cut_media::probe(source).ok().and_then(|p| p.duration_ms);
    let baked_dur = cut_media::probe(baked).ok().and_then(|p| p.duration_ms);
    let plan_dur = if raw {
        None
    } else {
        std::fs::read(plan_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<record_core::EditPlan>(&b).ok())
            .map(|p| p.duration_ms)
    };
    let close = |a: u64, b: u64| a > 0 && b > 0 && a.abs_diff(b) <= POLISH_DURATION_TOLERANCE_MS;
    let mapping_ok = match (raw, source_dur, baked_dur, plan_dur) {
        (true, Some(sd), Some(bd), _) => close(sd, bd),
        (false, Some(sd), Some(bd), Some(pd)) => close(bd, pd) && close(sd, pd),
        _ => false,
    };

    if !mapping_ok {
        let lengths = if raw {
            format!("recording {}, copy {}", secs(source_dur), secs(baked_dur))
        } else {
            format!(
                "recording {}, polish settings {}, polished clip {}",
                secs(source_dur),
                secs(plan_dur),
                secs(baked_dur)
            )
        };
        let notice = format!(
            "{} take marker{} not added to the polished clip: its length could not be matched to the recording ({lengths}), so the marker times could not be placed exactly.",
            markers.len(),
            if markers.len() == 1 { " was" } else { "s were" },
        );
        let _ = write_baked_sidecar(baked, baked_hash, Vec::new(), Some(notice.clone()));
        return (0, Some(notice));
    }

    let limit = baked_dur.unwrap_or(u64::MAX);
    let (kept, dropped): (Vec<_>, Vec<_>) = markers.into_iter().partition(|m| m.source_ms < limit);
    let mut notice = (!dropped.is_empty()).then(|| {
        format!(
            "{} take marker{} after the end of the polished clip ({}) {} not added.",
            dropped.len(),
            plural(dropped.len()),
            secs(baked_dur),
            if dropped.len() == 1 { "was" } else { "were" },
        )
    });
    let added = kept.len();
    if let Err(e) = write_baked_sidecar(baked, baked_hash, kept, notice.clone()) {
        notice = Some(format!(
            "{added} take marker{} could not be added to the polished clip ({}).",
            plural(added),
            e.message
        ));
        return (0, notice);
    }
    (added, notice)
}

pub(crate) fn notice_warnings(notice: &Option<String>) -> Vec<VerbWarning> {
    notice
        .iter()
        .map(|msg| VerbWarning {
            code: "clip_markers_skipped".into(),
            message: msg.clone(),
            detail: serde_json::Map::new(),
        })
        .collect()
}

fn parse_baked_sidecar(sidecar: &Path, asset_hash: &str) -> Option<BakedMarkersSidecar> {
    let BoundedCacheRead::Bytes(bytes) = read_bounded_cache(sidecar, MAX_SIDECAR_BYTES) else {
        return None;
    };
    let parsed: BakedMarkersSidecar = serde_json::from_slice(&bytes).ok()?;
    (parsed.schema == BAKED_MARKERS_SCHEMA && parsed.baked_hash == asset_hash).then_some(parsed)
}

pub(super) fn read_baked_sidecar(asset_path: &Path, asset_hash: &str) -> Option<Vec<MarkerSeed>> {
    parse_baked_sidecar(&baked_sidecar_path(asset_path), asset_hash).map(|parsed| parsed.markers)
}

pub(super) fn is_owned_baked_sidecar(sidecar: &Path, render_hash: &str) -> bool {
    parse_baked_sidecar(sidecar, render_hash).is_some()
}

/// Resolve recording clip markers for an imported asset in this project.
pub(crate) fn resolve_markers_for_asset(
    project_dir: Option<&Path>,
    project: &Project,
    asset_id: &str,
) -> Vec<MarkerSeed> {
    let Some(asset) = project.assets.get(asset_id) else {
        return Vec::new();
    };
    let clean_path = strip_verbatim_prefix(Path::new(&asset.path));
    if let Some(from_polish) = read_baked_sidecar(&clean_path, &asset.hash) {
        return from_polish;
    }

    // The take's own screen file sits beside its journal and is in recorded
    // time. Other files in the capture folder (camera, recovered take) are not
    // matched here: their clock differs or they are placed explicitly.
    let file_name = clean_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if file_name == crate::screen_record::pause_projection::LEGACY_ROOT_SOURCE_OUTPUT {
        if let Some(parent) = clean_path.parent() {
            let direct = read_capture_markers(parent);
            if !direct.is_empty() {
                return direct;
            }
        }
    }

    let Some(dir) = project_dir else {
        return Vec::new();
    };
    let Ok(cache) = screen_record_cache_dir(dir) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(cache) else {
        return Vec::new();
    };
    for entry in entries.flatten().take(MAX_CAPTURE_DIRS_SCAN) {
        let cap_dir = entry.path();
        if !cap_dir.is_dir() {
            continue;
        }
        let link_path = cap_dir.join(RAW_LINK_FILE);
        let BoundedCacheRead::Bytes(bytes) = read_bounded_cache(&link_path, 64 * 1024) else {
            continue;
        };
        let Ok(link) = serde_json::from_slice::<RawLinkRecord>(&bytes) else {
            continue;
        };
        if link.schema == RAW_LINK_SCHEMA && link.raw_hash == asset.hash {
            return read_capture_markers(&cap_dir);
        }
    }
    Vec::new()
}

pub(crate) fn should_attach_for_track(project: &Project, asset_id: &str, track_id: &str) -> bool {
    let Some(track) = project.track(track_id) else {
        return false;
    };
    match track.kind {
        TrackKind::Video => true,
        TrackKind::Audio => {
            project
                .assets
                .get(asset_id)
                .and_then(|a| a.probe.as_ref())
                .and_then(|p| p.get("kind"))
                .and_then(Value::as_str)
                == Some("audio")
        }
        TrackKind::Caption => false,
    }
}

pub(crate) fn marker_inputs_with_ids(
    project: &Project,
    seeds: &[MarkerSeed],
    offset: usize,
) -> Vec<ClipMarker> {
    let base = cut_core::clip_markers::next_clip_marker_seq(project) + offset as u64;
    seeds
        .iter()
        .enumerate()
        .map(|(i, s)| ClipMarker {
            id: format!("cm{}", base + i as u64),
            source_ms: s.source_ms,
            label: s.label.clone(),
            note: s.note.clone(),
            color: s.color.clone(),
        })
        .collect()
}

pub(crate) fn set_clip_markers_step(clip_id: &str, markers: Vec<ClipMarker>) -> InverseOp {
    InverseOp {
        verb: "edit._set_clip_markers".into(),
        args: json!({ "clip": clip_id, "markers": markers }),
    }
}

/// Commit `edit.insert` together with `edit._set_clip_markers` in one atomic op when applicable.
pub(crate) fn apply_insert_with_markers(
    store: &mut ProjectStore,
    args: Value,
    actor: Actor,
    rationale: Option<String>,
    explicit_markers: Option<Vec<MarkerSeed>>,
) -> Result<OpRecord, CutError> {
    let asset = args.get("asset").and_then(Value::as_str).unwrap_or("");
    let track = args.get("track").and_then(Value::as_str).unwrap_or("");
    let seeds = match explicit_markers {
        Some(m) => m,
        None if should_attach_for_track(&store.project, asset, track) => {
            resolve_markers_for_asset(Some(&store.dir), &store.project, asset)
        }
        None => Vec::new(),
    };
    if seeds.is_empty() {
        return store.apply("edit.insert", args, actor, rationale);
    }
    let clip_id = edit::new_clip_id(&store.project);
    let markers = marker_inputs_with_ids(&store.project, &seeds, 0);
    let steps = vec![
        InverseOp {
            verb: "edit.insert".into(),
            args: args.clone(),
        },
        set_clip_markers_step(&clip_id, markers),
    ];
    store.apply_lowered("edit.insert", args, actor, rationale, steps, vec![])
}

/// Dispatch `edit.insert` atomically with any capture/polish/recover clip markers attached.
pub(crate) async fn commit_insert_dispatch(
    state: &AppState,
    args: Value,
    actor: Actor,
    explicit_markers: Option<Vec<MarkerSeed>>,
) -> Result<cut_core::VerbResult, CutError> {
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let include_legacy_inverse = crate::dispatch::wants_legacy_inverse(&args);
    let op = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().ok_or_else(crate::dispatch::no_project)?;
        crate::dispatch::safety::guard_call("edit.insert", || {
            apply_insert_with_markers(store, args, actor, rationale, explicit_markers)
        })?
    };
    let op_id = op.op_id.clone();
    state
        .events
        .publish(crate::events::Event::OpApplied { op: op.clone() });
    Ok(cut_core::VerbResult::ok_with_ops(
        crate::dispatch::shape_core_result("edit.insert", &op, include_legacy_inverse),
        vec![op_id],
    ))
}

/// Persist exact clip-marker metadata beside a file about to be imported by `screen_record.recover`.
pub(crate) fn write_file_sidecar(
    file_path: &Path,
    markers: Vec<MarkerSeed>,
    notice: Option<String>,
) -> Result<(), CutError> {
    let hash = hash_file(file_path)?;
    let sidecar = BakedMarkersSidecar {
        schema: BAKED_MARKERS_SCHEMA.to_string(),
        baked_hash: hash,
        markers,
        notice,
    };
    replace_project_file(
        &baked_sidecar_path(file_path),
        &serde_json::to_vec_pretty(&sidecar)?,
    )?;
    Ok(())
}

/// Format an honest non-silent recovery notice when some take markers fall past saved duration.
pub(crate) fn recover_unplaced_notice(unplaced: &[MarkerSeed]) -> Option<String> {
    if unplaced.is_empty() {
        return None;
    }
    Some(format!(
        "{} take marker{} after the end of what was saved could not be added to the recovered clip.",
        unplaced.len(),
        plural(unplaced.len())
    ))
}

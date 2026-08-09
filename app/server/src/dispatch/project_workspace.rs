//! Project, library, and comment dispatch handlers.
//!
//! Kept as a child module of `dispatch` so this extraction is behavior-preserving:
//! handlers still share the same commit, draft, library, event, and error helpers.

use super::*;
use crate::jobs::{run_owned, ProcessControl, ProcessTermination};

mod project_health;
mod project_paths;
mod project_sync;
use project_health::project_health as project_health_read;
use project_paths::default_projects_dir;

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ProjectStarter {
    FirstEdit,
}

const FIRST_EDIT_SAMPLE: &[u8] = include_bytes!("../../assets/first-edit-sample.mp4");
const FIRST_EDIT_SAMPLE_PERCEPTION: &str =
    include_str!("../../assets/first-edit-sample.perception.json");

fn install_project_starter(
    store: &ProjectStore,
    starter: ProjectStarter,
) -> Result<PathBuf, CutError> {
    let starter_dir = store.dir.join("starter");
    std::fs::create_dir_all(&starter_dir)?;
    let path = match starter {
        ProjectStarter::FirstEdit => starter_dir.join("first-edit-sample.mp4"),
    };
    std::fs::write(&path, FIRST_EDIT_SAMPLE)?;
    let mut report: cut_perception::PerceptionReport =
        serde_json::from_str(FIRST_EDIT_SAMPLE_PERCEPTION)?;
    report.asset_hash = cut_core::hash_file(&path)?;
    report.source_path = path.display().to_string();
    let transcript = report.words.clone().ok_or_else(|| {
        CutError::new(
            error_codes::IO,
            "bundled First edit sample has no transcript",
            "the embedded perception template must include its authored transcript",
        )
    })?;
    let receipts = store.receipts_dir();
    std::fs::create_dir_all(&receipts)?;
    std::fs::write(
        receipts.join("a1.perception.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    std::fs::write(
        receipts.join("a1.words.json"),
        serde_json::to_vec_pretty(&transcript)?,
    )?;
    Ok(path)
}

/// Replace the open project while the caller holds `project_transition`.
///
/// The project lock must not be held while jobs drain: cancelled
/// `spawn_blocking` workers can still finish through code paths that inspect
/// project state. Removing the store first also prevents new project-owned jobs
/// from being started during the transition. A timed-out drain restores the
/// previous store; a later attach failure reattaches its job persistence before
/// restoring it.
async fn activate_project_locked(state: &AppState, next: ProjectStore) -> Result<(), CutError> {
    let next_name = next.project.name.clone();
    let previous = {
        let mut project = state.project.write().await;
        if let Some(store) = project.as_ref() {
            store.save()?;
        }
        project.take()
    };
    let next_dir = next.dir.clone();

    if let Err(change_error) = state.jobs.switch_project(&next_dir).await {
        // A drain timeout leaves the old JobManager attachment intact. Any
        // other error happened after detach, while attaching the next project,
        // so rebuild the old attachment before making its store available.
        if change_error.code != "job_cancel_pending" {
            if let Some(store) = previous.as_ref() {
                if let Err(restore_error) = state.jobs.attach_project(&store.dir) {
                    tracing::error!(
                        change_error = %change_error,
                        restore_error = %restore_error,
                        project = %store.dir.display(),
                        "project transition failed and prior job persistence could not be restored"
                    );
                    return Err(CutError::new(
                        error_codes::IO,
                        "project change failed and the previous project could not be restored",
                        format!(
                            "change failed: {change_error}; restoring {} failed: {restore_error}",
                            store.dir.display()
                        ),
                    )
                    .with_suggested_action(
                        "reopen the previous project after checking that its directory is writable",
                    ));
                }
            }
        }
        *state.project.write().await = previous;
        return Err(change_error);
    }

    *state.project.write().await = Some(next);
    state.events.publish(Event::ProjectChanged {
        open: true,
        name: Some(next_name),
    });
    Ok(())
}

fn rollback_created_project(path: &Path) {
    if let Err(error) = crate::projects_index::forget(&path.to_string_lossy()) {
        tracing::error!(
            project = %path.display(),
            error = %error,
            "could not roll back failed project creation from the recent index"
        );
    }
    if path.exists() {
        if let Err(error) = std::fs::remove_dir_all(path) {
            tracing::error!(
                project = %path.display(),
                error = %error,
                "could not remove a project whose activation failed"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// project.* handlers
// ---------------------------------------------------------------------------

/// project.create{name, settings?, dir?, starter?} → creates .cutproj, opens it.
/// the single-state-holder contract: `dir` is the TARGET .cutproj path, default <cwd>/<name>.cutproj.
/// Without `dir`, use a writable "ShellX Cut Projects" folder under the OS home
/// so the installed UI can create a project without a path picker.
pub(super) async fn project_create(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        name: String,
        settings: Option<cut_core::ProjectSettings>,
        dir: Option<String>,
        starter: Option<ProjectStarter>,
    }
    let a: Args = parse_args(args)?;
    // ProjectStore::create appends "<name>.cutproj" to its parent arg; when
    // an explicit dir is given, its stem must equal "<name>.cutproj".
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let parent = match &a.dir {
        // Omitted dir = a user-writable projects folder (NOT the app's launch
        // cwd, which on an installed Windows shell is opaque/unwritable). The UI
        // "New project" affordance relies on this default landing somewhere sane.
        None => default_projects_dir(),
        Some(d) => {
            let p = PathBuf::from(d);
            let expected = format!("{}.cutproj", a.name);
            if p.file_name().and_then(|f| f.to_str()) != Some(expected.as_str()) {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("dir must end in '{expected}'"),
                    format!("got '{d}' — project dir name is derived from the project name"),
                )
                .with_suggested_action(format!("pass dir ending in /{expected} or omit dir")));
            }
            p.parent().map(|x| x.to_path_buf()).unwrap_or(cwd)
        }
    };
    let _transition = state.project_transition.lock().await;
    let store = ProjectStore::create_with_actor(&parent, &a.name, a.settings, actor)?;
    let starter_asset_path = match a.starter {
        Some(starter) => match install_project_starter(&store, starter) {
            Ok(path) => Some(path),
            Err(error) => {
                let project_dir = store.dir.clone();
                drop(store);
                let _ = std::fs::remove_dir_all(project_dir);
                return Err(error);
            }
        },
        None => None,
    };
    // Register the new project in the global index (~/.shellx-cut/projects.json) so
    // it appears in the recent-projects list and can be reopened later by path.
    if let Err(error) =
        crate::projects_index::note_created(&store.project.name, &store.dir.to_string_lossy())
    {
        let project_dir = store.dir.clone();
        drop(store);
        rollback_created_project(&project_dir);
        return Err(error.into());
    }
    let op_id = store
        .log
        .current_revision()?
        .expect("a created project has its project.create operation");
    let mut payload = json!({"path": store.dir, "project": store.project});
    if let Some(path) = starter_asset_path {
        payload["starter_asset_path"] = json!(path);
    }
    let project_dir = store.dir.clone();
    if let Err(error) = activate_project_locked(state, store).await {
        rollback_created_project(&project_dir);
        return Err(error);
    }
    Ok(VerbResult::ok_with_ops(payload, vec![op_id]))
}

/// project.open{path} → loads (or rebuilds) the project.
pub(super) async fn project_open(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        path: String,
    }
    let a: Args = parse_args(args)?;
    let _transition = state.project_transition.lock().await;
    let store = ProjectStore::open(Path::new(&a.path))?;
    let recovery_scan = crate::screen_record::recovery::scan_recovery_for_project(&store.dir)?;
    // Touch (or register, if externally-created) this project in the global index,
    // refreshing its recency + timeline summary for the recent-projects list.
    let clips: u64 = store
        .project
        .tracks
        .iter()
        .map(|t| t.clips.len() as u64)
        .sum();
    crate::projects_index::note_opened(
        &store.project.name,
        &store.dir.to_string_lossy(),
        Some(store.project.duration_ms()),
        Some(clips),
    )?;
    let project_revision = store.log.current_revision()?;
    let payload = json!({
        "path": store.dir,
        "project": store.project,
        "project_revision": project_revision,
        "recovery_scan": {
            "recovered": recovery_scan.recovered,
            "deferred": recovery_scan.deferred,
            "failed_closed": recovery_scan.failed_closed,
        },
    });
    activate_project_locked(state, store).await?;
    Ok(VerbResult::ok(payload))
}

/// project.list{sort?, q?} → the recent-projects index, reconciled against the
/// managed projects dir + the filesystem (missing `.cutproj`s flagged, un-indexed
/// ones discovered). READ-ONLY discovery — the reopen itself is `project.open`.
pub(super) async fn project_list(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        sort: Option<String>,
        q: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let sort = a.sort.as_deref().unwrap_or("recent");
    let managed = default_projects_dir();
    let projects = crate::projects_index::list(&managed, sort, a.q.as_deref())?;
    Ok(VerbResult::ok(json!({ "projects": projects })))
}

/// project.forget{id?|path?} → drop a project from the recent index. Does NOT delete
/// the `.cutproj` on disk (forget ≠ delete). Returns {forgotten:bool}.
pub(super) async fn project_forget(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        id: Option<String>,
        path: Option<String>,
        #[serde(default)]
        missing: bool,
    }
    let a: Args = parse_args(args)?;
    let mode_count =
        usize::from(a.id.is_some()) + usize::from(a.path.is_some()) + usize::from(a.missing);
    if mode_count != 1 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "project.forget needs exactly one of id, path, or missing:true",
            "pass the project's index id, its .cutproj path, or missing:true to bulk-forget every entry whose directory is gone",
        ));
    }
    // Bulk hygiene: drop every entry whose .cutproj is gone from disk RIGHT NOW
    // (per-entry stat — never the persisted `missing` flag, so a project on a
    // remounted drive survives). Nothing on disk is touched (forget ≠ delete).
    if a.missing {
        let removed = crate::projects_index::forget_missing()?;
        return Ok(VerbResult::ok(
            json!({ "forgotten": removed > 0, "removed": removed }),
        ));
    }
    let key = match (a.id, a.path) {
        (Some(id), _) => id,
        // Canonicalize the path so ANY equivalent form matches the stored canonical
        // entry — redundant separators (C:\\a\\b), a relative path, or the \\?\
        // verbatim prefix all collapse to the same string. Falls back to the raw
        // path when the .cutproj no longer exists on disk (forget.remove still
        // strips \\?\ via normalize_path, so a stored canonical entry still matches).
        (None, Some(path)) => Path::new(&path)
            .canonicalize()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(path),
        (None, None) => unreachable!("mode_count == 1 guarantees id or path here"),
    };
    let forgotten = crate::projects_index::forget(&key)?;
    Ok(VerbResult::ok(json!({ "forgotten": forgotten })))
}

/// project.delete{id?|path?} — PERMANENTLY delete a project's `.cutproj` directory
/// from disk AND drop it from the recent index (forget = index only; delete = the
/// files). GUARDRAILS so this never destroys the wrong thing: the resolved path
/// must be an EXISTING `*.cutproj` DIRECTORY, and it must NOT be the currently-open
/// project (open/create another first). Source MEDIA referenced by the project
/// lives OUTSIDE the `.cutproj` (linked by path) and is never touched — only the
/// project's own dir (edits, proxies, receipts) is removed. Returns
/// {deleted, path, forgotten}. Destructive: the UI must confirm before calling.
pub(super) async fn project_delete(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        id: Option<String>,
        path: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let _transition = state.project_transition.lock().await;
    if a.id.is_some() && a.path.is_some() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "project.delete needs exactly one of id or path",
            "pass the project's index id or its .cutproj path, not both",
        ));
    }
    // Keep the explicit id (if any) so we can forget the EXACT index entry the
    // caller pointed at, even if its stored path-string differs from the
    // canonicalized delete path (the source of the lingering "missing" ghost).
    let explicit_id = a.id.clone();
    let path = match (a.path, a.id) {
        (Some(p), _) => p,
        (None, Some(id)) => crate::projects_index::path_for(&id).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                "project not found in the index",
                format!("no project with id {id} in the recent index"),
            )
            .with_suggested_action(
                "pass the .cutproj path directly, or project.list to find the id",
            )
        })?,
        (None, None) => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "project.delete needs id or path",
                "pass the project's index id or its .cutproj path",
            ))
        }
    };
    let canon = Path::new(&path).canonicalize().map_err(|e| {
        CutError::new(
            error_codes::NOT_FOUND,
            "project path does not exist",
            format!("{path}: {e}"),
        )
        .with_suggested_action("it may already be deleted — project.forget drops the index entry")
    })?;
    // Guardrail: ONLY a *.cutproj directory — never an arbitrary path.
    if canon.extension().and_then(|e| e.to_str()) != Some("cutproj") || !canon.is_dir() {
        return Err(CutError::new(
            error_codes::GUARDRAIL,
            "refusing to delete: not a .cutproj directory",
            format!("{} is not a *.cutproj directory", canon.display()),
        )
        .with_suggested_action("project.delete only removes a project's own .cutproj directory"));
    }
    // Guardrail: never delete the project that is currently open (stale in-memory
    // state + lost edits). The caller must switch/create another project first.
    let open_guard = state.project.write().await;
    if let Some(store) = open_guard.as_ref() {
        let open = store
            .dir
            .canonicalize()
            .unwrap_or_else(|_| store.dir.clone());
        if open == canon {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "cannot delete the currently-open project",
                format!("{} is the open project", canon.display()),
            )
            .with_suggested_action("open or create another project first, then delete this one"));
        }
    }
    // Collect EVERY index entry that points at this directory BEFORE removing it,
    // comparing by canonicalized path (post-delete the path no longer resolves, so
    // the canonicalize comparison must happen now). This is what stops a stale
    // "missing" ghost from lingering when the stored creation path-string differs
    // from the canonicalized delete path (Windows casing/8.3/`\\?\` quirks).
    let mut victim_ids = crate::projects_index::ids_for_dir(&canon);
    if let Some(id) = explicit_id {
        if !victim_ids.contains(&id) {
            victim_ids.push(id);
        }
    }
    std::fs::remove_dir_all(&canon).map_err(|e| {
        CutError::new(
            error_codes::IO,
            "failed to delete the project directory",
            format!("{}: {e}", canon.display()),
        )
    })?;
    drop(open_guard);
    // Forget the captured entries by their exact ids; fall back to the legacy
    // path-string forget for any entry the id-match somehow missed.
    let mut forgotten = crate::projects_index::forget_ids(&victim_ids)? > 0;
    if crate::projects_index::forget(&crate::projects_index::normalize_path(
        &canon.to_string_lossy(),
    ))? {
        forgotten = true;
    }
    Ok(VerbResult::ok(json!({
        "deleted": true,
        "path": canon.to_string_lossy(),
        "forgotten": forgotten,
    })))
}

mod library_handlers;
mod library_mutations;

pub(super) use library_handlers::{library_add, library_list, library_relink};
pub(super) use library_mutations::{
    library_add_to_project, library_favorite, library_folder_add, library_folder_remove,
    library_folder_rename, library_move, library_remove, library_tag, library_use,
};

/// project.save{} — force-write the cache.
pub(super) async fn project_save(state: &AppState) -> Result<VerbResult, CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    store.save()?;
    Ok(VerbResult::ok(
        json!({"saved": true, "path": store.dir.join("project.json")}),
    ))
}

/// project.state{} — the full materialized project.json.
/// Build clip_id → CURRENT editable text for every title clip created by
/// `title.add` (folding any later `title.update` text edits, in op order).
/// Kinetic-caption titles are intentionally excluded — their text is the
/// transcript, not a single editable field. One forward pass over the op-log;
/// used by `project_state` to seed the Inspector's in-place title editor.
fn title_clip_texts(ops: &[OpRecord]) -> std::collections::BTreeMap<String, String> {
    use std::collections::{BTreeMap, HashSet};
    let added_clip = |op: &OpRecord| -> Option<String> {
        op.effects
            .iter()
            .find_map(|e| e.detail.get("added_clip").and_then(|v| v.as_str()))
            .map(|s| s.to_string())
    };
    let mut texts: BTreeMap<String, String> = BTreeMap::new();
    let mut kinetic: HashSet<String> = HashSet::new();
    for op in ops {
        match op.verb.as_str() {
            "title.add" => {
                if let (Some(c), Some(t)) =
                    (added_clip(op), op.args.get("text").and_then(|v| v.as_str()))
                {
                    texts.insert(c, t.to_string());
                }
            }
            "captions.kinetic" => {
                if let Some(c) = added_clip(op) {
                    kinetic.insert(c);
                }
            }
            "title.update" => {
                if let (Some(c), Some(t)) = (
                    op.args.get("clip").and_then(|v| v.as_str()),
                    op.args.get("text").and_then(|v| v.as_str()),
                ) {
                    if texts.contains_key(c) {
                        texts.insert(c.to_string(), t.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    for c in kinetic {
        texts.remove(&c);
    }
    texts
}

/// Build clip_id → CURRENT merged `edit.add_shape` args for every SHAPE overlay
/// clip created by `edit.add_shape` (folding any later `shape.update` overrides,
/// in op order — `apply_shape_overrides` reuses the same fold the verb uses, so
/// the annotation matches exactly what a re-render would produce). The merged
/// value is in edit.add_shape's arg shape (`shape`, `text`, `fill`/`stroke`/
/// `color`, …); `project_state` derives the Inspector seeds (`shape_kind`,
/// `shape_label`, `shape_color`) from it. One forward pass over the op-log.
/// Shapes and titles BOTH live on `title*` tracks, so this is the marker that
/// lets the Inspector route a shape clip to the shape editor (`shape.update`)
/// and a title clip to the title editor (`title.update`) — a clip created by
/// `edit.add_shape` appears HERE and never in `title_clip_texts`, and vice
/// versa. (shape-editing regression)
fn shape_clip_props(ops: &[OpRecord]) -> std::collections::BTreeMap<String, Value> {
    use std::collections::BTreeMap;
    let added_clip = |op: &OpRecord| -> Option<String> {
        op.effects
            .iter()
            .find_map(|e| e.detail.get("added_clip").and_then(|v| v.as_str()))
            .map(|s| s.to_string())
    };
    let mut merged: BTreeMap<String, Value> = BTreeMap::new();
    for op in ops {
        match op.verb.as_str() {
            "edit.add_shape" => {
                if let Some(c) = added_clip(op) {
                    merged.insert(c, op.args.clone());
                }
            }
            "shape.update" => {
                if let Some(c) = op.args.get("clip").and_then(|v| v.as_str()) {
                    if let Some(base) = merged.get_mut(c) {
                        super::edit_tools::apply_shape_overrides(base, &op.args);
                    }
                }
            }
            _ => {}
        }
    }
    merged
}

/// The shape's dominant VISIBLE color, for seeding the Inspector's color control:
/// the fill if set, else the stroke/line color, else the label text color. Pulled
/// from a merged `edit.add_shape` arg object.
fn shape_display_color(merged: &Value) -> Option<String> {
    for k in ["fill", "stroke", "color"] {
        if let Some(s) = merged.get(k).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

pub(super) fn full_project_state(store: &ProjectStore, sync: Value) -> Result<Value, CutError> {
    let mut v = serde_json::to_value(&store.project)?;
    // Annotate title overlay clips with their CURRENT editable text so the
    // Inspector can seed the in-place title editor (title.update). The op-log is
    // only read when the project actually HAS a non-empty title overlay track —
    // most projects pay nothing here.
    let has_titles = store
        .project
        .tracks
        .iter()
        .any(|t| t.id.starts_with("title") && !t.clips.is_empty());
    if has_titles {
        if let Ok(ops) = store.log.read_all() {
            let texts = title_clip_texts(&ops);
            // Shape clips share the `title*` tracks; annotate each with its
            // current editable props (shape_kind / shape_label / shape_color) so
            // the Inspector seeds the in-place shape editor (shape.update) and can
            // tell a shape clip apart from a title clip (a title carries title_text;
            // a shape carries shape_kind — never both).
            let shapes = shape_clip_props(&ops);
            if !texts.is_empty() || !shapes.is_empty() {
                if let Some(tracks) = v.get_mut("tracks").and_then(|t| t.as_array_mut()) {
                    for tr in tracks.iter_mut() {
                        let is_title = tr
                            .get("id")
                            .and_then(|i| i.as_str())
                            .map(|i| i.starts_with("title"))
                            .unwrap_or(false);
                        if !is_title {
                            continue;
                        }
                        if let Some(clips) = tr.get_mut("clips").and_then(|c| c.as_array_mut()) {
                            for cl in clips.iter_mut() {
                                let id =
                                    cl.get("id").and_then(|i| i.as_str()).map(|s| s.to_string());
                                if let Some(id) = id {
                                    if let Some(t) = texts.get(&id) {
                                        if let Some(obj) = cl.as_object_mut() {
                                            obj.insert(
                                                "title_text".into(),
                                                Value::String(t.clone()),
                                            );
                                        }
                                    }
                                    if let Some(m) = shapes.get(&id) {
                                        if let Some(obj) = cl.as_object_mut() {
                                            let kind = m
                                                .get("shape")
                                                .and_then(|x| x.as_str())
                                                .unwrap_or("rect");
                                            obj.insert(
                                                "shape_kind".into(),
                                                Value::String(kind.to_string()),
                                            );
                                            if let Some(label) =
                                                m.get("text").and_then(|x| x.as_str())
                                            {
                                                obj.insert(
                                                    "shape_label".into(),
                                                    Value::String(label.to_string()),
                                                );
                                            }
                                            if let Some(color) = shape_display_color(m) {
                                                obj.insert(
                                                    "shape_color".into(),
                                                    Value::String(color),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let has_media = store.project.tracks.iter().any(|track| {
        track
            .clips
            .iter()
            .any(|clip| matches!(clip, cut_core::Clip::Media(_)))
    });
    if has_media {
        if let Ok(ops) = store.log.read_all() {
            super::motion_link_projection::annotate_project_state(&mut v, &ops);
        }
    }
    if let Some(object) = v.as_object_mut() {
        object.insert(
            "project_revision".into(),
            serde_json::to_value(store.log.current_revision()?)?,
        );
        object.insert("sync".into(), sync);
    }
    Ok(v)
}

pub(super) async fn project_state(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    project_sync::project_state(state, args).await
}

fn sequence_summary(sequence: &cut_core::Sequence, active: &str) -> Value {
    json!({
        "id": sequence.id,
        "name": sequence.name,
        "active": sequence.id == active,
        "duration_ms": sequence.duration_ms(),
        "clip_count": sequence.clip_count(),
        "settings": sequence.settings,
    })
}

pub(super) async fn project_sequence_list(state: &AppState) -> Result<VerbResult, CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let active = store.project.active_sequence.as_str();
    let sequences = if store.project.sequences.is_empty() {
        vec![json!({
            "id": cut_core::DEFAULT_SEQUENCE_ID,
            "name": "Main",
            "active": true,
            "duration_ms": store.project.duration_ms(),
            "clip_count": store.project.tracks.iter().map(|track| track.clips.len()).sum::<usize>(),
            "settings": store.project.settings,
        })]
    } else {
        store
            .project
            .sequences
            .iter()
            .map(|sequence| sequence_summary(sequence, active))
            .collect()
    };
    Ok(VerbResult::ok(json!({
        "active_sequence": active,
        "sequences": sequences,
    })))
}

pub(super) async fn project_sequence_create(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        name: String,
        #[serde(default = "default_sequence_source")]
        from: String,
    }
    fn default_sequence_source() -> String {
        "empty".into()
    }
    let a: Args = parse_args(args.clone())?;
    let duplicate = match a.from.as_str() {
        "empty" => false,
        "active" => true,
        other => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown sequence source '{other}'"),
                "from must be empty or active",
            ))
        }
    };
    let rationale = args
        .get("rationale")
        .and_then(Value::as_str)
        .map(String::from);
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let (sequence, op) = guard_call("project.sequence_create", || {
        store.sequence_create(&a.name, duplicate, actor, rationale)
    })?;
    let op_id = op.op_id.clone();
    let result = sequence_summary(&sequence, &store.project.active_sequence);
    state.events.publish(Event::OpApplied { op });
    Ok(VerbResult::ok_with_ops(
        json!({"sequence": result, "active_sequence": store.project.active_sequence}),
        vec![op_id],
    ))
}

pub(super) async fn project_sequence_switch(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        id: String,
    }
    let a: Args = parse_args(args.clone())?;
    let rationale = args
        .get("rationale")
        .and_then(Value::as_str)
        .map(String::from);
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let op = guard_call("project.sequence_switch", || {
        store.sequence_switch(&a.id, actor, rationale)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op });
    Ok(VerbResult::ok_with_ops(
        json!({"active_sequence": store.project.active_sequence}),
        vec![op_id],
    ))
}

pub(super) async fn project_sequence_rename(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        id: String,
        name: String,
    }
    let a: Args = parse_args(args.clone())?;
    let rationale = args
        .get("rationale")
        .and_then(Value::as_str)
        .map(String::from);
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let op = guard_call("project.sequence_rename", || {
        store.sequence_rename(&a.id, &a.name, actor, rationale)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op });
    Ok(VerbResult::ok_with_ops(
        json!({"id": a.id, "name": a.name.trim()}),
        vec![op_id],
    ))
}

pub(super) async fn project_sequence_delete(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        id: String,
    }
    let a: Args = parse_args(args.clone())?;
    let rationale = args
        .get("rationale")
        .and_then(Value::as_str)
        .map(String::from);
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let op = guard_call("project.sequence_delete", || {
        store.sequence_delete(&a.id, actor, rationale)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op });
    Ok(VerbResult::ok_with_ops(
        json!({"deleted": true, "id": a.id}),
        vec![op_id],
    ))
}

/// project.ops{since?} — op-log records strictly AFTER `since`.
///
/// `since` accepts a checkpoint id/name OR a raw op id (the checkpoint-cursor contract — matches
/// `project.diff{from}` / `project.revert{to}`; before, only a raw op id worked,
/// and `since:"cp3"` failed with `op 'cp3' not found in log` even though the
/// checkpoint existed). Resolution goes through `cut_core::resolve_ref` (the
/// same rules the other ref-taking verbs use): a checkpoint resolves to its
/// `at_op`, so the returned ops are those AFTER the checkpoint — the obvious
/// "what changed since this checkpoint?" read.
pub(super) async fn project_ops(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    project_sync::project_ops(state, args).await
}

/// project.health{cursor?, revision?, limit?} — bounded path-free journal and
/// registered-media recovery state. Kept outside project.state because it
/// deliberately performs page-bounded filesystem checks.
pub(super) async fn project_health(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    project_health_read(state, args).await
}

/// project.close{} — save + drop the open project.
pub(super) async fn project_close(state: &AppState) -> Result<VerbResult, CutError> {
    let _transition = state.project_transition.lock().await;
    let previous = {
        let mut project = state.project.write().await;
        if let Some(store) = project.as_ref() {
            store.save()?;
        }
        project.take()
    };
    let closed_name = previous.as_ref().map(|store| store.project.name.clone());
    if let Err(error) = state.jobs.detach_project().await {
        *state.project.write().await = previous;
        return Err(error);
    }
    if closed_name.is_some() {
        state.events.publish(Event::ProjectChanged {
            open: false,
            name: closed_name,
        });
    }
    Ok(VerbResult::ok(json!({"closed": true})))
}

/// project.checkpoint{name, rationale?} — checkpoint IS an op (the append-only operation-log contract).
/// store.checkpoint mutates the project (todo-guarded), then the op commits.
pub(super) async fn project_checkpoint(
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
    // Core COMMITS the checkpoint op itself (store.checkpoint) — appending a
    // second op here double-logged every checkpoint and broke replay
    // determinism (duplicated checkpoints on rebuild).
    let (cp, op) = guard_call("project.checkpoint", || {
        store.checkpoint(&a.name, actor, rationale)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op });
    // Contract (verbs.json): {checkpoint: {id, name, at_op, ts}} — the bare
    // Checkpoint object was a result-shape drift.
    Ok(VerbResult::ok_with_ops(
        json!({"checkpoint": cp}),
        vec![op_id],
    ))
}

/// project.rename{name} — change the project's display name (label). Logged as a
/// non-timeline op (store.rename commits it) so it survives reopen/revert. The
/// .cutproj directory is not renamed. Returns {name}.
pub(super) async fn project_rename(
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
    let op = guard_call("project.rename", || store.rename(&a.name, actor, rationale))?;
    let op_id = op.op_id.clone();
    let name = store.project.name.clone();
    // Keep the global index's display name in sync (the .cutproj dir is unchanged).
    crate::projects_index::note_renamed(&store.dir.to_string_lossy(), &name)?;
    state.events.publish(Event::OpApplied { op });
    Ok(VerbResult::ok_with_ops(
        json!({ "name": name }),
        vec![op_id],
    ))
}

/// project.format{width?, height?, fps?} — set the timeline output resolution +
/// frame rate. Render output geometry/fps come from these, so a lower res/fps
/// can make renders + proxies faster on heavy footage.
/// At least one of width/height/fps is required; values are validated in
/// `ProjectStore::set_format`. Recorded as a metadata op (audit, like rename).
pub(super) async fn project_format(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        width: Option<u32>,
        height: Option<u32>,
        fps: Option<f64>,
    }
    let a: Args = parse_args(args.clone())?;
    if a.width.is_none() && a.height.is_none() && a.fps.is_none() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "nothing to set".to_string(),
            "pass width and/or height and/or fps",
        ));
    }
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let op = guard_call("project.format", || {
        store.set_format(a.width, a.height, a.fps, actor, rationale)
    })?;
    let op_id = op.op_id.clone();
    let s = store.project.settings.clone();
    state.events.publish(Event::OpApplied { op });
    Ok(VerbResult::ok_with_ops(
        json!({ "width": s.width, "height": s.height, "fps": s.fps }),
        vec![op_id],
    ))
}

/// project.color{working?, output?, rationale?} — set the project's color management:
/// the WORKING space the renderer composites/grades in and the OUTPUT space the
/// delivered file is tagged + encoded in. Supported spaces: rec709 (default),
/// rec2020, srgb, linear; an unknown name is rejected with an actionable error. At
/// least one of working/output is required. Recorded as a metadata op (audit, like
/// project.format / project.rename) — the config lives in the project file, so it
/// survives reopen/revert; rec709/rec709 restores the byte-identical default render.
pub(super) async fn project_color(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        working: Option<String>,
        output: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    if a.working.is_none() && a.output.is_none() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "nothing to set".to_string(),
            "pass working and/or output",
        )
        .with_suggested_action(format!(
            "supported color spaces: {}",
            cut_core::ColorSpace::SUPPORTED
        )));
    }
    // Validate each provided name up front so a typo errors actionably (not silently
    // ignored). None → leave that field unchanged.
    let parse_space = |label: &str,
                       v: &Option<String>|
     -> Result<Option<cut_core::ColorSpace>, CutError> {
        match v {
            None => Ok(None),
            Some(s) => cut_core::ColorSpace::parse(s).map(Some).ok_or_else(|| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("unknown {label} color space '{s}'"),
                    format!("supported spaces: {}", cut_core::ColorSpace::SUPPORTED),
                )
                .with_suggested_action(format!("pass one of: {}", cut_core::ColorSpace::SUPPORTED))
            }),
        }
    };
    let working = parse_space("working", &a.working)?;
    let output = parse_space("output", &a.output)?;
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let op = guard_call("project.color", || {
        store.set_color(working, output, actor, rationale)
    })?;
    let op_id = op.op_id.clone();
    let c = store.project.settings.color;
    state.events.publish(Event::OpApplied { op });
    Ok(VerbResult::ok_with_ops(
        json!({ "working": c.working.as_str(), "output": c.output.as_str() }),
        vec![op_id],
    ))
}

/// project.brand{brand?, clear?, rationale?} — replace or clear the durable
/// project-owned brand constraints. The full snapshot is recorded as a
/// non-timeline metadata op and therefore survives replay/reopen.
pub(super) async fn project_brand(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        brand: Option<cut_core::BrandKit>,
        #[serde(default)]
        clear: bool,
    }
    let a: Args = parse_args(args.clone())?;
    if a.clear && a.brand.is_some() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "project.brand cannot set and clear in the same call",
            "brand and clear:true are mutually exclusive",
        )
        .with_suggested_action("pass brand or clear:true"));
    }
    if !a.clear && a.brand.is_none() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "project.brand has nothing to change",
            "a complete brand snapshot or clear:true is required",
        )
        .with_suggested_action("pass brand:{...} or clear:true"));
    }
    let rationale = args
        .get("rationale")
        .and_then(Value::as_str)
        .map(String::from);
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let op = guard_call("project.brand", || {
        store.set_brand(a.brand, actor, rationale)
    })?;
    let op_id = op.op_id.clone();
    let brand = store.project.brand.clone();
    state.events.publish(Event::OpApplied { op });
    Ok(VerbResult::ok_with_ops(
        json!({ "brand": brand, "cleared": brand.is_none() }),
        vec![op_id],
    ))
}

/// comment.add{at_ms, text, end_ms?, author?, rationale?} — append a timecoded
/// review comment. A non-timeline metadata op (store.add_comment commits
/// it). Returns {comment}. `author` defaults to "client" (the canonical review
/// source); `end_ms` makes it a range comment.
pub(super) async fn comment_add(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        at_ms: u64,
        text: String,
        end_ms: Option<u64>,
        author: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    if let Some(end_ms) = a.end_ms {
        if end_ms < a.at_ms {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "comment end_ms must be greater than or equal to at_ms",
                format!("got at_ms={} end_ms={end_ms}", a.at_ms),
            ));
        }
    }
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let author = a.author.as_deref().unwrap_or("client");
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let (cm, op) = guard_call("comment.add", || {
        store.add_comment(a.at_ms, a.end_ms, &a.text, author, actor, rationale)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op });
    Ok(VerbResult::ok_with_ops(json!({"comment": cm}), vec![op_id]))
}

/// comment.list{status?} — review comments, optionally filtered by lifecycle
/// status ("open" | "addressed" | "dismissed"). Read-only, no op.
pub(super) async fn comment_list(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        status: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let comments: Vec<_> = store
        .project
        .comments
        .iter()
        .filter(|c| a.status.as_deref().is_none_or(|s| c.status == s))
        .collect();
    Ok(VerbResult::ok(
        json!({"comments": comments, "count": comments.len()}),
    ))
}

/// comment.resolve{comment_id, status, rationale?} — set a comment's lifecycle
/// status (open | addressed | dismissed). Non-timeline metadata op.
pub(super) async fn comment_resolve(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        comment_id: String,
        status: String,
    }
    let a: Args = parse_args(args.clone())?;
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let (cm, op) = guard_call("comment.resolve", || {
        store.resolve_comment(&a.comment_id, &a.status, actor, rationale)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op });
    Ok(VerbResult::ok_with_ops(json!({"comment": cm}), vec![op_id]))
}

const DRAFT_TIMEOUT_S: u64 = 200;

/// Locate the explicitly configured draft adapter. Drafting is an optional
/// integration, not a bundled ShellX Cut resource, so installed and source-tree
/// builds use the same contract. Tests point this variable at a fixture.
fn find_draft_adapter() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CUTD_DRAFT_ADAPTER") {
        if !p.is_empty() {
            let pb = PathBuf::from(&p);
            return pb.is_file().then_some(pb);
        }
    }
    None
}

/// The verb catalog the drafting agent may use — the ACTIONABLE verbs (edit /
/// transcript / captions), name + first-line description + args schema. Render/
/// project/ui/system verbs are excluded (a draft addresses the EDIT, not the
/// workflow). The engine re-validates names + shapes against this catalog after
/// the draft returns — the model's self-report of validity is never trusted.
fn draft_verb_catalog(registry: &crate::registry::VerbRegistry) -> Vec<Value> {
    registry
        .verbs
        .iter()
        .filter(|v| matches!(v.domain.as_str(), "edit" | "transcript" | "captions"))
        .map(|v| {
            let first = v.description.lines().next().unwrap_or("");
            json!({"name": v.name, "description": first, "args": v.args})
        })
        .collect()
}

/// Timeline digest for the drafting context: every clip whose timeline span is
/// within ±`window_ms` of the comment anchor, with the ids/tracks/ranges the
/// agent must anchor its edits to (it may NOT invent clip ids).
fn draft_timeline_context(project: &cut_core::Project, at_ms: u64, window_ms: u64) -> String {
    let edl = cut_core::edl_from_project(project);
    let lo = at_ms.saturating_sub(window_ms);
    let hi = at_ms + window_ms;
    let mut lines: Vec<String> = Vec::new();
    for seg in &edl.segments {
        if seg.timeline_out_ms < lo || seg.timeline_in_ms > hi {
            continue;
        }
        let id = seg.clip_id.as_deref().unwrap_or("(gap)");
        match (&seg.asset, seg.src_in_ms, seg.src_out_ms) {
            (Some(asset), Some(si), Some(so)) => lines.push(format!(
                "  clip {id} on {} — timeline [{},{}] ms, source [{},{}] of asset {asset}{}",
                seg.track,
                seg.timeline_in_ms,
                seg.timeline_out_ms,
                si,
                so,
                if seg.speed != 1.0 {
                    format!(", speed {}x", seg.speed)
                } else {
                    String::new()
                },
            )),
            _ => lines.push(format!(
                "  {id} on {} — timeline [{},{}] ms ({})",
                seg.track,
                seg.timeline_in_ms,
                seg.timeline_out_ms,
                if seg.caption_text.is_some() {
                    "caption"
                } else {
                    "gap"
                },
            )),
        }
    }
    if lines.is_empty() {
        "(no clips near this timecode)".into()
    } else {
        lines.join("\n")
    }
}

/// Best-effort transcript window for the drafting context: the spoken words
/// within ±`window_ms` (source time) of the comment anchor, on whatever asset
/// sits under `at_ms`. Empty string when there is no clip / no transcript at the
/// position (the agent still drafts from the timeline + comment text). Read-only.
async fn draft_transcript_context(
    state: &AppState,
    project: &cut_core::Project,
    at_ms: u64,
    window_ms: u64,
) -> String {
    // The base media segment under the comment (prefer audio — that's the speech).
    let edl = cut_core::edl_from_project(project);
    let seg = edl
        .segments
        .iter()
        .filter(|s| {
            s.asset.is_some()
                && s.timeline_in_ms <= at_ms
                && at_ms < s.timeline_out_ms
                && edl.is_audio_bearing_track(&s.track)
        })
        .min_by_key(|s| {
            if s.track_kind == cut_core::TrackKind::Audio {
                0
            } else {
                1
            }
        });
    let (Some(seg), Some(si)) = (seg, seg.and_then(|s| s.src_in_ms)) else {
        return String::new();
    };
    let Some(asset) = seg.asset.as_deref() else {
        return String::new();
    };
    // Reverse-map the timeline anchor to a SOURCE position through clip speed.
    let src_pos = si + cut_core::tl_off_to_src(at_ms.saturating_sub(seg.timeline_in_ms), seg.speed);
    let Ok(t) = load_transcript(state, asset).await else {
        return String::new();
    };
    let lo = src_pos.saturating_sub(window_ms);
    let hi = src_pos + window_ms;
    let words: Vec<String> = t
        .words
        .iter()
        .filter(|w| w.end_ms >= lo && w.start_ms <= hi)
        .map(|w| w.word.clone())
        .collect();
    if words.is_empty() {
        String::new()
    } else {
        format!("asset {asset}, source ~{src_pos} ms: …{}…", words.join(" "))
    }
}

/// Spawn the draft adapter with the context JSON on stdin and parse its
/// envelope (mirrors run_judge_adapter; stdin PIPED for the request). A missing
/// adapter / spawn failure / timeout / unparseable output all become an honest
/// envelope (status not_run | error) — never a fabricated draft.
async fn run_draft_adapter(adapter: Option<&Path>, context: &Value) -> Value {
    let Some(adapter) = adapter else {
        return json!({
            "status": "not_run",
            "reason": "draft adapter is not configured or does not point to a file (set CUTD_DRAFT_ADAPTER) - honest not_run, no drafting backend",
            "draft": Value::Null,
        });
    };
    let Some(python) = configured_adapter_python() else {
        return json!({
            "status": "not_run",
            "reason": format!("no adapter Python configured (set {ENV_ADAPTER_PYTHON} or install the ShellX Cut perception runtime) - honest not_run, no drafting backend"),
            "draft": Value::Null,
        });
    };

    let input = context.to_string();
    let mut cmd = tokio::process::Command::new(python);
    cmd.arg(adapter).arg("draft");
    let control = ProcessControl::for_operation(std::time::Duration::from_secs(DRAFT_TIMEOUT_S));
    let out = match run_owned(&mut cmd, Some(input.as_bytes()), &control).await {
        Ok(output) => output,
        Err(error) => match error.termination() {
            Some(ProcessTermination::DeadlineExceeded) => {
                return json!({"status": "error", "reason": format!("draft adapter exceeded {DRAFT_TIMEOUT_S}s timeout"), "draft": Value::Null})
            }
            Some(ProcessTermination::Cancelled(reason)) => {
                return json!({"status": "error", "reason": format!("draft adapter cancelled ({})", reason.label()), "draft": Value::Null})
            }
            None => {
                return json!({"status": "error", "reason": format!("draft adapter io error: {error}"), "draft": Value::Null})
            }
        },
    };
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: String = err
            .chars()
            .rev()
            .take(600)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        return json!({"status": "error", "reason": format!("draft adapter exit {:?}: {tail}", out.status.code()), "draft": Value::Null});
    }
    match serde_json::from_slice::<Value>(&out.stdout) {
        Ok(v) => v,
        Err(e) => {
            let s = String::from_utf8_lossy(&out.stdout);
            let head: String = s.chars().take(300).collect();
            json!({"status": "error", "reason": format!("draft adapter emitted non-JSON ({e}): {head}"), "draft": Value::Null})
        }
    }
}

/// Validate a draft's proposed verbs against the real registry: each `verb` must
/// exist AND be an actionable (edit/transcript/captions) verb the catalog
/// offered. Returns {ok, invalid:[{verb, why}]} — the model's self-report is
/// never trusted; comment.apply additionally dry-runs them.
fn validate_draft_verbs(registry: &crate::registry::VerbRegistry, draft: &Value) -> Value {
    let mut invalid: Vec<Value> = Vec::new();
    let steps = draft.get("verbs").and_then(|v| v.as_array());
    let n = steps.map(|s| s.len()).unwrap_or(0);
    if let Some(steps) = steps {
        for step in steps {
            let name = step.get("verb").and_then(|v| v.as_str()).unwrap_or("");
            match registry.get(name) {
                None => invalid.push(json!({"verb": name, "why": "unknown verb"})),
                Some(spec) if !matches!(spec.domain.as_str(), "edit" | "transcript" | "captions") => {
                    invalid.push(json!({"verb": name, "why": format!("'{}' is not an actionable edit verb", spec.domain)}))
                }
                Some(_) => {}
            }
        }
    }
    json!({"ok": invalid.is_empty(), "verb_count": n, "invalid": invalid})
}

/// comment.draft{comment_id} — ask the drafting agent (the verify.judge
/// claude-CLI ladder, reused) to propose a concrete change set for an OPEN
/// comment, given the timeline + transcript context + the verb catalog. The
/// proposal is STORED on the comment for review (NOT applied — comment.apply,
/// A later explicit apply executes it. Honest backend semantics: a missing/failed CLI yields
/// status not_run|error and stores NO draft (never a fabricated edit).
pub(super) async fn comment_draft(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        comment_id: String,
    }
    let a: Args = parse_args(args.clone())?;
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);

    // READ phase: clone the comment + build the context under a read guard.
    let context = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let cm = store
            .project
            .comments
            .iter()
            .find(|c| c.id == a.comment_id)
            .ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no comment '{}'", a.comment_id),
                    "comment ids come from comment.list",
                )
            })?
            .clone();
        let timeline = draft_timeline_context(&store.project, cm.at_ms, 6000);
        let transcript = draft_transcript_context(state, &store.project, cm.at_ms, 5000).await;
        let verbs = draft_verb_catalog(&state.registry);
        json!({"comment": cm, "timeline": timeline, "transcript": transcript, "verbs": verbs})
    };

    // Spawn the drafting agent (read-guard released — the CLI call is slow).
    let adapter = find_draft_adapter();
    let envelope = run_draft_adapter(adapter.as_deref(), &context).await;
    let status = envelope
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("error");
    if status != "completed" {
        // Honest not_run/error — no draft stored, no op. Surface the reason.
        return Ok(VerbResult::ok(json!({
            "comment_id": a.comment_id,
            "status": status,
            "reason": envelope.get("reason"),
            "backend": envelope.get("backend"),
        })));
    }

    let raw = envelope.get("draft").cloned().unwrap_or(Value::Null);
    let validation = validate_draft_verbs(&state.registry, &raw);
    let stored_draft = json!({
        "verbs": raw.get("verbs").cloned().unwrap_or(json!([])),
        "rationale": raw.get("rationale").cloned().unwrap_or(Value::Null),
        "confidence": raw.get("confidence").cloned().unwrap_or(Value::Null),
        "backend": envelope.get("backend"),
        "validation": validation,
        "ts": OpRecord::now_ts(),
    });

    // WRITE phase: store the draft on the comment (a metadata op).
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let (cm, op) = guard_call("comment.draft", || {
        store.set_comment_draft(&a.comment_id, stored_draft.clone(), actor, rationale)
    })?;
    let op_id = op.op_id.clone();
    let draft_out = cm.draft.clone();
    state.events.publish(Event::OpApplied { op });
    Ok(VerbResult::ok_with_ops(
        json!({"comment_id": a.comment_id, "status": "completed", "draft": draft_out}),
        vec![op_id],
    ))
}

/// comment.apply{comment_id} — EXECUTE a comment's drafted change.
/// Wraps the apply in an auto-checkpoint (one-click revert as a unit), dispatches
/// each drafted verb as a real op carrying the comment as its rationale (the
/// receipt linking edit→comment), computes the before/after project.diff (the
/// review artifact), and marks the comment addressed. Stops on the first failing
/// verb and hands back the checkpoint to revert the partial apply — a half-applied
/// draft is never left silently. Reuses the existing checkpoint / diff / revert
/// machinery; the diff + checkpoint ARE the review-after-apply.
pub(super) async fn comment_apply(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        comment_id: String,
    }
    let a: Args = parse_args(args.clone())?;

    // Load the comment + its drafted verbs (read guard).
    let (cm_text, verbs) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let cm = store
            .project
            .comments
            .iter()
            .find(|c| c.id == a.comment_id)
            .ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no comment '{}'", a.comment_id),
                    "comment ids come from comment.list",
                )
            })?;
        let draft = cm.draft.as_ref().ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                format!("comment '{}' has no draft to apply", a.comment_id),
                "run comment.draft first to propose a change",
            )
        })?;
        let verbs: Vec<Value> = draft
            .get("verbs")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        (cm.text.clone(), verbs)
    };
    if verbs.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("comment '{}' draft has no verbs to apply", a.comment_id),
            "the draft was an honest no-op — dismiss the comment (comment.resolve status:dismissed) or re-draft",
        ));
    }
    // Re-validate against the live registry before executing anything.
    let validation = validate_draft_verbs(&state.registry, &json!({"verbs": verbs}));
    if validation.get("ok") != Some(&Value::Bool(true)) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "the draft contains verbs that are not valid in this build",
            "re-run comment.draft; the proposed verbs failed registry validation",
        )
        .with_suggested_action(validation.to_string()));
    }

    // Auto-checkpoint so the whole apply is one revertible unit.
    let cp_name = format!("before-apply-{}", a.comment_id);
    let cp_res = Box::pin(dispatch(
        state,
        "project.checkpoint",
        json!({"name": cp_name, "rationale": format!("auto: before applying comment {} draft", a.comment_id)}),
        actor.clone(),
    ))
    .await;
    if !cp_res.ok {
        return Ok(cp_res);
    }
    let checkpoint_id = cp_res
        .result
        .as_ref()
        .and_then(|r| r["checkpoint"]["id"].as_str())
        .unwrap_or_default()
        .to_string();

    // Dispatch each drafted verb as a real op, the comment as its rationale.
    let mut applied: Vec<Value> = Vec::new();
    for step in &verbs {
        let verb = step.get("verb").and_then(|v| v.as_str()).unwrap_or("");
        let mut vargs = step.get("args").cloned().unwrap_or_else(|| json!({}));
        if let Value::Object(m) = &mut vargs {
            m.insert(
                "rationale".into(),
                json!(format!("addressing comment {}: {}", a.comment_id, cm_text)),
            );
        }
        let res = Box::pin(dispatch(state, verb, vargs, actor.clone())).await;
        let ok = res.ok;
        applied.push(json!({"verb": verb, "ok": ok, "result": res.result, "error": res.error}));
        if !ok {
            // Stop on the first failure — never leave a half-applied draft
            // silently. The checkpoint reverts the partial apply in one step.
            return Ok(VerbResult::ok(json!({
                "comment_id": a.comment_id,
                "status": "failed",
                "failed_verb": verb,
                "applied": applied,
                "checkpoint": checkpoint_id,
                "revert_hint": format!("a verb failed mid-apply — project.revert{{to:\"{checkpoint_id}\"}} undoes the partial apply"),
            })));
        }
    }

    // The before/after diff (the review artifact) — from the checkpoint to the tip.
    let tip = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        store.log.read_all()?.last().map(|o| o.op_id.clone())
    };
    let diff = match tip {
        Some(tip) => {
            Box::pin(dispatch(
                state,
                "project.diff",
                json!({"from": checkpoint_id, "to": tip}),
                actor.clone(),
            ))
            .await
            .result
        }
        None => None,
    };

    // Mark the comment addressed (its drafted change is now in the timeline).
    let resolved = Box::pin(dispatch(
        state,
        "comment.resolve",
        json!({"comment_id": a.comment_id, "status": "addressed", "rationale": "draft applied"}),
        actor.clone(),
    ))
    .await;
    if !resolved.ok {
        return Ok(VerbResult::ok(json!({
            "comment_id": a.comment_id,
            "status": "applied_comment_update_failed",
            "applied": applied,
            "diff": diff,
            "checkpoint": checkpoint_id,
            "comment_error": resolved.error,
            "revert_hint": format!("project.revert{{to:\"{checkpoint_id}\"}} undoes this whole apply"),
        })));
    }

    Ok(VerbResult::ok(json!({
        "comment_id": a.comment_id,
        "status": "addressed",
        "applied": applied,
        "diff": diff,
        "checkpoint": checkpoint_id,
        "revert_hint": format!("project.revert{{to:\"{checkpoint_id}\"}} undoes this whole apply"),
    })))
}

/// project.revert{to, if_tip?, rationale?} — appends one atomic, tip-undoable
/// restore op and publishes it without rewriting history (the append-only operation-log contract).
pub(super) async fn project_revert(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        to: String,
        if_tip: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    review_handoff::require_history_tip(store, a.if_tip.as_deref())?;
    let new_op_ids = guard_call("project.revert", || store.revert(&a.to, actor))?;
    // Publish the appended restore ops (read back from the log tail).
    let all = store.log.read_all()?;
    for op in all.iter().filter(|o| new_op_ids.contains(&o.op_id)) {
        state.events.publish(Event::OpApplied { op: op.clone() });
    }
    Ok(VerbResult::ok_with_ops(
        json!({"reverted_to": a.to}),
        new_op_ids,
    ))
}

/// project.undo{} — step the linear history cursor back one edit (Ctrl+Z). The
/// core fixes the old oscillation (a 2nd undo redid); here we just append the
/// nav op, publish it, and hand back the new cursor/availability for the UI.
pub(super) async fn project_undo(state: &AppState, actor: Actor) -> Result<VerbResult, CutError> {
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let op = guard_call("project.undo", || store.undo(actor))?;
    let op_id = op.op_id.clone();
    let to_op = op
        .args
        .get("to_op")
        .and_then(|v| v.as_str())
        .map(String::from);
    state.events.publish(Event::OpApplied { op });
    Ok(VerbResult::ok_with_ops(
        json!({
            "to_op": to_op,
            "cursor": store.undo_pos,
            "undo_available": store.undo_available(),
            "redo_available": store.redo_available(),
        }),
        vec![op_id],
    ))
}

/// project.redo{} — step the linear history cursor forward one edit
/// (Ctrl+Shift+Z / Ctrl+Y). Counterpart of project.undo.
pub(super) async fn project_redo(state: &AppState, actor: Actor) -> Result<VerbResult, CutError> {
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let op = guard_call("project.redo", || store.redo(actor))?;
    let op_id = op.op_id.clone();
    let to_op = op
        .args
        .get("to_op")
        .and_then(|v| v.as_str())
        .map(String::from);
    state.events.publish(Event::OpApplied { op });
    Ok(VerbResult::ok_with_ops(
        json!({
            "to_op": to_op,
            "cursor": store.undo_pos,
            "undo_available": store.undo_available(),
            "redo_available": store.redo_available(),
        }),
        vec![op_id],
    ))
}

/// project.diff{from, to} — ops between two refs + computed summary.
pub(super) async fn project_diff(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        from: String,
        to: String,
    }
    let a: Args = parse_args(args)?;
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let ops = store.log.read_all()?;
    let summary = guard_call("project.diff", || {
        cut_core::diff(&store.project, &ops, &a.from, &a.to)
    })?;
    Ok(VerbResult::ok(serde_json::to_value(&summary)?))
}

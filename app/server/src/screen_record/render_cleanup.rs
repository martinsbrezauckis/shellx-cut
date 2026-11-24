//! Recording renders are retired only with a local, take-bound origin record.
//! Received projects cannot supply that record. Legacy/shared bakes stay put.
use super::{
    render_files::{self, Identity},
    take_edit,
};
use cut_core::{recording::RecordingTake, Clip, CutError, Project};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
struct Origin {
    take: String,
    source: PathBuf,
    path: PathBuf,
    identity: Identity,
    /// The creation identity of the project this render was made in (see
    /// `project_identity`). A record without it (older build) is never removed.
    #[serde(default)]
    project: String,
}

/// What the engine wrote into the project's journal when it created the
/// project: its first record (`project.create`, with the creation time to the
/// millisecond). The journal is append-only and creation is not undoable, so
/// the line never changes for the life of the project. The origin records live
/// outside the project (app home) and carry a copy of this value, so a project
/// that replaces this folder with its own journal has a different first record
/// and cannot claim the renders of the project it replaced.
fn project_identity(dir: &Path) -> Option<String> {
    use std::io::BufRead;
    let file = std::fs::File::open(dir.join("ops.jsonl")).ok()?;
    let mut line = Vec::new();
    std::io::BufReader::new(file.take(64 * 1024 + 1))
        .read_until(b'\n', &mut line)
        .ok()?;
    if line.len() > 64 * 1024 {
        return None;
    }
    let record: Value = serde_json::from_slice(&line).ok()?;
    (record["verb"] == "project.create").then(|| key(line.trim_ascii()))
}

fn key(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}
fn same_lexical(a: &Path, b: &Path) -> bool {
    super::strip_verbatim_prefix(a) == super::strip_verbatim_prefix(b)
}
fn root_for_id(dir: &Path, take_id: &str) -> PathBuf {
    dir.join("cache/screen_record/renders")
        .join(key(take_id.as_bytes()))
}
fn root(dir: &Path, take: &RecordingTake) -> PathBuf {
    root_for_id(dir, &take.id)
}
fn registry(dir: &Path) -> Option<PathBuf> {
    let dir = super::strip_verbatim_prefix(dir);
    Some(
        crate::userdata::shellx_cut_home()?
            .join("recording-render-origins")
            .join(format!("{}.json", key(dir.as_os_str().as_encoded_bytes()))),
    )
}
fn load(dir: &Path) -> Vec<Origin> {
    let Some(path) = registry(dir) else {
        return vec![];
    };
    let Ok(file) = std::fs::File::open(path) else {
        return vec![];
    };
    let mut bytes = Vec::new();
    if file
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() > 4 * 1024 * 1024
    {
        return vec![];
    }
    serde_json::from_slice::<Vec<Origin>>(&bytes)
        .ok()
        .filter(|v| v.len() <= 20_000)
        .unwrap_or_default()
}
fn save(dir: &Path, origins: &[Origin]) -> Result<(), CutError> {
    let path = registry(dir)
        .ok_or_else(|| take_edit::bad("recording-render ownership storage is unavailable"))?;
    std::fs::create_dir_all(path.parent().expect("registry parent"))?;
    crate::output_paths::write_output_atomic(&path, &serde_json::to_vec(origins)?)
}

fn ensure_render_dirs(dir: &Path, folder: &Path) -> Result<(), CutError> {
    let cache = super::screen_record_cache_dir(dir)?;
    for path in [cache.join("renders"), folder.to_path_buf()] {
        match std::fs::create_dir(&path) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
        cut_core::matte_cache::require_plain_directory(&path)?;
    }
    Ok(())
}

pub(super) fn destination(
    dir: &Path,
    take: &RecordingTake,
    name: &std::ffi::OsStr,
) -> Result<PathBuf, CutError> {
    let folder = root(dir, take);
    ensure_render_dirs(dir, &folder)?;
    Ok(folder.join(name))
}

pub(crate) fn polish_destination(
    dir: &Path,
    cache: &Path,
    source_dir: &Path,
    key: &str,
    raw: bool,
) -> Result<(Option<String>, PathBuf), CutError> {
    let name = format!("{key}.mp4");
    if raw || !source_dir.join("events.json").exists() {
        return Ok((None, cache.join(name)));
    }
    let id = take_edit::take_id_for_capture(dir, source_dir)?;
    let folder = root_for_id(dir, &id);
    ensure_render_dirs(dir, &folder)?;
    Ok((Some(id), folder.join(name)))
}

/// Called only after a newly encoded render passed probe, duration and hash.
/// Never adopt a cached/received file by trusting its name or project metadata.
fn remember_take(
    dir: &Path,
    take_id: &str,
    source_path: &Path,
    path: &Path,
) -> Result<(), CutError> {
    let identity = render_files::access(path, None, false)?;
    let project = project_identity(dir)
        .ok_or_else(|| take_edit::bad("the project's creation record cannot be read"))?;
    let mut origins = load(dir);
    origins.retain(|o| !same_lexical(&o.path, path));
    if origins.len() >= 20_000 {
        return Err(take_edit::bad(
            "recording-render ownership list is full; review old renders in Settings",
        ));
    }
    origins.push(Origin {
        take: take_id.to_owned(),
        source: source_path.to_owned(),
        path: path.to_owned(),
        identity,
        project,
    });
    save(dir, &origins)
}

pub(super) fn remember(dir: &Path, take: &RecordingTake, path: &Path) -> Result<(), CutError> {
    remember_take(dir, &take.id, &resolved(dir, &take.source), path)
}

pub(crate) fn remember_polish(
    dir: &Path,
    take_id: Option<&str>,
    source: &Path,
    baked: &Path,
    cached: bool,
) -> Result<(), CutError> {
    if cached {
        return Ok(());
    }
    if let Some(id) = take_id {
        remember_take(dir, id, source, baked)?;
    }
    Ok(())
}

fn resolved(dir: &Path, value: &str) -> PathBuf {
    let p = Path::new(value);
    if p.is_absolute() {
        p.to_owned()
    } else {
        dir.join(p)
    }
}
fn same_file(dir: &Path, value: &str, candidate: &Path) -> bool {
    let path = resolved(dir, value);
    same_lexical(&path, candidate)
        || path
            .canonicalize()
            .ok()
            .zip(candidate.canonicalize().ok())
            .is_some_and(|(a, b)| a == b)
}
fn take_uses_render(dir: &Path, project: &Project, take: &RecordingTake, path: &Path) -> bool {
    project
        .assets
        .get(&take.asset)
        .is_some_and(|a| same_file(dir, &a.path, path))
        || take
            .camera_asset
            .as_deref()
            .and_then(|id| project.assets.get(id))
            .is_some_and(|a| same_file(dir, &a.path, path))
}
fn referenced(dir: &Path, project: &Project, path: &Path) -> bool {
    let tracks_use = |tracks: &[cut_core::Track]| {
        tracks.iter().flat_map(|t| &t.clips).any(|c| {
        matches!(c, Clip::Media(m) if project.assets.get(&m.asset).is_some_and(|a| same_file(dir, &a.path, path)))
    })
    };
    tracks_use(&project.tracks)
        || project.nests.iter().any(|n| tracks_use(&n.tracks))
        || project
            .sequences
            .iter()
            .any(|s| tracks_use(&s.tracks) || s.nests.iter().any(|n| tracks_use(&n.tracks)))
        || project
            .recordings
            .iter()
            .any(|t| take_uses_render(dir, project, t, path))
}
fn contains_input(dir: &Path, value: &Value, path: &Path) -> bool {
    match value {
        Value::String(s) => same_file(dir, s, path),
        Value::Array(items) => items.iter().any(|v| contains_input(dir, v, path)),
        Value::Object(items) => items.values().any(|v| contains_input(dir, v, path)),
        _ => false,
    }
}
fn reason(dir: &Path, project: &Project, origin: &Origin) -> Option<&'static str> {
    if project_identity(dir).is_none_or(|now| now != origin.project) {
        return Some("recorded for a different project at this folder");
    }
    let Some(take) = project
        .recordings
        .iter()
        .find(|t| t.id == origin.take && same_lexical(&resolved(dir, &t.source), &origin.source))
    else {
        return Some("take ownership cannot be verified");
    };
    if !origin
        .path
        .parent()
        .is_some_and(|p| same_lexical(p, &root(dir, take)))
    {
        return Some("outside this take's render folder");
    }
    if contains_input(
        dir,
        &serde_json::to_value(&project.recordings).unwrap_or(Value::Null),
        &origin.path,
    ) {
        return Some("an original recording or recording input");
    }
    if referenced(dir, project, &origin.path) {
        return Some("still used by a clip or take");
    }
    if render_files::access(&origin.path, Some(&origin.identity), false).is_err() {
        return Some("file changed or its folder cannot be safely opened");
    }
    None
}
/// A name safe to show: relative to the project folder, never an absolute path.
fn shown(dir: &Path, path: &Path) -> String {
    let (dir, path) = (
        super::strip_verbatim_prefix(dir),
        super::strip_verbatim_prefix(path),
    );
    match path.strip_prefix(&dir) {
        Ok(inside) => inside.display().to_string(),
        Err(_) => format!(
            "outside project/{}",
            path.file_name().unwrap_or_default().to_string_lossy()
        ),
    }
}

/// The file's size now (no content read). A link, a missing or a changed
/// kind of entry falls back to the recorded size, which the reply marks.
fn current_bytes(origin: &Origin) -> (u64, bool) {
    match std::fs::symlink_metadata(&origin.path) {
        Ok(m) if m.is_file() => (m.len(), false),
        _ => (origin.identity.bytes, true),
    }
}

fn kept(reason: &str) -> Value {
    json!({"status":"kept", "reason":reason, "message":format!("Earlier render kept: {reason}")})
}

/// On Windows another reader (a probe, a scanner) can hold the old render open
/// without sharing deletion for a moment. Retry only those refusals (sharing
/// violation 32, access denied 5) for a bounded time; a file that stays open is
/// kept, never forced. Everything else fails at once.
fn remove_with_short_retry(
    path: &Path,
    identity: &render_files::Identity,
) -> std::io::Result<render_files::Identity> {
    const ATTEMPTS: u32 = 30;
    for attempt in 1..=ATTEMPTS {
        match render_files::access(path, Some(identity), true) {
            Err(error)
                if cfg!(windows)
                    && matches!(error.raw_os_error(), Some(5 | 32))
                    && attempt < ATTEMPTS =>
            {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            other => return other,
        }
    }
    unreachable!("the last attempt always returns")
}

fn owned_sidecar_for_render(
    dir: &Path,
    project: &Project,
    render_path: &Path,
) -> Option<(PathBuf, render_files::Identity)> {
    let sidecar = super::clip_markers::baked_sidecar_path(render_path);
    std::fs::symlink_metadata(&sidecar).ok()?;
    if referenced(dir, project, &sidecar) {
        return None;
    }
    let identity = render_files::access(&sidecar, None, false).ok()?;
    let render_hash = cut_core::hash_file(render_path).ok()?;
    super::clip_markers::is_owned_baked_sidecar(&sidecar, &render_hash)
        .then_some((sidecar, identity))
}

/// The only recording-render deletion entry point. Both automatic cleanup and
/// confirmed Settings cleanup recheck ownership, every input and all live clips.
fn remove_previous_render(dir: &Path, project: &Project, origin: &Origin) -> Value {
    if let Some(why) = reason(dir, project, origin) {
        return kept(why);
    }
    let sidecar = owned_sidecar_for_render(dir, project, &origin.path);
    match remove_with_short_retry(&origin.path, &origin.identity) {
        Ok(identity) => {
            if let Some((sidecar_path, sidecar_identity)) = sidecar {
                let _ = remove_with_short_retry(&sidecar_path, &sidecar_identity);
            }
            json!({"status":"removed", "bytes":identity.bytes, "message":format!("Earlier render removed ({:.1} MB)", identity.bytes as f64 / 1_000_000.0)})
        }
        Err(error) if cfg!(windows) && matches!(error.raw_os_error(), Some(5 | 32)) => {
            kept("it was in use")
        }
        Err(_) => kept("file changed or removal was not permitted"),
    }
}

#[cfg(test)]
pub(super) fn retire(
    dir: &Path,
    project: &Project,
    take: &RecordingTake,
    old: Option<&Path>,
    new: &Path,
) -> Value {
    let Some(old) = old else {
        return kept("earlier video is missing");
    };
    if same_lexical(old, new) {
        return kept("this is still the current render");
    }
    let mut origins = load(dir);
    let take_source = resolved(dir, &take.source);
    let Some(index) = origins.iter().position(|o| {
        same_lexical(&o.path, old) && o.take == take.id && same_lexical(&o.source, &take_source)
    }) else {
        return kept("no independent ownership record for this take");
    };
    let mut outcome = remove_previous_render(dir, project, &origins[index]);
    if outcome["status"] == "removed" {
        origins.remove(index);
        if save(dir, &origins).is_err() {
            outcome["warning"] = json!("The video was removed, but its ownership list could not be updated. Refresh recording storage in Settings.");
        }
    }
    outcome
}

/// True when any part of the project other than the asset registry names the
/// asset id (clip, track, sequence, compound, take, marker, adjustment, ...).
/// Deliberately wide: any string equal to the id counts, so doubt keeps the asset.
fn asset_named_elsewhere(project: &Project, asset_id: &str) -> bool {
    fn names(value: &Value, id: &str) -> bool {
        match value {
            Value::String(s) => s == id,
            Value::Array(items) => items.iter().any(|v| names(v, id)),
            Value::Object(items) => items.iter().any(|(k, v)| k == id || names(v, id)),
            _ => false,
        }
    }
    let Ok(mut value) = serde_json::to_value(project) else {
        return true;
    };
    if let Some(object) = value.as_object_mut() {
        object.remove("assets");
    }
    names(&value, asset_id)
}

/// After Cut removed an old render file, drop the asset that pointed at it
/// through the project's journal (`media.remove`), but only when nothing else
/// names it. Returns the op when the asset was dropped.
pub(super) fn forget_removed_asset(
    store: &mut cut_core::ProjectStore,
    asset_id: &str,
    removed: &Path,
    actor: cut_core::Actor,
) -> Option<cut_core::OpRecord> {
    let dir = store.dir.clone();
    let asset = store.project.assets.get(asset_id)?;
    if !same_file(&dir, &asset.path, removed) || asset_named_elsewhere(&store.project, asset_id) {
        return None;
    }
    if store
        .project
        .assets
        .iter()
        .any(|(id, other)| id != asset_id && same_file(&dir, &other.path, removed))
    {
        return None;
    }
    store
        .record_remove_asset(
            asset_id,
            actor,
            Some("old recording render removed in Settings".into()),
        )
        .ok()
        .map(|(_, op)| op)
}

fn legacy_inventory(dir: &Path, project: &Project) -> Value {
    let folder = dir.join("cache/screen_record");
    let mut files = Vec::new();
    let mut bytes = 0u64;
    let mut reveal_asset = None;
    let owned: std::collections::HashSet<_> = load(dir)
        .into_iter()
        .map(|o| super::strip_verbatim_prefix(&o.path))
        .collect();
    let mut seen = std::collections::HashSet::new();
    let mut folders = vec![folder.clone()];
    folders.extend(project.recordings.iter().take(20_000).map(|t| root(dir, t)));
    let mut remaining = 20_000usize;
    for scan in folders {
        if remaining == 0 {
            break;
        }
        if !scan
            .ancestors()
            .take_while(|p| p.starts_with(dir))
            .all(|p| cut_core::matte_cache::require_plain_directory(p).is_ok())
        {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&scan) else {
            continue;
        };
        for entry in entries.take(remaining).flatten() {
            remaining = remaining.saturating_sub(1);
            let path = entry.path();
            let normalized = super::strip_verbatim_prefix(&path);
            if owned.contains(&normalized) || !seen.insert(normalized) {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !(name.starts_with("sha256_") || name.starts_with("sha256s_"))
                || !name.ends_with(".mp4")
            {
                continue;
            }
            if referenced(dir, project, &path) {
                continue;
            }
            // No contents are read, even for a planted huge legacy leaf.
            let Ok(identity) = render_files::access(&path, None, false) else {
                continue;
            };
            bytes = bytes.saturating_add(identity.bytes);
            files.push(shown(dir, &path));
            if reveal_asset.is_none() {
                // "Show in folder" opens the folder of a registered file that
                // lies next to the kept renders; no path is sent to anyone.
                reveal_asset = project
                    .assets
                    .iter()
                    .find(|(_, a)| {
                        resolved(dir, &a.path)
                            .parent()
                            .zip(path.parent())
                            .is_some_and(|(x, y)| same_lexical(x, y))
                    })
                    .map(|(id, _)| id.clone());
            }
        }
    }
    json!({"files":files,"bytes":bytes,"reveal_asset":reveal_asset,"reason":"older or unrecorded renders; the editor does not remove them"})
}

fn inventory(dir: &Path, project: &Project) -> (Vec<Origin>, Value) {
    let origins = load(dir);
    let mut eligible = Vec::new();
    let mut files = Vec::new();
    let mut bytes = 0u64;
    for origin in origins {
        // The current render is not an old render. Used old renders remain
        // listed with their refusal reason; they can become eligible later.
        if project
            .recordings
            .iter()
            .any(|t| take_uses_render(dir, project, t, &origin.path))
        {
            continue;
        }
        if std::fs::symlink_metadata(&origin.path)
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            continue;
        }
        let why = reason(dir, project, &origin);
        let (size, recorded) = current_bytes(&origin);
        bytes = bytes.saturating_add(size);
        files.push(json!({"name":shown(dir, &origin.path),"bytes":size,"size_is_recorded":recorded,"reason":why,"removable":why.is_none()}));
        if why.is_none() {
            eligible.push(origin);
        }
    }
    let plan_id = key(&serde_json::to_vec(&(dir, &eligible)).unwrap_or_default());
    let value = json!({"files":files,"bytes":bytes,"plan_id":plan_id,"removable_files":eligible.len(),"legacy":legacy_inventory(dir, project)});
    (eligible, value)
}

pub(crate) async fn handle(
    state: &crate::state::AppState,
    args: Value,
    remove: bool,
    actor: cut_core::Actor,
) -> Result<cut_core::VerbResult, CutError> {
    let _transition = state.project_transition.lock().await;
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(crate::dispatch::no_project)?;
    let (targets, preview) = inventory(&store.dir, &store.project);
    if !remove {
        return Ok(cut_core::VerbResult::ok(preview));
    }
    if args["confirm"] != true || args["plan_id"] != preview["plan_id"] {
        return Err(take_edit::bad("old recording render list changed; preview again before removing").with_suggested_action("Call screen_record.render_cache_preview, review the listed files, then confirm its plan_id."));
    }
    let results: Vec<_> = targets
        .iter()
        .map(|o| remove_previous_render(&store.dir, &store.project, o))
        .collect();
    let removed: Vec<_> = targets
        .iter()
        .zip(&results)
        .filter(|(_, r)| r["status"] == "removed")
        .map(|(o, _)| o.path.clone())
        .collect();
    let mut origins = load(&store.dir);
    origins.retain(|o| !removed.iter().any(|r| same_lexical(&o.path, r)));
    for path in &removed {
        let ids: Vec<String> = store.project.assets.keys().cloned().collect();
        for id in ids {
            if let Some(op) = forget_removed_asset(store, &id, path, actor.clone()) {
                state.events.publish(crate::events::Event::OpApplied { op });
            }
        }
    }
    let warning = save(&store.dir, &origins).err().map(|_| "Some videos were removed, but their ownership list could not be updated. Refresh recording storage.");
    Ok(cut_core::VerbResult::ok(
        json!({"results":results,"warning":warning}),
    ))
}

#[cfg(test)]
mod tests;

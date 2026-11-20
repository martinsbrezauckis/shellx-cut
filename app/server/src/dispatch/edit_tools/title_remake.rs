//! Titles and shapes are drawn by Cut into transparent movies in the project's
//! `titles/` folder: derived files, like proxies, made from the title's own
//! settings in the edit history (`title.add` / `title.update`,
//! `edit.add_shape` / `shape.update`). When such a movie is missing (deleted,
//! or a project copied without its `titles/` folder), Cut makes it again from
//! those settings instead of showing a hole in the picture. When it cannot,
//! the error names the title in plain words and says why, never an asset id
//! alone.
//!
//! Two callers: a project open (a background `title_remake` job, so the
//! preview's live overlay gets its file and every client re-reads the project
//! when the job ends), and every media read (`snapshot_for_media_io`: exact
//! frames, renders, exports), which makes a missing one before reading.

use super::title_movie_path::{title_movie, writable_titles_folder, TitleMovie};
pub(crate) use super::title_proof::begin_open_title_check;
use super::title_proof::{
    open_check_pending, present_unproven, resolve_spec, set_open_check, title_binding,
};
use super::*;
use std::path::{Path, PathBuf};

/// One title or shape movie a timeline clip shows and whose file is missing.
#[derive(Debug, Clone)]
pub(super) struct MissingOverlay {
    pub(super) asset: String,
    pub(super) clip: String,
    /// Where it is drawn again: built from the project folder, or `None` when
    /// the recorded path is not in this project's titles folder (never written).
    pub(super) path: Option<PathBuf>,
}

/// Remakes run one at a time: two exact-frame requests for the same missing
/// title must not encode it twice into the same file.
static REMAKE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The last remake that could not be done: (project folder, missing assets,
/// history length) and its error. Asking again for the same project state
/// returns that error without a new job (scrubbing must not pile up failed
/// jobs); an edit or a restored file is a new state and is tried again.
type FailedRemake = (PathBuf, Vec<String>, usize, CutError);
static LAST_FAILURE: std::sync::Mutex<Option<FailedRemake>> = std::sync::Mutex::new(None);

/// A movie Cut drew for a title or shape in this project's own `titles`
/// folder (see title_movie_path.rs for the rules).
pub(crate) fn is_drawn_overlay(project_dir: &Path, asset_path: &str) -> bool {
    matches!(title_movie(project_dir, asset_path), TitleMovie::Ours(_))
}

fn missing_overlays(project: &cut_core::Project, dir: &Path) -> Vec<MissingOverlay> {
    let mut found = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for track in &project.tracks {
        for clip in &track.clips {
            let cut_core::Clip::Media(clip) = clip else {
                continue;
            };
            let Some(asset) = project.assets.get(&clip.asset) else {
                continue;
            };
            let target = match title_movie(dir, &asset.path) {
                TitleMovie::NotATitle => continue,
                TitleMovie::Ours(path) => Some(path),
                TitleMovie::Elsewhere => None,
            };
            if !seen.insert(clip.asset.clone()) {
                continue;
            }
            // Present means anything at the name, a link included (never followed).
            let present = match &target {
                Some(path) => std::fs::symlink_metadata(path).is_ok(),
                None => {
                    let recorded = Path::new(&asset.path);
                    recorded.is_absolute() && std::fs::symlink_metadata(recorded).is_ok()
                }
            };
            if !present {
                found.push(MissingOverlay {
                    asset: clip.asset.clone(),
                    clip: clip.id.clone(),
                    path: target,
                });
            }
        }
    }
    found
}

/// The words a person reads for this title or shape: its text, else its shape.
pub(super) fn overlay_name(args: &Value) -> String {
    let text = args
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if !text.is_empty() {
        return format!("the title '{}'", text.chars().take(60).collect::<String>());
    }
    match args.get("shape").and_then(Value::as_str) {
        Some(shape) => format!("the {shape} shape"),
        None => "a title".into(),
    }
}

pub(super) fn not_drawn(name: &str, why: &str, action: &str) -> CutError {
    CutError::new(
        error_codes::NOT_FOUND,
        format!("{name} could not be drawn: its rendered file is missing and could not be made again ({why})"),
        "titles and shapes are movies Cut draws into the project's titles folder; this one is gone",
    )
    .with_suggested_action(action)
}

/// Make one missing movie again at its own path. Returns the person's name for it.
/// Draw one title movie again at its own path, recorded as this engine's own
/// for exactly this spec. Returns the person's name for it.
async fn remake(
    state: &AppState,
    project_dir: &Path,
    ops: &[OpRecord],
    settings: (u32, u32, f64),
    missing: &MissingOverlay,
) -> Result<String, CutError> {
    let (name, spec, duration_ms) = resolve_spec(ops, settings, &missing.clip)?;
    if duration_ms == 0 {
        return Err(not_drawn(&name, "its length is zero", "add it again"));
    }
    let Some(out) = missing.path.clone() else {
        return Err(not_drawn(
            &name,
            "its file is recorded outside this project's titles folder",
            "add it again",
        ));
    };
    // Checked again right before writing: the real titles folder is a plain
    // folder inside the real project folder, and the movie goes exactly there.
    let titles = writable_titles_folder(project_dir).map_err(|why| {
        not_drawn(
            &name,
            &why,
            "remove the link or move the project folder, then try again",
        )
    })?;
    if out.parent() != Some(titles.as_path()) {
        return Err(not_drawn(
            &name,
            "its file is recorded outside this project's titles folder",
            "add it again",
        ));
    }
    // A link or a second name planted at the movie's name is removed (it is an
    // entry in the project's own titles folder; what it points at is left
    // alone). encode_title_overlay refuses one at `out` and writes through a
    // staged temporary file (internal_files.rs / atomic_output.rs).
    if std::fs::symlink_metadata(&out).is_ok()
        && !cut_core::matte_cache::plain_file_exists(&out).unwrap_or(false)
    {
        let _ = std::fs::remove_file(&out);
    }
    let out_cl = out.clone();
    let binding = title_binding(&missing.asset, &spec);
    let encoded = tokio::task::spawn_blocking(move || {
        cut_media::render::encode_title_overlay(&spec, &out_cl)
    })
    .await
    .map_err(|e| not_drawn(&name, &e.to_string(), "try again"))?;
    if let Err(error) = encoded {
        return Err(not_drawn(
            &name,
            &error.message,
            "check that FFmpeg is installed, then try again",
        ));
    }
    // The movie is content-addressed by its first hash; a renderer update can
    // draw different bytes. The clip plays the file at the same path either way.
    cut_media::cache_trust::remember_bound(&out, &binding);
    // Its probe is what the engine just drew (the cached one is not a fact).
    let (w, h, _) = settings;
    if let Some(store) = state.project.write().await.as_mut() {
        if let Some(asset) = store.project.assets.get_mut(&missing.asset) {
            asset.probe = Some(json!({"kind": "video", "width": w, "height": h,
                "has_audio": false, "has_video": true, "duration_ms": duration_ms}));
            store.project.mark_probe_verified(&missing.asset);
        }
    }
    Ok(name)
}

/// Make every missing title/shape movie the timeline shows. `Ok(names)` lists
/// what was made again (empty when nothing was missing); the first one that
/// cannot be made is the error, in plain words.
async fn remake_missing(state: &AppState, and_unproven: bool) -> Result<Vec<String>, CutError> {
    let (missing, ops, settings, dir) = {
        let guard = state.project.read().await;
        let Some(store) = guard.as_ref() else {
            return Ok(Vec::new());
        };
        let mut missing = missing_overlays(&store.project, &store.dir);
        let s = &store.project.settings;
        if and_unproven {
            let ops = store.log.read_all()?;
            missing.extend(present_unproven(
                &store.project,
                &store.dir,
                &ops,
                (s.width, s.height, s.fps),
            ));
        }
        if missing.is_empty() {
            return Ok(Vec::new());
        }
        let s = &store.project.settings;
        (
            missing,
            store.log.read_all()?,
            (s.width, s.height, s.fps),
            store.dir.clone(),
        )
    };
    let mut made = Vec::new();
    for item in &missing {
        made.push(remake(state, &dir, &ops, settings, item).await?);
    }
    Ok(made)
}

/// Before reading the project's media (exact frames, renders, exports): make
/// any missing title or shape movie again. A remake is announced as a finished
/// `title_remake` job so clients re-read the project (the preview reloads the
/// overlay it could not play).
pub(crate) async fn ensure_overlay_movies(state: &AppState) -> Result<(), CutError> {
    let (dir, nothing_missing) = {
        let guard = state.project.read().await;
        let Some(store) = guard.as_ref() else {
            return Ok(());
        };
        let missing = missing_overlays(&store.project, &store.dir);
        (store.dir.clone(), missing.is_empty())
    };
    // Nothing to draw and no open check of this project's present movies
    // running: no wait (the lock is shared by every open project).
    if nothing_missing && !open_check_pending(&dir) {
        return Ok(());
    }
    // One at a time, and after the open's check of present title movies.
    let _one_at_a_time = REMAKE.lock().await;
    draw_titles(state, false).await
}

async fn draw_titles(state: &AppState, and_unproven: bool) -> Result<(), CutError> {
    let key = {
        let guard = state.project.read().await;
        let Some(store) = guard.as_ref() else {
            return Ok(());
        };
        let missing: Vec<String> = missing_overlays(&store.project, &store.dir)
            .into_iter()
            .map(|m| m.asset)
            .collect();
        if missing.is_empty() && !and_unproven {
            return Ok(());
        }
        (
            store.dir.clone(),
            missing,
            store.log.read_all().map(|ops| ops.len()).unwrap_or(0),
        )
    };
    if let Some((dir, missing, ops, error)) = LAST_FAILURE.lock().ok().and_then(|last| last.clone())
    {
        if (dir, missing, ops) == key {
            return Err(error);
        }
    }
    if and_unproven && key.1.is_empty() {
        // Only present movies to prove: no job unless one is drawn again.
        let made = remake_missing(state, true).await?;
        if !made.is_empty() {
            let job = state.jobs.create("title_remake");
            state.jobs.finish(&job.job_id, json!({"made_again": made}));
        }
        return Ok(());
    }
    let job = state.jobs.create("title_remake");
    state.jobs.progress(
        &job.job_id,
        0.05,
        Some("making missing titles again".into()),
    );
    match remake_missing(state, and_unproven).await {
        Ok(made) => {
            state.jobs.finish(&job.job_id, json!({"made_again": made}));
            Ok(())
        }
        Err(error) => {
            state.jobs.fail(&job.job_id, error.clone());
            if let Ok(mut last) = LAST_FAILURE.lock() {
                *last = Some((key.0, key.1, key.2, error.clone()));
            }
            Err(error)
        }
    }
}

/// At a project open: make missing title/shape movies again, and draw again
/// every present one this engine did not draw for its current settings (a
/// received project's movie can show anything). Media reads wait for this.
/// `dir` was marked by [`begin_open_title_check`] before it became visible.
pub(crate) fn remake_overlays_at_open(state: &AppState, dir: PathBuf) {
    let state = state.clone();
    tokio::spawn(async move {
        let _one_at_a_time = REMAKE.lock().await;
        let open = state
            .project
            .read()
            .await
            .as_ref()
            .map(|store| store.dir.clone());
        if open.as_ref() == Some(&dir) {
            if let Err(error) = draw_titles(&state, true).await {
                tracing::warn!(error = %error.message, "a title could not be made again at open");
            }
        }
        set_open_check(&dir, false);
    });
}

#[cfg(test)]
#[path = "title_remake_tests.rs"]
mod tests;

//! Which present title movies are this engine's own drawing of their current
//! settings. A received project can carry a movie at a title's name showing
//! anything; it counts only with an engine-origin record bound to the asset and
//! the exact spec (cache_trust), else it is drawn again at the next open.

use super::title_movie_path::{title_movie, TitleMovie};
use super::title_remake::{not_drawn, overlay_name, MissingOverlay};
use super::title_shape::{recover_shape_args, recover_title_args};
use super::*;
use std::path::Path;

/// What a title movie is bound to: its asset and the exact spec it was drawn
/// from (a movie drawn for other settings, or by someone else, does not count).
pub(crate) fn title_binding(asset_id: &str, spec: &cut_media::title::TitleSpec) -> String {
    format!(
        "title\n{asset_id}\n{}",
        serde_json::to_string(spec).unwrap_or_default()
    )
}

/// The person's name for the clip's title or shape, the spec it is drawn
/// from (its settings in the edit history) and its length.
pub(super) fn resolve_spec(
    ops: &[OpRecord],
    settings: (u32, u32, f64),
    clip: &str,
) -> Result<(String, cut_media::title::TitleSpec, u64), CutError> {
    let (w, h, fps) = settings;
    let resolved = match recover_title_args(ops, clip) {
        Ok(args) => {
            let name = overlay_name(&args);
            let parsed: TitleArgs = serde_json::from_value(args).map_err(|e| {
                not_drawn(
                    &name,
                    &format!("its settings do not read back: {e}"),
                    "add the title again",
                )
            })?;
            let duration = parsed.range_ms[1].saturating_sub(parsed.range_ms[0]);
            (
                name.clone(),
                build_title_spec(&parsed, w, h, fps)
                    .map_err(|e| not_drawn(&name, &e.message, "add the title again"))?,
                duration,
            )
        }
        Err(title_error) => match recover_shape_args(ops, clip) {
            Ok(args) => {
                let name = overlay_name(&args);
                let parsed: ShapeArgs = serde_json::from_value(args).map_err(|e| {
                    not_drawn(
                        &name,
                        &format!("its settings do not read back: {e}"),
                        "add the shape again",
                    )
                })?;
                let duration = parsed.range_ms[1].saturating_sub(parsed.range_ms[0]);
                (
                    name.clone(),
                    build_shape_spec(&parsed, w, h, fps)
                        .map_err(|e| not_drawn(&name, &e.message, "add the shape again"))?,
                    duration,
                )
            }
            Err(_) if title_error.message.contains("captions.kinetic") => {
                return Err(not_drawn(
                    "a caption title",
                    "caption titles are drawn from the transcript",
                    "run captions.kinetic again for this range",
                ))
            }
            Err(_) => {
                return Err(not_drawn(
                    "a title",
                    "its settings are not in this project's edit history",
                    "add the title again",
                ))
            }
        },
    };
    Ok(resolved)
}

/// Present title movies of this project that this engine did not draw for
/// their current settings (no record bound to the spec): drawn again.
pub(super) fn present_unproven(
    project: &cut_core::Project,
    dir: &Path,
    ops: &[OpRecord],
    settings: (u32, u32, f64),
) -> Vec<MissingOverlay> {
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
            let TitleMovie::Ours(path) = title_movie(dir, &asset.path) else {
                continue;
            };
            if std::fs::symlink_metadata(&path).is_err() || !seen.insert(clip.asset.clone()) {
                continue; // missing ones are listed by missing_overlays
            }
            // A movie without settings in the history is the sender's media, not
            // a title Cut draws: left as it is.
            let Ok((_, spec, _)) = resolve_spec(ops, settings, &clip.id) else {
                continue;
            };
            let plain = cut_core::matte_cache::plain_file_exists(&path).unwrap_or(false);
            if !(plain
                && cut_media::cache_trust::trusted_bound(&path, &title_binding(&clip.asset, &spec)))
            {
                found.push(MissingOverlay {
                    asset: clip.asset.clone(),
                    clip: clip.id.clone(),
                    path: Some(path),
                });
            }
        }
    }
    found
}

/// Project folders whose open check of present title movies has not ended.
static OPEN_CHECKS: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());

pub(super) fn open_check_pending(dir: &std::path::Path) -> bool {
    OPEN_CHECKS
        .lock()
        .map(|dirs| dirs.iter().any(|pending| pending == dir))
        .unwrap_or(true)
}

pub(super) fn set_open_check(dir: &std::path::Path, pending: bool) {
    if let Ok(mut dirs) = OPEN_CHECKS.lock() {
        if pending {
            dirs.push(dir.to_path_buf());
        } else if let Some(at) = dirs.iter().position(|d| d == dir) {
            dirs.remove(at);
        }
    }
}

/// Mark `dir`'s open check of present title movies as running. Called under
/// the project write lock that makes `dir` the open project, so no media read
/// sees the project before the mark; `remake_overlays_at_open` clears it.
pub(crate) fn begin_open_title_check(dir: &std::path::Path) {
    set_open_check(dir, true);
}

/// Tests: hold the open-check marks so a test can see what an open does
/// before its mark can be set.
#[cfg(test)]
pub(crate) fn hold_open_checks() -> std::sync::MutexGuard<'static, Vec<std::path::PathBuf>> {
    OPEN_CHECKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

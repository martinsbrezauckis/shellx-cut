//! output_paths.rs — shared output-folder preference and output path fencing.
//!
//! Role: own the session default export folder (`project.set_output_dir`) and the
//! path-fence helpers used by file-writing verbs. Dispatch remains the verb
//! router and export/render handlers call this module for output resolution.

use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use cut_core::{error_codes, CutError, VerbResult};
use cut_media::PathFence;
use serde_json::{json, Value};

/// Session-global output directory chosen by the user (project.set_output_dir /
/// the UI "Choose folder..."). When set, file-writing verbs that don't get an
/// explicit `path` write their default-named file HERE instead of
/// <project>/exports, and it becomes an allowed PathFence root. Process-global
/// because cutd serves a single open project per process (same scope as the
/// CUTD_OUTPUTS_DIR env root). A workspace preference — NOT part of the project
/// or the op-log, never replayed.
static SESSION_OUTPUT_DIR: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);
static OUTPUT_WRITE_SEQ: AtomicU64 = AtomicU64::new(0);

/// The current session output dir, if set AND still an existing directory
/// (a folder deleted out from under us is ignored, not a hard export failure).
fn session_output_dir() -> Option<PathBuf> {
    let dir = SESSION_OUTPUT_DIR.read().ok()?.clone()?;
    dir.is_dir().then_some(dir)
}

/// Set (Some) or clear (None) the session output dir.
pub(crate) fn set_session_output_dir(dir: Option<PathBuf>) {
    if let Ok(mut g) = SESSION_OUTPUT_DIR.write() {
        *g = dir;
    }
}

/// Build the project's PathFence (cut-media owns the output-fencing policy). Writes
/// may target the project dir, the env-configured CUTD_OUTPUTS_DIR, and the
/// user's chosen session output dir — those are the only allowed roots.
pub(crate) fn make_fence(project_dir: &Path) -> Result<PathFence, CutError> {
    let mut fence = PathFence::new(project_dir)?;
    if let Ok(d) = std::env::var("CUTD_OUTPUTS_DIR") {
        if !d.is_empty() {
            fence = fence.with_extra_root(Path::new(&d))?;
        }
    }
    if let Some(dir) = session_output_dir() {
        fence = fence.with_extra_root(&dir)?;
    }
    Ok(fence)
}

/// Resolve + validate an output path for a file-writing verb. Precedence:
/// explicit `requested` path -> the session output dir (default file name dropped
/// in the chosen folder) -> `default_rel` inside the project. Fenced by
/// cut_media::PathFence. Creates the parent dir so first-time exports work.
pub(crate) fn fence_output_path(
    project_dir: &Path,
    requested: Option<&str>,
    default_rel: &str,
) -> Result<PathBuf, CutError> {
    let fence = make_fence(project_dir)?;
    let mut candidate = match requested {
        Some(p) => {
            let pb = PathBuf::from(p);
            // A relative requested path resolves INSIDE the project (the fence
            // rejects escapes). Create its parent — e.g. exports/ — so an explicit
            // "exports/_monitor_a.mp3" works on a FRESH project, matching the
            // default-path branch below (which already creates exports/ on first
            // use).
            if pb.is_relative() {
                if let Some(parent) = project_dir.join(&pb).parent() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            pb
        }
        None => match session_output_dir() {
            Some(dir) => {
                let name = Path::new(default_rel)
                    .file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("export.out"));
                dir.join(name)
            }
            None => {
                let p = project_dir.join(default_rel);
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                p
            }
        },
    };
    if requested.is_none() {
        candidate = next_available_output_path(&candidate);
    }
    fence.fence_output_path(&candidate)
}

/// Resolve an output whose DEFAULT must stay in the project's served exports
/// tree even when the user configured a session-wide delivery folder. Review
/// packages use this because the UI opens them through `/api/export/*`; an
/// explicit caller path still follows the normal output-root policy.
pub(crate) fn fence_project_output_path(
    project_dir: &Path,
    requested: Option<&str>,
    default_rel: &str,
) -> Result<PathBuf, CutError> {
    if requested.is_some() {
        return fence_output_path(project_dir, requested, default_rel);
    }
    let path = project_dir.join(default_rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    make_fence(project_dir)?.fence_output_path(&next_available_output_path(&path))
}

fn atomic_write_tmp_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("export.out");
    let n = OUTPUT_WRITE_SEQ.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), n))
}

/// Build a sibling temp path for renderers such as ffmpeg that infer the output
/// container from the file extension. Unlike `atomic_write_tmp_path`, this keeps
/// the final extension last: `.range_0_4000.<pid>.<n>.tmp.mp4`.
pub(crate) fn temp_output_path_for_render(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("export");
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty());
    let n = OUTPUT_WRITE_SEQ.fetch_add(1, Ordering::Relaxed);
    let name = match ext {
        Some(ext) => format!(".{stem}.{}.{}.tmp.{ext}", std::process::id(), n),
        None => format!(".{stem}.{}.{}.tmp", std::process::id(), n),
    };
    parent.join(name)
}

/// Publish a completed sibling temp output to its final fenced path. The caller
/// owns creating/flushing the temp file; this helper only does the final replace.
pub(crate) fn publish_output_atomic(tmp: &Path, path: &Path) -> Result<(), CutError> {
    match std::fs::rename(tmp, path) {
        Ok(()) => Ok(()),
        Err(first_err) => {
            // Windows does not replace an existing destination with rename().
            // Re-check the path entry, remove only a file/symlink, then retry.
            // Unix normally never reaches this branch for existing files.
            let removable = std::fs::symlink_metadata(path)
                .map(|m| m.file_type().is_file() || m.file_type().is_symlink())
                .unwrap_or(false);
            if removable {
                if let Err(remove_err) = std::fs::remove_file(path) {
                    let _ = std::fs::remove_file(tmp);
                    return Err(CutError::new(
                        error_codes::IO,
                        format!("could not replace output {}", path.display()),
                        format!("rename failed: {first_err}; remove failed: {remove_err}"),
                    ));
                }
                if let Err(rename_err) = std::fs::rename(tmp, path) {
                    let _ = std::fs::remove_file(tmp);
                    return Err(CutError::new(
                        error_codes::IO,
                        format!("could not publish output {}", path.display()),
                        rename_err.to_string(),
                    ));
                }
                Ok(())
            } else {
                let _ = std::fs::remove_file(tmp);
                Err(CutError::new(
                    error_codes::IO,
                    format!("could not publish output {}", path.display()),
                    first_err.to_string(),
                ))
            }
        }
    }
}

/// Write a fenced export/sidecar file through a sibling temp file, then publish
/// it with rename. This closes the late-symlink race left by a separate
/// `fence_output_path()` validation followed by direct `std::fs::write()`: the
/// final publish replaces a path entry instead of opening the final path for
/// writing. Callers must pass a path already resolved by `fence_output_path`.
pub(crate) fn write_output_atomic(path: &Path, bytes: impl AsRef<[u8]>) -> Result<(), CutError> {
    let tmp = atomic_write_tmp_path(path);
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&tmp)
        .map_err(|e| {
            CutError::new(
                error_codes::IO,
                format!("could not create temp output {}", tmp.display()),
                e.to_string(),
            )
        })?;
    file.write_all(bytes.as_ref()).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("could not write temp output {}", tmp.display()),
            e.to_string(),
        )
    })?;
    file.sync_all().map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("could not flush temp output {}", tmp.display()),
            e.to_string(),
        )
    })?;
    drop(file);

    publish_output_atomic(&tmp, path)
}

fn next_available_output_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("export");
    let ext = path.extension().and_then(|s| s.to_str());
    for n in 2..10_000 {
        let name = match ext {
            Some(ext) if !ext.is_empty() => format!("{stem}-{n}.{ext}"),
            _ => format!("{stem}-{n}"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

pub(crate) fn fenced_existing_file_under_dir(
    base_dir: &Path,
    path: &Path,
    label: &str,
    suggested_action: &str,
) -> Result<PathBuf, CutError> {
    let base = base_dir.canonicalize().map_err(|e| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("cannot access {label} base {}: {e}", base_dir.display()),
            "the containing directory must exist before resolving the file",
        )
        .with_suggested_action(suggested_action)
    })?;
    let target = path.canonicalize().map_err(|e| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("{label} not found: {} ({e})", path.display()),
            "the requested file must exist",
        )
        .with_suggested_action(suggested_action)
    })?;
    if !target.is_file() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("{label} is not a file: {}", target.display()),
            "the requested path must point at a file",
        )
        .with_suggested_action(suggested_action));
    }
    if !target.starts_with(&base) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("{label} must stay inside {}", base.display()),
            format!("resolved path was {}", target.display()),
        )
        .with_suggested_action(suggested_action));
    }
    Ok(target)
}

pub(crate) fn resolve_existing_project_file(
    project_dir: &Path,
    requested: &str,
    label: &str,
    suggested_action: &str,
) -> Result<PathBuf, CutError> {
    if requested.trim().is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("{label} path is empty"),
            "a project-local file path is required",
        )
        .with_suggested_action(suggested_action));
    }
    let raw = Path::new(requested);
    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        project_dir.join(raw)
    };
    fenced_existing_file_under_dir(project_dir, &candidate, label, suggested_action)
}

/// project.set_output_dir{dir?} — choose where exports/renders land when a verb
/// isn't given an explicit `path`. Empty/absent `dir` clears it (back to
/// <project>/exports). The folder must already exist (the UI's native folder
/// picker only returns existing dirs); it is canonicalized and becomes an
/// allowed fence root. A workspace preference — never an op, never replayed.
pub(crate) async fn project_set_output_dir(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        dir: Option<String>,
    }
    let a: Args = crate::dispatch::parse_args(args)?;
    match a.dir.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => {
            set_session_output_dir(None);
            Ok(VerbResult::ok(
                json!({ "dir": Value::Null, "cleared": true }),
            ))
        }
        Some(d) => {
            let path = std::fs::canonicalize(d).map_err(|e| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("output folder not found: {d}"),
                    e.to_string(),
                )
                .with_suggested_action("pick an existing folder (the desktop picker returns one)")
            })?;
            if !path.is_dir() {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "output path is not a directory",
                    path.display().to_string(),
                ));
            }
            set_session_output_dir(Some(path.clone()));
            Ok(VerbResult::ok(json!({ "dir": path.display().to_string() })))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the output-fencing contract: output paths are fenced — traversal, foreign dirs and
    /// non-media suffixes are refused.
    #[test]
    fn output_path_fencing() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        std::fs::create_dir_all(&proj).unwrap();
        assert!(fence_output_path(&proj, None, "exports/out.mp4").is_ok());
        assert!(fence_output_path(&proj, Some("../evil.mp4"), "x.mp4").is_err());
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("evil.mp4");
        assert!(fence_output_path(&proj, outside_file.to_str(), "x.mp4").is_err());
        std::fs::create_dir_all(proj.join("exports")).unwrap();
        let inside = proj.join("exports/project.json");
        std::fs::write(&inside, b"existing project data file").unwrap();
        assert!(fence_output_path(&proj, Some(inside.to_str().unwrap()), "x.mp4").is_err());
    }

    #[test]
    fn default_output_paths_avoid_existing_files() {
        set_session_output_dir(None);
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        std::fs::create_dir_all(proj.join("exports")).unwrap();
        let first = fence_output_path(&proj, None, "exports/recording.mp4").unwrap();
        assert!(first.ends_with("recording.mp4"));
        std::fs::write(&first, b"existing recording").unwrap();
        let second = fence_output_path(&proj, None, "exports/recording.mp4").unwrap();
        assert!(second.ends_with("recording-2.mp4"));
        assert!(!second.exists());
        let explicit = fence_output_path(
            &proj,
            Some(first.to_str().unwrap()),
            "exports/recording.mp4",
        )
        .unwrap();
        assert_eq!(explicit, first, "explicit Save As paths stay exact");

        let outside = tempfile::tempdir().unwrap();
        set_session_output_dir(Some(outside.path().to_path_buf()));
        let selected = outside.path().join("selected.mp4");
        let resolved = fence_output_path(
            &proj,
            Some(selected.to_str().unwrap()),
            "exports/selected.mp4",
        )
        .unwrap();
        let request_fence = make_fence(&proj).unwrap();
        set_session_output_dir(None);
        assert!(
            request_fence.fence_output_path(&resolved).is_ok(),
            "an async render keeps the request-time Save As authorization"
        );
        assert!(
            make_fence(&proj)
                .unwrap()
                .fence_output_path(&resolved)
                .is_err(),
            "restoring the default still revokes that root for later requests"
        );
        set_session_output_dir(None);
    }

    #[test]
    fn render_temp_output_preserves_extension_for_ffmpeg() {
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("exports").join("range_0_4000.mp4");
        let tmp = temp_output_path_for_render(&final_path);
        assert_eq!(tmp.parent(), final_path.parent());
        let name = tmp.file_name().and_then(|s| s.to_str()).unwrap();
        assert!(name.starts_with(".range_0_4000."));
        assert!(name.ends_with(".tmp.mp4"));
    }
}

//! output_paths.rs — shared output-folder preference and output path fencing.
//!
//! Role: own the session default export folder (`project.set_output_dir`) and the
//! path-fence helpers used by file-writing verbs. Dispatch remains the verb
//! router and export/render handlers call this module for output resolution.

use std::{
    fs::File,
    io::Write,
    ops::Deref,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use cut_core::{error_codes, CutError, VerbResult};
use cut_media::PathFence;
use fs2::FileExt;
use serde_json::{json, Value};

mod policy;
pub(crate) use policy::OutputPathPolicy;

/// A fenced output path. Resolved outputs retain an exclusive sidecar
/// reservation until the final clone is dropped, so cooperating writers cannot
/// select the same destination concurrently.
#[derive(Debug, Clone)]
pub(crate) struct OutputPath {
    path: PathBuf,
    _reservation: Option<std::sync::Arc<OutputReservation>>,
}

impl OutputPath {
    fn reserved(path: PathBuf, reservation: OutputReservation) -> Self {
        Self {
            path,
            _reservation: Some(std::sync::Arc::new(reservation)),
        }
    }
}

impl Deref for OutputPath {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl AsRef<Path> for OutputPath {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl PartialEq for OutputPath {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl Eq for OutputPath {}

impl serde::Serialize for OutputPath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.path.serialize(serializer)
    }
}

#[derive(Debug)]
struct OutputReservation {
    marker: PathBuf,
    // This durable, per-output lock file is deliberately retained after normal
    // cleanup. Its OS-managed advisory lock is released on process death, so a
    // future writer can safely reclaim a crash-left marker without guessing
    // whether another process is still encoding the same destination.
    _lease: File,
}

impl Drop for OutputReservation {
    fn drop(&mut self) {
        // Remove the visible marker while the advisory lease is still held; the
        // File drops immediately after this method and unlocks at process exit.
        // Keeping the lease file itself avoids an unlink/recreate race between
        // two processes that would otherwise split ownership across inodes.
        let _ = std::fs::remove_file(&self.marker);
    }
}

/// Session-global output directory chosen by the user (project.set_output_dir /
/// the UI "Choose folder..."). When set, file-writing verbs that don't get an
/// explicit `path` write their default-named file HERE instead of
/// <project>/exports, and it becomes an allowed PathFence root. Process-global
/// because cutd serves a single open project per process (same scope as the
/// CUTD_OUTPUTS_DIR env root). A workspace preference — NOT part of the project
/// or the op-log, never replayed.
static SESSION_OUTPUT_DIR: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);
static OUTPUT_WRITE_SEQ: AtomicU64 = AtomicU64::new(0);

/// The session output dir is process-global (it is a user preference for the
/// whole cutd session), and `cargo test` runs the tests in one binary on
/// several threads — so every test that reads or writes it has to take this
/// lock, or an unrelated test's `set_session_output_dir` decides the outcome.
/// Poisoning is recovered from deliberately: a panicking test tells us about
/// its own assertion, not about lock hygiene.
#[cfg(test)]
pub(crate) static SESSION_OUTPUT_DIR_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

/// The directories the HTTP export routes may READ from, canonical, most
/// specific first: `<project>/exports`, then `CUTD_OUTPUTS_DIR`, then the user's
/// session output dir. Roots that do not currently resolve are dropped.
///
/// The read fence extends past `<project>/exports` because the same preference set
/// that authorizes an export WRITE (`make_fence`) decides where a finished
/// export physically lands. Before this existed the two disagreed — the engine
/// happily wrote a render into the user's chosen delivery folder and then the
/// serve route, fenced to `<project>/exports` alone, could not hand that same
/// file back. Serving a file the app itself just wrote into a folder
/// the user explicitly designated is NOT new authority; it is the read half of
/// an authorization the user already gave. What is deliberately NOT included is
/// the project dir itself (unlike `make_fence`): reads stay fenced to the
/// exports SUBTREE, so `project.json`, the op log, media and proxies are never
/// reachable through an export URL. A momentary Save As authorization
/// (`withAuthorizedOutputPath`) is likewise NOT retained — it authorized one
/// write, not a standing read root.
pub(crate) fn authorized_export_read_roots(project_dir: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut push = |dir: PathBuf| {
        // Canonicalize so membership tests compare resolved paths (a symlinked
        // root and its target are the same fence), and so a root that no longer
        // exists simply drops out instead of matching by prefix string.
        if let Ok(canon) = dir.canonicalize() {
            if canon.is_dir() && !roots.contains(&canon) {
                roots.push(canon);
            }
        }
    };
    push(project_dir.join("exports"));
    if let Ok(d) = std::env::var("CUTD_OUTPUTS_DIR") {
        if !d.is_empty() {
            push(PathBuf::from(d));
        }
    }
    if let Some(dir) = session_output_dir() {
        push(dir);
    }
    roots
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
    policy: OutputPathPolicy,
) -> Result<OutputPath, CutError> {
    let fence = make_fence(project_dir)?;
    let (mut candidate, create_parent) = match requested {
        Some(p) => {
            let pb = PathBuf::from(p);
            reject_relative_output_traversal(&pb)?;
            let parent = pb
                .is_relative()
                .then(|| project_dir.join(&pb))
                .and_then(|path| path.parent().map(Path::to_path_buf));
            (pb, parent)
        }
        None => match session_output_dir() {
            Some(dir) => {
                let name = Path::new(default_rel)
                    .file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("export.out"));
                (dir.join(name), None)
            }
            None => {
                let p = project_dir.join(default_rel);
                let parent = p.parent().map(Path::to_path_buf);
                (p, parent)
            }
        },
    };
    // Validate the declared output type before creating a caller-supplied
    // directory. The path fence below still owns canonicalization, traversal,
    // symlink and root checks.
    policy.validate(&candidate)?;
    if let Some(parent) = create_parent {
        // A relative requested path resolves INSIDE the project (the fence
        // rejects escapes). Create its parent — e.g. exports/ — so an explicit
        // "exports/_monitor_a.mp3" works on a fresh project, matching default
        // project exports.
        std::fs::create_dir_all(parent)?;
    }
    if requested.is_some() {
        let path = fence.fence_output_path(&candidate)?;
        return reserve_explicit_output_path(path);
    }

    // Default names are part of the public delivery contract. Reserve one with
    // create_new rather than testing `exists()` and hoping it stays empty until
    // the encoder starts; the OutputPath wrapper holds the reservation through
    // async render jobs and releases it after their last reference drops.
    let (reserved, reservation) = reserve_next_available_output_path(&fence, &candidate)?;
    candidate = reserved;
    Ok(OutputPath::reserved(candidate, reservation))
}

/// Refuse a relative caller path that can escape the project before calculating
/// and creating its parent. `PathFence` repeats this canonical contract check;
/// this early mirror only prevents `create_dir_all(project/../outside)` from
/// mutating the filesystem before the canonical fence returns its error.
fn reject_relative_output_traversal(path: &Path) -> Result<(), CutError> {
    if path.is_relative()
        && path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("output path {} contains '..'", path.display()),
            "path traversal is refused by the public output-fencing contract",
        )
        .with_suggested_action("pass an absolute path inside the project or outputs dir"));
    }
    Ok(())
}

/// Resolve an output whose DEFAULT must stay in the project's served exports
/// tree even when the user configured a session-wide delivery folder. Review
/// packages use this because the UI opens them through `/api/export/*`; an
/// explicit caller path still follows the normal output-root policy.
pub(crate) fn fence_project_output_path(
    project_dir: &Path,
    requested: Option<&str>,
    default_rel: &str,
    policy: OutputPathPolicy,
) -> Result<OutputPath, CutError> {
    if requested.is_some() {
        return fence_output_path(project_dir, requested, default_rel, policy);
    }
    let path = project_dir.join(default_rel);
    policy.validate(&path)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let fence = make_fence(project_dir)?;
    let (path, reservation) = reserve_next_available_output_path(&fence, &path)?;
    Ok(OutputPath::reserved(path, reservation))
}

/// Resolve a private project-relative directory destination. Unlike export
/// files, these paths are internal package directories and have no suffix or
/// user-chosen default-name contract. They retain the prior collision suffix
/// behavior and receive the canonical PathFence before a caller creates the
/// directory.
pub(crate) fn fence_project_directory_path(
    project_dir: &Path,
    relative: &str,
) -> Result<PathBuf, CutError> {
    let path = project_dir.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let fence = make_fence(project_dir)?;
    let (path, reservation) = reserve_next_available_output_path(&fence, &path)?;
    drop(reservation);
    Ok(path)
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

fn reserve_next_available_output_path(
    fence: &PathFence,
    path: &Path,
) -> Result<(PathBuf, OutputReservation), CutError> {
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("export");
    let ext = path.extension().and_then(|s| s.to_str());
    for n in 1..10_000 {
        let candidate = if n == 1 {
            path.to_path_buf()
        } else {
            let name = match ext {
                Some(ext) if !ext.is_empty() => format!("{stem}-{n}.{ext}"),
                _ => format!("{stem}-{n}"),
            };
            parent.join(name)
        };
        // Preserve every PathFence check for each collision suffix. This must
        // happen before the sidecar is created, so a rejected path is never
        // reserved and the canonical fence remains the authority on location.
        let candidate = fence.fence_output_path(&candidate)?;
        if candidate.exists() {
            continue;
        }
        match try_reserve_output_path(&candidate)? {
            Some(reservation) => {
                // A non-cooperating process may create the final path after
                // our first check. Discard this marker and try -2 rather than
                // claiming an already-created default output.
                if candidate.exists() {
                    drop(reservation);
                    continue;
                }
                return Ok((candidate, reservation));
            }
            None => continue,
        }
    }
    Err(CutError::new(
        error_codes::CONFLICT,
        format!(
            "could not allocate a free output name near {}",
            path.display()
        ),
        "10,000 default names are already files or reserved by active writers",
    )
    .with_suggested_action("choose an explicit output filename or clear old exports"))
}

fn reserve_explicit_output_path(path: PathBuf) -> Result<OutputPath, CutError> {
    match try_reserve_output_path(&path)? {
        Some(reservation) => Ok(OutputPath::reserved(path, reservation)),
        None => Err(CutError::new(
            error_codes::CONFLICT,
            format!(
                "output path {} is reserved by an active export",
                path.display()
            ),
            "an active writer owns this output path until it finishes",
        )
        .with_suggested_action(
            "choose a different output filename or wait for the active export to finish",
        )),
    }
}

/// Claim a path's cross-process lease and refresh its visible reservation
/// marker. `None` means another live writer holds the lease. An existing marker
/// with an unlocked lease is from a crashed writer, so it is safe to replace.
fn try_reserve_output_path(path: &Path) -> Result<Option<OutputReservation>, CutError> {
    let lease_path = reservation_lease_path(path);
    let lease = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lease_path)
        .map_err(|error| {
            CutError::new(
                error_codes::IO,
                format!("could not open output lease {}", lease_path.display()),
                error.to_string(),
            )
        })?;
    hide_reservation_artifact(&lease_path);
    match lease.try_lock_exclusive() {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
        Err(error) => {
            return Err(CutError::new(
                error_codes::IO,
                format!("could not lock output lease {}", lease_path.display()),
                error.to_string(),
            ));
        }
    }

    let marker = reservation_marker_path(path);
    match std::fs::remove_file(&marker) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(CutError::new(
                error_codes::IO,
                format!("could not reclaim output reservation {}", marker.display()),
                error.to_string(),
            ));
        }
    }
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&marker)
        .map_err(|error| {
            CutError::new(
                error_codes::IO,
                format!("could not reserve output name {}", path.display()),
                error.to_string(),
            )
        })?;
    hide_reservation_artifact(&marker);
    Ok(Some(OutputReservation {
        marker,
        _lease: lease,
    }))
}

fn reservation_marker_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("export.out");
    parent.join(format!(".{name}.shellx-cut-reserving"))
}

fn reservation_lease_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("export.out");
    parent.join(format!(".{name}.shellx-cut-reservation.lock"))
}

/// Keep reservation artifacts out of Windows Explorer's normal file view. This
/// is cosmetic only: a failure to set the attribute must never block an export
/// or weaken the held advisory lease.
#[cfg(windows)]
fn hide_reservation_artifact(path: &Path) {
    let Some(attributes) = windows_file_attributes(path) else {
        return;
    };
    let path = windows_wide_path(path);
    // Best effort: the file has already been created and remains usable even
    // when an unusual filesystem refuses the cosmetic attribute change.
    let _ =
        unsafe { SetFileAttributesW(path.as_ptr(), attributes | WINDOWS_FILE_ATTRIBUTE_HIDDEN) };
}

#[cfg(not(windows))]
fn hide_reservation_artifact(_path: &Path) {}

#[cfg(windows)]
const WINDOWS_FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
#[cfg(windows)]
const WINDOWS_INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;

#[cfg(windows)]
fn windows_file_attributes(path: &Path) -> Option<u32> {
    let path = windows_wide_path(path);
    let attributes = unsafe { GetFileAttributesW(path.as_ptr()) };
    (attributes != WINDOWS_INVALID_FILE_ATTRIBUTES).then_some(attributes)
}

#[cfg(windows)]
fn windows_wide_path(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
#[link(name = "Kernel32")]
extern "system" {
    fn GetFileAttributesW(path: *const u16) -> u32;
    fn SetFileAttributesW(path: *const u16, attributes: u32) -> i32;
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

/// Resolve an EXISTING export the engine itself produced, fenced to the same
/// authorized read roots the HTTP export routes use
/// (`authorized_export_read_roots`: the `<project>/exports` subtree,
/// `CUTD_OUTPUTS_DIR`, the session output dir).
///
/// WHY this exists next to `fenced_existing_file_under_dir`: a verb that consumes
/// a previous render (today `comment.export`, whose source is the render named by
/// a RenderReceipt) must be able to find that render wherever the engine was
/// authorized to deliver it. Fencing such a lookup to `<project>/exports` alone
/// made the verb impossible for anyone who had chosen a default export folder —
/// `render.final` delivers there, then the receipt's `output_path` sits outside
/// the fence and the verb refuses a file it wrote itself. Reproduction: with a
/// default export folder set, `receipts/<render>.json` points at that folder and
/// `comment.export` returns ok:false while the render job reports done.
///
/// This grants no new authority: it is the read half of a write the user already
/// authorized, and it deliberately inherits the roots' exclusion of the project
/// dir itself, so `project.json`, the op log, media and proxies stay unreachable.
/// Canonicalization happens FIRST, so `..` and symlinks resolve before the
/// membership test, and `starts_with` compares whole path components.
pub(crate) fn fenced_existing_export_read(
    project_dir: &Path,
    path: &Path,
    label: &str,
    suggested_action: &str,
) -> Result<PathBuf, CutError> {
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
    let roots = authorized_export_read_roots(project_dir);
    if roots.iter().any(|root| target.starts_with(root)) {
        return Ok(target);
    }
    Err(CutError::new(
        error_codes::INVALID_ARGS,
        format!("{label} is outside every authorized export folder"),
        format!(
            "resolved path was {}; authorized: {}",
            target.display(),
            if roots.is_empty() {
                "none".to_string()
            } else {
                roots
                    .iter()
                    .map(|r| r.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        ),
    )
    .with_suggested_action(suggested_action))
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

    #[cfg(windows)]
    #[test]
    fn live_reservation_artifacts_are_hidden_on_windows() {
        let _guard = SESSION_OUTPUT_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_session_output_dir(None);
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        std::fs::create_dir_all(&proj).unwrap();

        let reserved =
            fence_output_path(&proj, None, "exports/recording.mp4", OutputPathPolicy::MP4)
                .expect("live output reservation");
        for artifact in [
            reservation_marker_path(&reserved),
            reservation_lease_path(&reserved),
        ] {
            let attributes =
                windows_file_attributes(&artifact).expect("reservation artifact remains readable");
            assert_ne!(
                attributes & WINDOWS_FILE_ATTRIBUTE_HIDDEN,
                0,
                "{} must stay out of Explorer's normal file view",
                artifact.display()
            );
        }
    }

    /// The output-fencing contract remains responsible for canonical paths;
    /// each writer additionally declares the suffix its bytes require.
    #[test]
    fn output_path_fencing() {
        let _guard = SESSION_OUTPUT_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_session_output_dir(None);
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        std::fs::create_dir_all(&proj).unwrap();
        assert!(fence_output_path(&proj, None, "exports/out.mp4", OutputPathPolicy::MP4).is_ok());
        assert!(
            fence_output_path(&proj, Some("../evil.mp4"), "x.mp4", OutputPathPolicy::MP4).is_err()
        );
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("evil.mp4");
        assert!(
            fence_output_path(&proj, outside_file.to_str(), "x.mp4", OutputPathPolicy::MP4)
                .is_err()
        );
        std::fs::create_dir_all(proj.join("exports")).unwrap();
        let inside = proj.join("exports/project.json");
        std::fs::write(&inside, b"existing project data file").unwrap();
        assert!(fence_output_path(
            &proj,
            Some(inside.to_str().unwrap()),
            "x.mp4",
            OutputPathPolicy::MP4
        )
        .is_err());
    }

    #[test]
    fn verb_output_suffixes_are_exact_without_relaxing_the_path_fence() {
        let _guard = SESSION_OUTPUT_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_session_output_dir(None);
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        std::fs::create_dir_all(&proj).unwrap();

        // These represent the public writers' concrete byte containers, not a
        // broad media extension allowlist. A correct suffix passes; a different
        // otherwise-safe extension is a contract error before any write occurs.
        for (policy, accepted, rejected) in [
            (OutputPathPolicy::MP4, "render.mp4", "render.webm"),
            (OutputPathPolicy::WEBM, "render.webm", "render.mp4"),
            (OutputPathPolicy::MOV, "master.mov", "master.mp4"),
            (OutputPathPolicy::GIF, "clip.gif", "clip.mp4"),
            (OutputPathPolicy::JPEG, "frame.jpeg", "frame.png"),
            (OutputPathPolicy::SRT, "captions.srt", "captions.vtt"),
            (OutputPathPolicy::VTT, "captions.vtt", "captions.srt"),
            (OutputPathPolicy::ASS, "captions.ass", "captions.srt"),
            (OutputPathPolicy::OTIO, "timeline.otio", "timeline.json"),
            (OutputPathPolicy::EDL, "timeline.edl", "timeline.txt"),
            (OutputPathPolicy::TXT, "chapters.txt", "chapters.md"),
            (OutputPathPolicy::HTML, "review.html", "review.htm"),
        ] {
            assert!(
                fence_output_path(&proj, Some(accepted), "exports/default", policy).is_ok(),
                "{accepted} should satisfy its own output policy"
            );
            let error = fence_output_path(&proj, Some(rejected), "exports/default", policy)
                .expect_err("a different output type must be rejected");
            assert_eq!(error.code, error_codes::INVALID_ARGS, "{rejected}");
        }

        // A valid suffix never bypasses the canonical fence.
        let traversal = fence_output_path(
            &proj,
            Some("../escape.mp4"),
            "exports/default.mp4",
            OutputPathPolicy::MP4,
        )
        .expect_err("traversal with a valid output suffix must remain fenced");
        assert!(traversal.message.contains(".."));
        let outside = tempfile::tempdir().unwrap();
        let outside_path = outside.path().join("escape.mp4");
        assert!(
            fence_output_path(
                &proj,
                outside_path.to_str(),
                "exports/default.mp4",
                OutputPathPolicy::MP4,
            )
            .is_err(),
            "a valid suffix outside the authorized roots must remain refused"
        );
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "Windows concurrent output lease allocation is tracked separately"
    )]
    fn default_output_names_are_reserved_atomically_for_concurrent_writers() {
        let _guard = SESSION_OUTPUT_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_session_output_dir(None);
        let dir = tempfile::tempdir().unwrap();
        let project = std::sync::Arc::new(dir.path().join("p.cutproj"));
        std::fs::create_dir_all(project.join("exports")).unwrap();
        let workers = 16;
        let start = std::sync::Arc::new(std::sync::Barrier::new(workers + 1));
        let hold_reservations = std::sync::Arc::new(std::sync::Barrier::new(workers + 1));
        let (tx, rx) = std::sync::mpsc::channel();
        let mut joins = Vec::new();

        for _ in 0..workers {
            let project = project.clone();
            let start = start.clone();
            let hold_reservations = hold_reservations.clone();
            let tx = tx.clone();
            joins.push(std::thread::spawn(move || {
                start.wait();
                let output = fence_output_path(
                    project.as_ref(),
                    None,
                    "exports/recording.mp4",
                    OutputPathPolicy::MP4,
                );
                let result = output
                    .as_ref()
                    .map(|output| output.to_path_buf())
                    .map_err(|err| format!("{err:?}"));
                tx.send(result).unwrap();
                // Keep every reservation live until the parent has inspected all
                // choices. Without create_new sidecars this deterministically
                // exposes the old exists-then-use collision.
                hold_reservations.wait();
                drop(output);
            }));
        }
        drop(tx);
        start.wait();
        let outputs: Vec<_> = (0..workers).map(|_| rx.recv().unwrap()).collect();
        let canonical_project = std::fs::canonicalize(project.as_ref()).unwrap();
        hold_reservations.wait();
        for join in joins {
            join.join().expect("allocation worker");
        }
        let names: std::collections::BTreeSet<PathBuf> = outputs
            .into_iter()
            .map(|output| output.expect("concurrent default output allocation"))
            .collect();
        assert_eq!(
            names.len(),
            workers,
            "each concurrent default gets a unique path"
        );
        assert!(names.contains(&canonical_project.join("exports/recording.mp4")));
        assert!(names.contains(&canonical_project.join("exports/recording-2.mp4")));
        assert!(
            names
                .iter()
                .all(|path| !reservation_marker_path(path).exists()),
            "normal completion releases every hidden reservation marker"
        );
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "Windows concurrent output lease allocation is tracked separately"
    )]
    fn explicit_save_as_conflicts_with_a_live_default_name_reservation() {
        let _guard = SESSION_OUTPUT_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_session_output_dir(None);
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        std::fs::create_dir_all(&proj).unwrap();

        let reserved =
            fence_output_path(&proj, None, "exports/recording.mp4", OutputPathPolicy::MP4)
                .expect("default name reservation");
        let reserved_path = reserved.to_path_buf();
        assert!(reservation_marker_path(&reserved_path).exists());

        let error = fence_output_path(
            &proj,
            Some("exports/recording.mp4"),
            "exports/recording.mp4",
            OutputPathPolicy::MP4,
        )
        .expect_err("explicit Save As must not race a live default writer");
        assert_eq!(error.code, error_codes::CONFLICT);
        assert!(error.message.contains("reserved"));

        drop(reserved);
        let explicit = fence_output_path(
            &proj,
            Some("exports/recording.mp4"),
            "exports/recording.mp4",
            OutputPathPolicy::MP4,
        )
        .expect("the explicit path becomes available when the writer releases it");
        assert_eq!(explicit.as_ref(), reserved_path);
    }

    #[test]
    fn unlocked_stale_reservation_marker_is_reclaimed() {
        let _guard = SESSION_OUTPUT_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_session_output_dir(None);
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        let output = proj.join("exports/recording.mp4");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        let marker = reservation_marker_path(&output);
        std::fs::write(&marker, b"writer crashed before cleanup").unwrap();

        let recovered = fence_output_path(
            &proj,
            Some("exports/recording.mp4"),
            "exports/recording.mp4",
            OutputPathPolicy::MP4,
        )
        .expect("an unlocked crash-left marker is reclaimable");
        let canonical_output = std::fs::canonicalize(&proj)
            .unwrap()
            .join("exports/recording.mp4");
        assert_eq!(recovered.as_ref(), canonical_output);
        assert_eq!(
            std::fs::read(&marker).unwrap(),
            b"",
            "the stale marker was replaced by the new live reservation"
        );

        drop(recovered);
        assert!(
            !marker.exists(),
            "normal completion still cleans up the visible reservation marker"
        );
    }

    #[test]
    fn rejected_relative_traversal_never_creates_an_outside_parent() {
        let _guard = SESSION_OUTPUT_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_session_output_dir(None);
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&proj).unwrap();

        let error = fence_output_path(
            &proj,
            Some("../outside/out.mp4"),
            "exports/recording.mp4",
            OutputPathPolicy::MP4,
        )
        .expect_err("relative traversal must be refused before creating its parent");
        assert_eq!(error.code, error_codes::INVALID_ARGS);
        assert!(
            !outside.exists(),
            "the rejected path must leave no outside directory"
        );

        let nested = fence_output_path(
            &proj,
            Some("exports/nested/out.mp4"),
            "exports/recording.mp4",
            OutputPathPolicy::MP4,
        )
        .expect("a valid nested relative path remains supported");
        let canonical_nested = std::fs::canonicalize(&proj)
            .unwrap()
            .join("exports/nested/out.mp4");
        assert_eq!(nested.as_ref(), canonical_nested);
        assert!(proj.join("exports/nested").is_dir());
    }

    /// comment.export with a default export folder chosen: `render.final`
    /// delivers the review render THERE, so the receipt's
    /// output_path sits outside `<project>/exports` and the old
    /// `fenced_existing_file_under_dir(&dir.join("exports"), …)` refused a file the
    /// engine had just written itself (`render=done; export=false`). The read fence
    /// must be the same authorized set the serve routes use — and must still refuse
    /// a path in neither root, and still refuse the project's own private files.
    #[test]
    fn review_render_reads_from_every_authorized_export_root() {
        let _guard = SESSION_OUTPUT_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_session_output_dir(None);
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        std::fs::create_dir_all(proj.join("exports")).unwrap();
        let inside = proj.join("exports/render_001.mp4");
        std::fs::write(&inside, b"render inside the project").unwrap();
        let chosen = tempfile::tempdir().unwrap();
        let delivered = chosen.path().join("render_001.mp4");
        std::fs::write(&delivered, b"render delivered to the chosen folder").unwrap();
        let unrelated = tempfile::tempdir().unwrap();
        let elsewhere = unrelated.path().join("render_001.mp4");
        std::fs::write(&elsewhere, b"render in a folder nobody authorized").unwrap();
        let private = proj.join("project.json");
        std::fs::write(&private, b"{}").unwrap();

        // No session dir: the project's exports subtree is the only root.
        assert!(fenced_existing_export_read(&proj, &inside, "review render", "x").is_ok());
        assert!(fenced_existing_export_read(&proj, &delivered, "review render", "x").is_err());

        // The user chose a default export folder — the render that landed there is
        // now readable, and nothing else moved.
        set_session_output_dir(Some(chosen.path().to_path_buf()));
        assert!(
            fenced_existing_export_read(&proj, &delivered, "review render", "x").is_ok(),
            "a render delivered to the chosen export folder must be packageable"
        );
        assert!(fenced_existing_export_read(&proj, &inside, "review render", "x").is_ok());
        assert!(
            fenced_existing_export_read(&proj, &elsewhere, "review render", "x").is_err(),
            "an unauthorized folder stays refused"
        );
        assert!(
            fenced_existing_export_read(&proj, &private, "review render", "x").is_err(),
            "the read fence never reaches the project's own files"
        );
        assert!(
            fenced_existing_export_read(
                &proj,
                &proj.join("exports/missing.mp4"),
                "review render",
                "x"
            )
            .is_err(),
            "a missing file is refused, never substituted"
        );
        set_session_output_dir(None);
    }

    /// render.bundle: a publish package is ONE directory, and its manifest
    /// path is hard-coded into the project tree. With a
    /// session output dir in force the old `fence_output_path(&dir, None, rel)`
    /// diverted the platform clips out of the project AND flattened every aspect
    /// onto one file name, so `<project>/exports/<bundle_id>/` was never created and
    /// the manifest write failed the job with ENOENT. Pin both halves.
    #[test]
    fn bundle_package_members_stay_in_the_project_under_a_session_output_dir() {
        let _guard = SESSION_OUTPUT_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        std::fs::create_dir_all(&proj).unwrap();
        let chosen = tempfile::tempdir().unwrap();
        let chosen_canon = chosen.path().canonicalize().unwrap();
        set_session_output_dir(Some(chosen.path().to_path_buf()));

        // The pre-fix helper: both aspects collapse into the chosen folder, and the
        // manifest's directory is never created. This is the defect, pinned.
        let diverted_a = fence_output_path(
            &proj,
            None,
            "exports/bundle_0_1500/9x16/clip.mp4",
            OutputPathPolicy::MP4,
        )
        .unwrap();
        assert_eq!(diverted_a.parent().unwrap(), chosen_canon);

        // The fix: every member resolves inside the package directory the manifest
        // already lives in, and that directory exists once the first clip resolves.
        let manifest_dir = proj.join("exports").join("bundle_0_1500");
        for aspect in ["9x16", "1x1", "16x9"] {
            let rel = format!("exports/bundle_0_1500/{aspect}/clip.mp4");
            let kept = fence_project_output_path(&proj, None, &rel, OutputPathPolicy::MP4).unwrap();
            assert!(
                kept.starts_with(manifest_dir.canonicalize().unwrap()),
                "{aspect} clip must stay in the package dir, got {}",
                kept.display()
            );
            assert!(
                kept.parent().unwrap().ends_with(aspect),
                "{aspect} keeps its own subdirectory instead of flattening"
            );
        }
        assert!(
            manifest_dir.is_dir(),
            "the manifest's directory must exist after the clips resolve"
        );
        set_session_output_dir(None);
    }

    #[test]
    fn default_output_paths_avoid_existing_files() {
        let _guard = SESSION_OUTPUT_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_session_output_dir(None);
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        std::fs::create_dir_all(proj.join("exports")).unwrap();
        let first =
            fence_output_path(&proj, None, "exports/recording.mp4", OutputPathPolicy::MP4).unwrap();
        assert!(first.ends_with("recording.mp4"));
        let first_path = first.to_path_buf();
        std::fs::write(&first_path, b"existing recording").unwrap();
        drop(first);
        let second =
            fence_output_path(&proj, None, "exports/recording.mp4", OutputPathPolicy::MP4).unwrap();
        assert!(second.ends_with("recording-2.mp4"));
        assert!(!second.exists());
        let explicit = fence_output_path(
            &proj,
            Some(first_path.to_str().unwrap()),
            "exports/recording.mp4",
            OutputPathPolicy::MP4,
        )
        .unwrap();
        assert_eq!(
            explicit.as_ref(),
            first_path,
            "explicit Save As paths stay exact"
        );

        let outside = tempfile::tempdir().unwrap();
        set_session_output_dir(Some(outside.path().to_path_buf()));
        let selected = outside.path().join("selected.mp4");
        let resolved = fence_output_path(
            &proj,
            Some(selected.to_str().unwrap()),
            "exports/selected.mp4",
            OutputPathPolicy::MP4,
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

//! output_paths.rs — shared output-folder preference and output path fencing.
//!
//! Role: own the session default export folder (`project.set_output_dir`) and the
//! path-fence helpers used by file-writing verbs. Dispatch remains the verb
//! router and export/render handlers call this module for output resolution.

#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    fs::File,
    ops::Deref,
    path::{Component, Path, PathBuf},
};

use cut_core::{error_codes, CutError, VerbResult};
use cut_media::PathFence;
use fs2::FileExt;
use serde_json::{json, Value};

mod authorization;
pub(crate) use authorization::OutputAuthorization;
mod lease_identity;
pub(crate) mod monitor_scratch;
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
    // The per-output lock file. Its OS-managed advisory lock is released on
    // process death, so a future writer can safely reclaim a crash-left marker
    // without guessing whether another process is still encoding the same
    // destination. Item 98: the owner deletes it on release (see
    // lease_identity for why that cannot split ownership across files).
    lease_path: PathBuf,
    lease: File,
}

impl Drop for OutputReservation {
    fn drop(&mut self) {
        // Remove the visible marker and then the lock file's name while the
        // advisory lock is still held; the File drops right after this method.
        // A writer that opened the old file meanwhile sees on its identity
        // check that the name is gone and opens it again.
        let _ = std::fs::remove_file(&self.marker);
        // Only the file this reservation created and locked is ever deleted:
        // the identity check fails for anything else at that name, including
        // a link planted there, which is then left alone and logged.
        if lease_identity::still_named(&self.lease, &self.lease_path) {
            let _ = std::fs::remove_file(&self.lease_path);
        } else {
            tracing::warn!(
                lease = %self.lease_path.display(),
                "output lock file no longer names this reservation's file; left in place"
            );
        }
    }
}

/// Session-global output directory chosen by the user (project.set_output_dir /
/// the UI "Choose folder..."). When set, file-writing verbs that don't get an
/// explicit `path` write their default-named file HERE instead of
/// <project>/exports, and it becomes an allowed PathFence root. Process-global
/// because cutd serves a single open project per process (same scope as the
/// CUTD_OUTPUTS_DIR env root). A workspace preference — NOT part of the project
/// or the op-log, never replayed.
#[cfg(not(test))]
static SESSION_OUTPUT_DIR: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);
// In a test binary every test shares ONE process, so a process-wide folder
// leaks from one test into every concurrent one (a queued render of another
// test once wrote into a folder that test had already deleted). Each test
// therefore owns its folder: tests run on their own thread, and the value
// lives there. Product code keeps the single engine-wide value above.
#[cfg(test)]
type TestFolderStore = std::sync::Arc<std::sync::Mutex<Option<PathBuf>>>;

#[cfg(test)]
thread_local! {
    /// This test's folder. A runtime the test builds hands the same store to its
    /// own worker threads (`test_fixture::thread_start_hook`); no other test
    /// can reach it.
    static SESSION_OUTPUT_DIR: std::cell::RefCell<TestFolderStore> =
        std::cell::RefCell::new(TestFolderStore::default());
}
#[cfg(test)]
static OUTPUT_WRITE_SEQ: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
mod reservation_link_tests;
/// The session output dir is process-global (it is a user preference for the
/// whole cutd session), and `cargo test` runs the tests in one binary on
/// several threads — so every test that reads or writes it has to take this
/// lock, or an unrelated test's `set_session_output_dir` decides the outcome.
/// Poisoning is recovered from deliberately: a panicking test tells us about
/// its own assertion, not about lock hygiene.
#[cfg(test)]
pub(crate) mod test_fixture;

#[cfg(test)]
pub(crate) static SESSION_OUTPUT_DIR_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The current session output dir, if set AND still an existing directory
/// (a folder deleted out from under us is ignored, not a hard export failure).
fn session_output_dir() -> Option<PathBuf> {
    if let Some(captured) = authorization::scoped_default_dir() {
        return captured;
    }
    unscoped_session_output_dir()
}

/// The app-wide export folder when one is chosen and exists.
pub(crate) fn app_export_folder() -> Option<PathBuf> {
    unscoped_session_output_dir()
}

fn unscoped_session_output_dir() -> Option<PathBuf> {
    let dir = chosen_session_output_dir()?;
    dir.is_dir().then_some(dir)
}

/// The folder as chosen, even when it has since been deleted. The choice is a
/// remembered app-wide preference: it is read back from the app-data settings
/// file once per run, so it survives a restart of the engine.
#[cfg(not(test))]
fn chosen_session_output_dir() -> Option<PathBuf> {
    static LOADED: std::sync::Once = std::sync::Once::new();
    LOADED.call_once(|| {
        if let Some(saved) = saved_folder_file().and_then(|file| read_saved_folder(&file)) {
            if let Ok(mut guard) = SESSION_OUTPUT_DIR.write() {
                guard.get_or_insert(saved);
            }
        }
    });
    SESSION_OUTPUT_DIR.read().ok()?.clone()
}

/// Where the remembered export folder is kept: in the state home with the other
/// per-user state (`userdata`), so `SHELLX_CUT_HOME` moves it too.
fn saved_folder_file() -> Option<PathBuf> {
    crate::userdata::export_folder_path()
}

/// The folder saved in `file`, if the file exists and names one.
fn read_saved_folder(file: &Path) -> Option<PathBuf> {
    let value: Value = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
    let dir = value.get("dir")?.as_str()?.trim();
    (!dir.is_empty()).then(|| PathBuf::from(dir))
}

/// Remember (Some) or forget (None) the export folder in `file`.
fn write_saved_folder(file: &Path, dir: Option<&Path>) -> std::io::Result<()> {
    match dir {
        Some(dir) => {
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(
                file,
                json!({ "dir": dir.display().to_string() }).to_string(),
            )
        }
        None => match std::fs::remove_file(file) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        },
    }
}

#[cfg(test)]
fn chosen_session_output_dir() -> Option<PathBuf> {
    SESSION_OUTPUT_DIR.with(|store| store.borrow().lock().ok()?.clone())
}

/// Set (Some) or clear (None) the session output dir.
#[cfg(not(test))]
pub(crate) fn set_session_output_dir(dir: Option<PathBuf>) {
    // Load the saved choice first, so this write replaces it rather than racing it.
    let _ = chosen_session_output_dir();
    if let Ok(mut g) = SESSION_OUTPUT_DIR.write() {
        persist_session_output_dir(dir.as_deref());
        *g = dir;
    }
}

#[cfg(not(test))]
fn persist_session_output_dir(dir: Option<&Path>) {
    if let Some(file) = saved_folder_file() {
        if let Err(error) = write_saved_folder(&file, dir) {
            tracing::warn!(%error, "could not remember the export folder across restarts");
        }
    }
}

#[cfg(not(test))]
fn set_session_output_dir_if_empty(dir: PathBuf) -> bool {
    let _ = chosen_session_output_dir();
    let Ok(mut current) = SESSION_OUTPUT_DIR.write() else {
        return false;
    };
    if current.is_some() {
        return false;
    }
    persist_session_output_dir(Some(&dir));
    *current = Some(dir);
    true
}

#[cfg(test)]
pub(crate) fn set_session_output_dir(dir: Option<PathBuf>) {
    // A test's temp folder must meet the render's permission rule whatever the
    // machine's umask is (many Linux accounts create 0775 folders by default).
    if let Some(dir) = &dir {
        test_fixture::make_private(dir);
    }
    SESSION_OUTPUT_DIR.with(|store| {
        if let Ok(mut slot) = store.borrow().lock() {
            *slot = dir;
        }
    });
}

#[cfg(test)]
fn set_session_output_dir_if_empty(dir: PathBuf) -> bool {
    test_fixture::make_private(&dir);
    SESSION_OUTPUT_DIR.with(|store| {
        let shared = store.borrow().clone();
        let Ok(mut current) = shared.lock() else {
            return false;
        };
        if current.is_some() {
            return false;
        }
        *current = Some(dir);
        true
    })
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
    // A project is untrusted data. Its literal exports component must be a
    // plain local directory, not a link/reparse point that canonicalization
    // would turn into standing read authority elsewhere on the host. The
    // record-recovery helper carries the Windows name-surrogate (junction) check as well
    // as the cross-platform symlink check.
    if let Ok(project) = project_dir.canonicalize() {
        let exports = project.join("exports");
        if record_recovery::is_plain_dir(&exports).unwrap_or(false) {
            push(exports);
        }
        // Item 98: the app's playback audio (export.audio scratch) lives in
        // the engine's private folder, never derived from the project path.
        if let Some(scratch) = monitor_scratch::monitor_dir() {
            push(scratch);
        }
    }
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

/// The render stages exports privately first, then moves them into the final
/// folder (or copies via a brief destination temp on cross-volume moves), and
/// still refuses a folder that EVERYONE can write to unless its sticky bit is
/// set; a folder only the group can write to is fine. The chosen-folder check
/// applies the same rule through `cut_media::check_output_dir_policy`.
fn shared_folder_refusal(error: CutError) -> CutError {
    if !error
        .message
        .contains("can be written by anyone on this computer")
    {
        return error;
    }
    let folder = error
        .message
        .rsplit_once(": ")
        .map(|(_, path)| path.to_owned())
        .unwrap_or_default();
    CutError::new(
        error_codes::IO,
        format!("export folder can be written by anyone on this computer: {folder}"),
        "exports stage in a private user-only folder first, and a cross-volume export briefly places a verified copy in the folder; a folder everyone on this computer can write to without the sticky bit is refused",
    )
}

/// Prove the chosen export folder still exists and accepts files; otherwise the
/// plain refusal that names the folder and the way forward.
pub(crate) fn check_export_folder(dir: &Path) -> Result<(), CutError> {
    let result = if dir.is_dir() {
        cut_media::folder_access::ensure_writable_folder("export folder", dir)
            .and_then(|()| cut_media::check_output_dir_policy(dir).map_err(shared_folder_refusal))
    } else {
        Err(cut_media::folder_access::folder_error(
            "export folder",
            dir,
            &std::io::Error::from(std::io::ErrorKind::NotFound),
        ))
    };
    result.map_err(|error| {
        error.with_suggested_action(
            "nothing was exported. Choose another export folder (Export > Choose folder, or project.set_output_dir), change this folder's permissions so that not everyone can write to it (or set its sticky bit), or clear the choice to use the project's exports folder (project.set_output_dir with no dir)",
        )
        .with_requirement("export_folder")
    })
}

/// How often the engine re-checks a chosen export folder on its own. A folder
/// deleted, unmounted or locked while the app runs then shows as unavailable
/// (status bar, `ui.state.export_folder`, the event) within this time, without a
/// restart and without anything else asking. Modest on purpose: one directory
/// stat and one mode check, no file is written.
const FOLDER_WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

/// The cheap part of "is the chosen folder usable": it exists and meets the
/// render's permission rule. The write probe is left to the explicit checks.
fn folder_watch_key() -> (Option<String>, bool) {
    match chosen_session_output_dir() {
        None => (None, true),
        Some(dir) => {
            let usable = dir.is_dir() && cut_media::check_output_dir_policy(&dir).is_ok();
            (Some(dir.display().to_string()), usable)
        }
    }
}

/// One look: publish `output_folder_changed` when the folder or its usability
/// differs from the previous look. The first look only records.
#[cfg(test)]
fn folder_watch_step(
    last: &mut Option<(Option<String>, bool)>,
    events: &crate::events::EventBus,
) -> bool {
    let now = folder_watch_key();
    let changed = last.as_ref().is_some_and(|previous| *previous != now);
    if changed {
        events.publish(crate::events::Event::OutputFolderChanged {
            status: output_folder_status(),
        });
    }
    *last = Some(now);
    changed
}

/// The engine's own watch over the chosen export folder (see
/// [`FOLDER_WATCH_INTERVAL`]).
pub(crate) fn spawn_folder_watch(events: crate::events::EventBus) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last: Option<(Option<String>, bool)> = None;
        let mut tick = tokio::time::interval(FOLDER_WATCH_INTERVAL);
        loop {
            tick.tick().await;
            // A hung network drive must not stall the runtime.
            let Ok(now) = tokio::task::spawn_blocking(folder_watch_key).await else {
                continue;
            };
            if last.as_ref().is_some_and(|previous| *previous != now) {
                if let Ok(status) = tokio::task::spawn_blocking(output_folder_status).await {
                    events.publish(crate::events::Event::OutputFolderChanged { status });
                }
            }
            last = Some(now);
        }
    })
}

/// The export destination as the engine holds it (the one truth the status bar,
/// the export menu and an agent all read): `dir` is the chosen folder or null,
/// `destination` is `chosen` or `project_exports`, and an unusable chosen
/// folder reads `usable:false` with its plain `problem` and `suggested_action`.
pub(crate) fn output_folder_status() -> Value {
    let Some(dir) = chosen_session_output_dir() else {
        return json!({
            "dir": Value::Null,
            "destination": "project_exports",
            "usable": true,
            "problem": Value::Null,
            "suggested_action": Value::Null,
        });
    };
    match check_export_folder(&dir) {
        Ok(()) => json!({
            "dir": dir.display().to_string(),
            "destination": "chosen",
            "usable": true,
            "problem": Value::Null,
            "suggested_action": Value::Null,
        }),
        Err(error) => json!({
            "dir": dir.display().to_string(),
            "destination": "chosen",
            "usable": false,
            "problem": error.message,
            "suggested_action": error.suggested_action,
        }),
    }
}

/// Build the project's PathFence (cut-media owns the output-fencing policy). Writes
/// may target the project dir, the env-configured CUTD_OUTPUTS_DIR, and the
/// user's chosen session output dir — those are the only allowed roots.
pub(crate) fn make_fence(project_dir: &Path) -> Result<PathFence, CutError> {
    if let Some(captured) = authorization::scoped_fence(project_dir) {
        return captured;
    }
    make_fence_with_default(project_dir, session_output_dir().as_deref())
}

fn make_fence_with_default(
    project_dir: &Path,
    default_dir: Option<&Path>,
) -> Result<PathFence, CutError> {
    let mut fence = PathFence::new(project_dir)?;
    if let Ok(d) = std::env::var("CUTD_OUTPUTS_DIR") {
        if !d.is_empty() {
            fence = fence.with_extra_root(Path::new(&d))?;
        }
    }
    if let Some(dir) = default_dir {
        fence = fence.with_extra_root(dir)?;
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
    if requested.is_none() && authorization::scoped_default_dir().is_none() {
        // The person's chosen export folder is where this file would go. If it
        // is gone or cannot be written, say so NOW, in plain words, before any
        // encoder starts: never fall back silently and never fail at the end
        // with an ffmpeg "No such file or directory".
        if let Some(dir) = chosen_session_output_dir() {
            check_export_folder(&dir)?;
        }
    }
    let (mut candidate, create_project_parent) = match requested {
        Some(p) => {
            let pb = PathBuf::from(p);
            reject_relative_output_traversal(&pb)?;
            let parent = pb
                .is_relative()
                .then(|| pb.parent().unwrap_or_else(|| Path::new("")).to_path_buf());
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
                let parent = Path::new(default_rel)
                    .parent()
                    .unwrap_or_else(|| Path::new(""))
                    .to_path_buf();
                (p, Some(parent))
            }
        },
    };
    // Validate the declared output type before creating a caller-supplied
    // directory. The path fence below still owns canonicalization, traversal,
    // symlink and root checks.
    policy.validate(&candidate)?;
    if let Some(parent) = create_project_parent {
        // A relative requested path resolves INSIDE the project (the fence
        // rejects escapes). Create its parent — e.g. exports/ — so an explicit
        // "exports/_monitor_a.mp3" works on a fresh project, matching default
        // project exports. Each literal component is checked before creation,
        // so a static link or Windows junction cannot redirect this
        // mutation before PathFence gets its canonical membership check.
        ensure_plain_project_relative_dir(fence.project_dir(), &parent)?;
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
    let fence = make_fence(project_dir)?;
    let relative = Path::new(default_rel);
    let path = fence.project_dir().join(relative);
    policy.validate(&path)?;
    ensure_plain_project_relative_dir(
        fence.project_dir(),
        relative.parent().unwrap_or_else(|| Path::new("")),
    )?;
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
    let fence = make_fence(project_dir)?;
    let relative = Path::new(relative);
    let path = fence.project_dir().join(relative);
    ensure_plain_project_relative_dir(
        fence.project_dir(),
        relative.parent().unwrap_or_else(|| Path::new("")),
    )?;
    let (path, reservation) = reserve_next_available_output_path(&fence, &path)?;
    drop(reservation);
    Ok(path)
}

/// Create a project-local directory chain only through literal, plain
/// directories. Unlike `create_dir_all`, this refuses a static symlink or a
/// Windows junction before it can redirect creation outside the canonical
/// project root. It intentionally does not claim to resist a concurrent local
/// path swap; that actor is outside Cut's project-input threat boundary.
pub(crate) fn ensure_plain_project_relative_dir(
    project_dir: &Path,
    relative: &Path,
) -> Result<PathBuf, CutError> {
    require_plain_project_dir(project_dir)?;
    let mut current = project_dir.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            if matches!(component, Component::CurDir) {
                continue;
            }
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!(
                    "project output directory {} is not relative",
                    relative.display()
                ),
                "project output directories must use literal child components",
            ));
        };
        let child = current.join(component);
        match std::fs::symlink_metadata(&child) {
            Ok(_) => require_plain_project_dir(&child)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                require_plain_project_dir(&current)?;
                match std::fs::create_dir(&child) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(output_directory_error(&child, error)),
                }
                require_plain_project_dir(&child)?;
            }
            Err(error) => return Err(output_directory_error(&child, error)),
        }
        current = child;
    }
    Ok(current)
}

/// Admit an existing internal project directory without creating anything.
/// Read routes use this before treating a project-owned subdirectory as an
/// authority root: canonicalizing a linked root would otherwise accept files
/// outside the project as if they were project files.
pub(crate) fn existing_plain_project_relative_dir(
    project_dir: &Path,
    relative: &Path,
) -> Result<PathBuf, CutError> {
    require_plain_project_dir(project_dir)?;
    let mut current = project_dir.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            if matches!(component, Component::CurDir) {
                continue;
            }
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("project directory {} is not relative", relative.display()),
                "project directories must use literal child components",
            ));
        };
        current.push(component);
        require_plain_project_dir(&current)?;
    }
    Ok(current)
}

fn require_plain_project_dir(path: &Path) -> Result<(), CutError> {
    match record_recovery::is_plain_dir(path) {
        Ok(true) => Ok(()),
        Ok(false) => Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "project output directory {} is not a plain directory",
                path.display()
            ),
            "project output components cannot be symlinks or Windows junctions",
        )),
        Err(error) => Err(output_directory_error(path, error)),
    }
}

fn output_directory_error(path: &Path, error: impl std::fmt::Display) -> CutError {
    CutError::new(
        error_codes::IO,
        format!(
            "could not inspect or create project output directory {}",
            path.display()
        ),
        error.to_string(),
    )
}

/// Build a sibling temp path for renderers such as ffmpeg that infer the output
/// container from the file extension. Retained for naming tests; production
/// exports stage inside `cut_media::PrivateStageDir`.
#[cfg(test)]
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

/// Verify an already-written staged file and move it atomically into its
/// parent-pinned destination directory without following links at the target or
/// its ancestors.
pub(crate) fn publish_output_atomic(tmp: &Path, path: &Path) -> Result<(), CutError> {
    cut_media::publish_staged_file(tmp, path, true)
}

/// Write a fenced export/sidecar file into Cut's private user-only staging
/// directory, flush to disk, and publish atomically into its parent-pinned
/// destination directory without following links at the target or ancestors.
pub(crate) fn write_output_atomic(path: &Path, bytes: impl AsRef<[u8]>) -> Result<(), CutError> {
    cut_media::write_bytes_privately_and_publish(path, bytes.as_ref())
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
        // A name that is already taken is never overwritten here, so the
        // overwrite rule must not judge it: a default name moves on to the
        // next free one whatever its suffix (a second review page, say).
        if std::fs::symlink_metadata(&candidate).is_ok() {
            continue;
        }
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
    // The lease sits in a folder a received project may have filled. Only an
    // ordinary single-link file (or none) is used; a link or shared file at
    // this name could otherwise create, lock or re-attribute a file elsewhere.
    if cut_core::store::plain_file_state(&lease_path) == cut_core::store::PlainFileState::NotPlain {
        return Err(CutError::new(
            error_codes::CONFLICT,
            format!(
                "output lease {} is a link or not an ordinary file",
                lease_path.display()
            ),
            "export reservation files must be ordinary files inside the output folder",
        )
        .with_suggested_action(
            "delete that hidden file from the output folder, then export again",
        ));
    }
    let mut lease_options = std::fs::OpenOptions::new();
    lease_options
        .create(true)
        .read(true)
        .write(true)
        .truncate(false);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        lease_options.custom_flags(libc::O_NOFOLLOW);
    }
    // A released owner deletes the lock file. Locking a file whose name is
    // already gone proves nothing, so lock, check the name, and retry a few
    // times when an owner released (and deleted) it in between.
    let mut attempts = 0;
    let lease = loop {
        attempts += 1;
        match lock_named_lease(&lease_options, &lease_path)? {
            LeaseLock::Locked(lease) => break lease,
            LeaseLock::Busy => return Ok(None),
            LeaseLock::Stale if attempts < 8 => continue,
            LeaseLock::Stale => return Ok(None),
        }
    };

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
        lease_path,
        lease,
    }))
}

enum LeaseLock {
    Locked(File),
    /// Another live writer holds the lock.
    Busy,
    /// Locked a file whose name an owner deleted meanwhile; open it again.
    Stale,
}

fn lock_named_lease(
    options: &std::fs::OpenOptions,
    lease_path: &Path,
) -> Result<LeaseLock, CutError> {
    let lease = options.open(lease_path).map_err(|error| {
        if matches!(
            error.kind(),
            std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::StorageFull
        ) {
            let folder = lease_path.parent().unwrap_or(lease_path);
            return cut_media::folder_access::folder_error("output folder", folder, &error);
        }
        CutError::new(
            error_codes::IO,
            format!("could not open output lease {}", lease_path.display()),
            error.to_string(),
        )
    })?;
    hide_reservation_artifact(lease_path);
    match lease.try_lock_exclusive() {
        Ok(()) => {}
        Err(error)
            if error.kind() == std::io::ErrorKind::WouldBlock
                || (cfg!(windows) && error.raw_os_error() == Some(33)) =>
        {
            return Ok(LeaseLock::Busy);
        }
        Err(error) => {
            return Err(CutError::new(
                error_codes::IO,
                format!("could not lock output lease {}", lease_path.display()),
                error.to_string(),
            ));
        }
    }
    if lease_identity::still_named(&lease, lease_path) {
        Ok(LeaseLock::Locked(lease))
    } else {
        Ok(LeaseLock::Stale)
    }
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

/// Read a review-package render only from this project's plain exports tree.
/// A delivery folder may receive renders, but a saved receipt cannot grant the
/// review package permission to copy a file from that folder.
pub(crate) fn fenced_existing_project_export_read(
    project_dir: &Path,
    path: &Path,
    label: &str,
    suggested_action: &str,
) -> Result<PathBuf, CutError> {
    let project = project_dir.canonicalize()?;
    let exports = project.join("exports");
    if !record_recovery::is_plain_dir(&exports).unwrap_or(false) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("{label} needs this project's exports folder"),
            format!("{} is missing or is not a plain folder", exports.display()),
        )
        .with_suggested_action(suggested_action));
    }
    fenced_existing_file_under_dir(&exports, path, label, suggested_action)
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

/// project.get_output_dir {} — read the export destination the engine holds.
/// Read-only; works with or without an open project.
pub(crate) async fn project_get_output_dir() -> Result<VerbResult, CutError> {
    Ok(VerbResult::ok(output_folder_status()))
}

/// project.set_output_dir{dir?, only_if_empty?} — choose where exports/renders land when a verb
/// isn't given an explicit `path`. Empty/absent `dir` clears it (back to
/// <project>/exports). The folder must already exist (the UI's native folder
/// picker only returns existing dirs); it is canonicalized and becomes an
/// allowed fence root. An app-wide remembered preference held by the engine
/// (not per project, never an op, never replayed): it stays until cleared, the
/// status bar shows it, and every change publishes `output_folder_changed`.
pub(crate) async fn project_set_output_dir(
    state: &crate::state::AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        dir: Option<String>,
        #[serde(default)]
        only_if_empty: bool,
    }
    let a: Args = crate::dispatch::parse_args(args)?;
    if a.only_if_empty && chosen_session_output_dir().is_some() {
        return Ok(VerbResult::ok(output_folder_status()));
    }
    if a.only_if_empty && a.dir.as_deref().is_none_or(|dir| dir.trim().is_empty()) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "conditional export folder needs a directory",
            "only_if_empty requires a nonempty dir",
        ));
    }
    let cleared = match a.dir.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => {
            set_session_output_dir(None);
            true
        }
        Some(d) => {
            let path = std::fs::canonicalize(d).map_err(|e| {
                cut_media::folder_access::folder_error("output folder", Path::new(d), &e)
            })?;
            if !path.is_dir() {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "output path is not a directory",
                    path.display().to_string(),
                ));
            }
            // An existing folder that refuses new files would only fail later,
            // at the end of an export or a recording. Refuse it now, by name.
            cut_media::folder_access::ensure_writable_folder("output folder", &path)?;
            // The render refuses a folder other users can write to; say so now,
            // not at the end of the first export.
            check_export_folder(&path)?;
            if a.only_if_empty {
                if !set_session_output_dir_if_empty(path) {
                    return Ok(VerbResult::ok(output_folder_status()));
                }
            } else {
                set_session_output_dir(Some(path));
            }
            false
        }
    };
    let mut status = output_folder_status();
    state
        .events
        .publish(crate::events::Event::OutputFolderChanged {
            status: status.clone(),
        });
    if cleared {
        status["cleared"] = json!(true);
    }
    Ok(VerbResult::ok(status))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn conditional_folder_carry_over_preserves_a_newer_choice_and_sets_when_empty() {
        let _fixture = test_fixture::SessionOutputDirFixture::new();
        let state = crate::state::AppState::new();
        let parent = tempfile::tempdir().unwrap();
        let legacy = parent.path().join("legacy");
        let newer = parent.path().join("newer");
        std::fs::create_dir(&legacy).unwrap();
        std::fs::create_dir(&newer).unwrap();
        // The setter stores the canonical folder; on macOS the temporary
        // folder is reached through a link, so compare canonical paths.
        let legacy = std::fs::canonicalize(&legacy).unwrap();
        let newer = std::fs::canonicalize(&newer).unwrap();
        test_fixture::make_private(&legacy);
        test_fixture::make_private(&newer);

        set_session_output_dir(Some(newer.clone()));
        let kept = project_set_output_dir(
            &state,
            json!({ "dir": legacy.display().to_string(), "only_if_empty": true }),
        )
        .await
        .unwrap()
        .result
        .unwrap();
        assert_eq!(kept["dir"], newer.display().to_string());
        assert_eq!(chosen_session_output_dir(), Some(newer));

        set_session_output_dir(None);
        let carried = project_set_output_dir(
            &state,
            json!({ "dir": legacy.display().to_string(), "only_if_empty": true }),
        )
        .await
        .unwrap()
        .result
        .unwrap();
        assert_eq!(carried["dir"], legacy.display().to_string());
        assert_eq!(chosen_session_output_dir(), Some(legacy));
    }

    #[test]
    fn the_remembered_export_folder_lives_in_the_state_home_not_the_tools_folder() {
        let file = saved_folder_file().expect("a state home");
        assert_eq!(
            Some(file.clone()),
            crate::userdata::shellx_cut_home().map(|home| home.join("export-folder.json")),
            "one rule for every per-user state file"
        );
        let old_place = cut_perception::appdata_sidecar_dir()
            .and_then(|dir| dir.parent().map(|parent| parent.join("export-folder.json")));
        assert_ne!(
            Some(file),
            old_place,
            "not next to the tools folder any more"
        );
    }

    #[test]
    fn the_export_folder_choice_is_remembered_in_its_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings").join("export-folder.json");
        assert_eq!(read_saved_folder(&file), None);
        let chosen = dir.path().join("exports out");
        write_saved_folder(&file, Some(&chosen)).unwrap();
        assert_eq!(read_saved_folder(&file), Some(chosen));
        write_saved_folder(&file, None).unwrap();
        assert_eq!(read_saved_folder(&file), None);
        // Clearing twice is fine.
        write_saved_folder(&file, None).unwrap();
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(read_saved_folder(&file), None);
    }

    #[tokio::test]
    async fn a_chosen_folder_that_is_gone_is_refused_before_the_export_starts() {
        let _fixture = test_fixture::SessionOutputDirFixture::new();
        let project = tempfile::tempdir().unwrap();
        let state = crate::state::AppState::new();
        let parent = tempfile::tempdir().unwrap();
        let chosen = parent.path().join("deliveries");
        std::fs::create_dir(&chosen).unwrap();
        test_fixture::make_private(&chosen);
        let set = project_set_output_dir(&state, json!({ "dir": chosen.display().to_string() }))
            .await
            .unwrap();
        let status = set.result.unwrap();
        assert_eq!(status["destination"], "chosen");
        assert_eq!(status["usable"], true);
        // While it is there the default export goes into it.
        let ok = fence_output_path(
            project.path(),
            None,
            "exports/out.mp4",
            OutputPathPolicy::MP4,
        )
        .unwrap();
        assert!(ok.starts_with(chosen.canonicalize().unwrap()));
        drop(ok);

        // The person's folder disappears (an unplugged drive, a deleted folder).
        std::fs::remove_dir_all(&chosen).unwrap();
        let status = project_get_output_dir().await.unwrap().result.unwrap();
        assert_eq!(status["usable"], false);
        assert!(
            status["problem"].as_str().unwrap().contains("not found"),
            "{status}"
        );
        assert!(status["suggested_action"]
            .as_str()
            .unwrap()
            .contains("Choose another export folder"));
        let refused = fence_output_path(
            project.path(),
            None,
            "exports/out.mp4",
            OutputPathPolicy::MP4,
        )
        .expect_err("a vanished export folder must be refused up front");
        assert!(
            refused.message.contains("export folder not found"),
            "{}",
            refused.message
        );
        assert!(
            refused.message.contains("deliveries"),
            "{}",
            refused.message
        );
        let action = refused.suggested_action.unwrap();
        assert!(action.contains("Choose another export folder"), "{action}");
        assert!(action.contains("clear the choice"), "{action}");
        // An explicit path is the caller's own choice and is not blocked by it.
        std::fs::create_dir_all(project.path().join("exports")).unwrap();
        let own = project.path().join("exports").join("mine.mp4");
        assert!(fence_output_path(
            project.path(),
            own.to_str(),
            "exports/out.mp4",
            OutputPathPolicy::MP4
        )
        .is_ok());

        // Clearing returns to the project's exports folder, and says so.
        let cleared = project_set_output_dir(&state, json!({}))
            .await
            .unwrap()
            .result
            .unwrap();
        assert_eq!(cleared["destination"], "project_exports");
        assert_eq!(cleared["cleared"], true);
        assert!(cleared["dir"].is_null());
        let back = fence_output_path(
            project.path(),
            None,
            "exports/out.mp4",
            OutputPathPolicy::MP4,
        )
        .unwrap();
        assert!(back.starts_with(project.path().canonicalize().unwrap()));
    }

    #[tokio::test]
    async fn changing_the_folder_announces_it_on_the_event_stream() {
        let _fixture = test_fixture::SessionOutputDirFixture::new();
        let state = crate::state::AppState::new();
        let mut events = state.events.subscribe();
        let chosen = tempfile::tempdir().unwrap();
        test_fixture::make_private(chosen.path());
        project_set_output_dir(
            &state,
            json!({ "dir": chosen.path().display().to_string() }),
        )
        .await
        .unwrap();
        match events.recv().await.unwrap() {
            crate::events::Event::OutputFolderChanged { status } => {
                assert_eq!(status["destination"], "chosen");
                assert_eq!(status["usable"], true);
            }
            other => panic!("expected output_folder_changed, got {other:?}"),
        }
        project_set_output_dir(&state, json!({})).await.unwrap();
        match events.recv().await.unwrap() {
            crate::events::Event::OutputFolderChanged { status } => {
                assert_eq!(status["destination"], "project_exports");
            }
            other => panic!("expected output_folder_changed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_engine_notices_a_folder_that_disappears_on_its_own() {
        let _fixture = test_fixture::SessionOutputDirFixture::new();
        let state = crate::state::AppState::new();
        let parent = tempfile::tempdir().unwrap();
        let chosen = parent.path().join("Deliveries");
        std::fs::create_dir(&chosen).unwrap();
        test_fixture::make_private(&chosen);
        project_set_output_dir(&state, json!({ "dir": chosen.display().to_string() }))
            .await
            .unwrap();
        let mut events = state.events.subscribe();
        let mut last = None;
        assert!(
            !folder_watch_step(&mut last, &state.events),
            "the first look only records"
        );
        assert!(
            !folder_watch_step(&mut last, &state.events),
            "nothing changed"
        );
        std::fs::remove_dir_all(&chosen).unwrap();
        assert!(
            folder_watch_step(&mut last, &state.events),
            "the folder is gone"
        );
        match events.recv().await.unwrap() {
            crate::events::Event::OutputFolderChanged { status } => {
                assert_eq!(status["usable"], false, "{status}");
                assert!(status["problem"].as_str().unwrap().contains("not found"));
            }
            other => panic!("expected output_folder_changed, got {other:?}"),
        }
        assert!(
            !folder_watch_step(&mut last, &state.events),
            "told once, not every look"
        );
        std::fs::create_dir(&chosen).unwrap();
        test_fixture::make_private(&chosen);
        assert!(
            folder_watch_step(&mut last, &state.events),
            "and again when it comes back"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_export_folder_rule_allows_the_group_and_refuses_everyone_unless_sticky() {
        use std::os::unix::fs::PermissionsExt;
        let _fixture = test_fixture::SessionOutputDirFixture::new();
        let state = crate::state::AppState::new();
        let parent = tempfile::tempdir().unwrap();
        let shared = parent.path().join("team");
        std::fs::create_dir(&shared).unwrap();
        let mode = |bits: u32| {
            std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(bits)).unwrap()
        };
        let set = || project_set_output_dir(&state, json!({ "dir": shared.display().to_string() }));

        // Group-writable (775, 770): allowed, as a shared team or family folder.
        for bits in [0o775, 0o770, 0o2775] {
            mode(bits);
            let status = set().await.unwrap().result.unwrap();
            assert_eq!(status["usable"], true, "{bits:o}: {status}");
            assert!(
                cut_media::check_output_dir_policy(&shared).is_ok(),
                "{bits:o}"
            );
        }
        // Everyone can write (777, 757, 772 style other-write) without sticky: refused.
        for bits in [0o777, 0o757, 0o773] {
            mode(bits);
            let refused = set()
                .await
                .expect_err("other-writable without sticky is refused");
            assert!(
                refused
                    .message
                    .starts_with("export folder can be written by anyone on this computer: "),
                "{bits:o}: {}",
                refused.message
            );
            assert!(refused.message.contains("team"), "{}", refused.message);
            let action = refused.suggested_action.unwrap();
            assert!(action.contains("Choose another export folder"), "{action}");
            assert!(action.contains("not everyone can write"), "{action}");
            assert!(
                cut_media::check_output_dir_policy(&shared).is_err(),
                "{bits:o}"
            );
        }
        // Everyone can write but the sticky bit protects each user's files (/tmp): allowed.
        for bits in [0o1777, 0o1757] {
            mode(bits);
            assert_eq!(
                set().await.unwrap().result.unwrap()["usable"],
                true,
                "{bits:o}"
            );
        }

        // Chosen while private and opened to everyone later: unusable at once
        // (the chip and ui.state follow), and the default export is refused
        // before it starts.
        mode(0o755);
        set().await.unwrap();
        mode(0o777);
        let status = project_get_output_dir().await.unwrap().result.unwrap();
        assert_eq!(status["usable"], false, "{status}");
        assert!(status["problem"]
            .as_str()
            .unwrap()
            .contains("can be written by anyone on this computer"));
        let project = tempfile::tempdir().unwrap();
        let denied = fence_output_path(
            project.path(),
            None,
            "exports/out.mp4",
            OutputPathPolicy::MP4,
        )
        .expect_err("refused before any encoder starts");
        assert!(
            denied
                .message
                .contains("can be written by anyone on this computer"),
            "{}",
            denied.message
        );
        // Group-writable again: fine for the render's own check and the export.
        mode(0o775);
        assert!(cut_media::check_output_dir_policy(&shared).is_ok());
        assert!(fence_output_path(
            project.path(),
            None,
            "exports/out.mp4",
            OutputPathPolicy::MP4
        )
        .is_ok());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn output_folder_that_cannot_be_written_is_refused_by_name() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            // Root ignores directory permissions; nothing to observe.
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = project_set_output_dir(
            &crate::state::AppState::new(),
            json!({ "dir": locked.display().to_string() }),
        )
        .await;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = result.unwrap_err();
        assert_eq!(error.code, error_codes::IO);
        assert!(
            error.message.contains("cannot be written"),
            "{}",
            error.message
        );
        assert!(error.message.contains("locked"), "{}", error.message);
        assert!(!error.message.contains("not found"), "{}", error.message);
    }

    #[tokio::test]
    async fn missing_output_folder_is_reported_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("absent");
        let error = project_set_output_dir(
            &crate::state::AppState::new(),
            json!({ "dir": missing.display().to_string() }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, error_codes::NOT_FOUND);
        assert!(
            error.message.starts_with("output folder not found"),
            "{}",
            error.message
        );
        assert_eq!(
            error.suggested_action.as_deref(),
            Some("create the folder first, or choose an existing folder")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_output_folder_is_named_not_reported_as_a_lease_failure() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            return; // root ignores directory permissions
        }
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = try_reserve_output_path(&locked.join("render.mp4"));
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = result.expect_err("a read-only folder must be refused");
        assert!(
            error.message.starts_with("output folder cannot be written"),
            "{}",
            error.message
        );
        assert!(!error.message.contains("lease"), "{}", error.message);
        assert!(error.suggested_action.is_some());
    }

    #[cfg(unix)]
    fn symlink_dir(original: &Path, link: &Path) {
        std::os::unix::fs::symlink(original, link).expect("create project link");
    }

    #[cfg(windows)]
    #[test]
    fn live_reservation_artifacts_are_hidden_on_windows() {
        let _guard = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
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
        let _guard = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
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
        let _guard = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
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
    fn default_output_names_are_reserved_atomically_for_concurrent_writers() {
        let _guard = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
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
    fn explicit_save_as_conflicts_with_a_live_default_name_reservation() {
        let _guard = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
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

    /// Item 98: the owner deletes its lock file when it releases the name,
    /// and a writer that opened the old file before that sees the name is gone.
    #[test]
    fn released_reservation_leaves_no_lock_file() {
        let _guard = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
        set_session_output_dir(None);
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        std::fs::create_dir_all(proj.join("exports")).unwrap();
        let output =
            fence_output_path(&proj, None, "exports/render_001.mp4", OutputPathPolicy::MP4)
                .unwrap();
        let lease_path = reservation_lease_path(&output);
        assert!(
            lease_path.exists(),
            "a live reservation holds its lock file"
        );
        let early_opener = std::fs::File::open(&lease_path).unwrap();
        drop(output);
        assert!(!lease_path.exists(), "the owner removed its lock file");
        assert!(
            !lease_identity::still_named(&early_opener, &lease_path),
            "a handle to the removed file is not the current lease"
        );
        let again = fence_output_path(&proj, None, "exports/render_001.mp4", OutputPathPolicy::MP4)
            .unwrap();
        assert!(lease_path.exists());
        drop(again);
        let leftovers: Vec<_> = std::fs::read_dir(proj.join("exports"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    /// A default name that is already taken moves on to the next free name even
    /// when its suffix is not one the overwrite rule allows: exporting a review
    /// package twice gives a second page, not a refusal.
    #[test]
    fn default_name_moves_past_an_existing_page_of_any_suffix() {
        let _guard = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
        set_session_output_dir(None);
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        std::fs::create_dir_all(proj.join("exports")).unwrap();
        std::fs::write(proj.join("exports/review_render_001.html"), b"first page").unwrap();
        let second = fence_project_output_path(
            &proj,
            None,
            "exports/review_render_001.html",
            OutputPathPolicy::HTML,
        )
        .expect("a taken default name is skipped, not refused");
        assert_eq!(
            second.file_name().and_then(|name| name.to_str()),
            Some("review_render_001-2.html")
        );
        assert_eq!(
            std::fs::read(proj.join("exports/review_render_001.html")).unwrap(),
            b"first page",
            "the first page is untouched"
        );
    }

    #[test]
    fn unlocked_stale_reservation_marker_is_reclaimed() {
        let _guard = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
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
        let _guard = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
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

    #[cfg(unix)]
    #[test]
    fn project_export_link_never_becomes_an_authorized_read_root() {
        let _guard = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
        set_session_output_dir(None);
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let private_export_like_file = outside.join("private.json");
        std::fs::write(&private_export_like_file, b"private host data").unwrap();
        symlink_dir(&outside, &proj.join("exports"));

        assert!(
            fenced_existing_export_read(&proj, &private_export_like_file, "export", "x").is_err(),
            "a static project exports link must not grant read authority outside the project"
        );
        assert!(
            authorized_export_read_roots(&proj).is_empty(),
            "a linked project exports directory must not be retained as a read root"
        );
        assert!(
            fenced_existing_project_export_read(&proj, &private_export_like_file, "render", "x")
                .is_err(),
            "a linked exports directory cannot become a review-package source"
        );
    }

    #[cfg(unix)]
    #[test]
    fn linked_project_components_never_create_outside_directories() {
        let _guard = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
        set_session_output_dir(None);
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        symlink_dir(&outside, &proj.join("exports"));

        assert!(
            fence_output_path(
                &proj,
                Some("exports/explicit/nested.mp4"),
                "exports/default.mp4",
                OutputPathPolicy::MP4,
            )
            .is_err(),
            "the canonical fence must refuse the redirected explicit output"
        );
        assert!(
            fence_project_output_path(
                &proj,
                None,
                "exports/default/nested.mp4",
                OutputPathPolicy::MP4,
            )
            .is_err(),
            "the canonical fence must refuse the redirected default output"
        );
        assert!(
            !outside.join("explicit").exists() && !outside.join("default").exists(),
            "a rejected output must not create directories through a project link"
        );

        let motion_outside = dir.path().join("motion-outside");
        std::fs::create_dir_all(&motion_outside).unwrap();
        symlink_dir(&motion_outside, &proj.join("motion-sources"));
        assert!(
            fence_project_directory_path(&proj, "motion-sources/clip/run").is_err(),
            "the canonical fence must refuse the redirected internal package path"
        );
        assert!(
            !motion_outside.join("clip").exists(),
            "a rejected package path must not create directories through a project link"
        );
    }

    #[test]
    fn plain_project_components_remain_creatable_and_readable() {
        let _guard = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
        set_session_output_dir(None);
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        std::fs::create_dir_all(&proj).unwrap();

        let output = fence_project_output_path(
            &proj,
            None,
            "exports/bundle/clip.mp4",
            OutputPathPolicy::MP4,
        )
        .expect("ordinary project export directories remain supported");
        assert!(output.starts_with(proj.canonicalize().unwrap()));
        assert!(proj.join("exports/bundle").is_dir());

        let package = fence_project_directory_path(&proj, "motion-sources/clip/run")
            .expect("ordinary project package directories remain supported");
        assert!(package.starts_with(proj.canonicalize().unwrap()));
        assert!(proj.join("motion-sources/clip").is_dir());

        let readable = proj.join("exports/bundle/receipt.json");
        std::fs::write(&readable, b"{}").unwrap();
        assert!(fenced_existing_export_read(&proj, &readable, "export", "x").is_ok());
        set_session_output_dir(None);
    }

    /// General export serving and verification can read a render from a chosen
    /// output folder. A review package has a narrower source fence below.
    #[test]
    fn export_reads_from_every_authorized_export_root() {
        let _guard = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
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

    #[test]
    fn review_package_reads_only_this_projects_exports() {
        let _guard = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        std::fs::create_dir_all(proj.join("exports/nested")).unwrap();
        let inside = proj.join("exports/nested/render.mp4");
        std::fs::write(&inside, b"project render").unwrap();
        let chosen = tempfile::tempdir().unwrap();
        let delivered = chosen.path().join("render.mp4");
        std::fs::write(&delivered, b"delivery render").unwrap();
        let private = proj.join("project.json");
        std::fs::write(&private, b"private").unwrap();
        set_session_output_dir(Some(chosen.path().to_path_buf()));
        assert!(fenced_existing_project_export_read(&proj, &inside, "render", "x").is_ok());
        assert!(fenced_existing_project_export_read(&proj, &delivered, "render", "x").is_err());
        assert!(fenced_existing_project_export_read(&proj, &private, "render", "x").is_err());
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
        let _guard = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
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
        let _guard = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
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

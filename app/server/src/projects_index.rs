//! Role: the global PROJECTS INDEX — a small registry at `~/.shellx-cut/projects.json`
//! that records every project created/opened so the UI can show "recent projects"
//! and reopen one (the engine reopen via `project.open{path}` already works; Cut
//! simply had no way to DISCOVER projects — no `project.list`). An entry LINKS a
//! `.cutproj` by absolute path; the project files themselves stay in the visible
//! `default_projects_dir()`. The index is pure metadata.
//!
//! Layering (mirrors receipt.rs / the Canvas manifest split): the manifest OPS
//! (`upsert`/`touch`/`remove`/`query`/`migrate`) are PURE and take an injected
//! `now_ms`, so they unit-test deterministically with no clock; the `load`/`save`
//! I/O and `reconcile` (flag-missing + scan the managed dir for un-indexed
//! `.cutproj`s) do the filesystem work; the `note_*` wrappers compose load → mutate
//! → save for the dispatch handlers.
//!
//! Callers: dispatch.rs — `project.create`/`open`/`rename` call the `note_*`
//! wrappers; `project.list`/`project.forget` call `list`/`forget`. Deps: serde,
//! sha2 (stable id from the canonical path), userdata.rs.

use crate::userdata;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static PROJECTS_INDEX_IO_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
const MAX_PROJECT_NAME_METADATA_BYTES: u64 = 16 * 1024 * 1024;
const LOCK_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
const LOCK_RETRY_PAUSE: Duration = Duration::from_millis(5);
const LOCK_BUSY_MESSAGE: &str =
    "Cut could not save your project list because another ShellX Cut is saving it. Try again.";

fn projects_index_io_lock() -> &'static Mutex<()> {
    PROJECTS_INDEX_IO_LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug)]
struct IndexLockBusy;

impl std::fmt::Display for IndexLockBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(LOCK_BUSY_MESSAGE)
    }
}

impl std::error::Error for IndexLockBusy {}

fn lock_path_for(index_path: &Path) -> PathBuf {
    let mut os = index_path.as_os_str().to_os_string();
    os.push(".lock");
    PathBuf::from(os)
}

fn is_lock_contended(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock || (cfg!(windows) && error.raw_os_error() == Some(33))
}

struct IndexLock {
    file: std::fs::File,
}

impl Drop for IndexLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

impl IndexLock {
    fn acquire(index_path: &Path) -> io::Result<Self> {
        Self::acquire_within(index_path, LOCK_WAIT_TIMEOUT)
    }

    fn acquire_within(index_path: &Path, timeout: Duration) -> io::Result<Self> {
        use fs2::FileExt;

        let parent = index_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let lock_path = lock_path_for(index_path);
        if cut_core::store::plain_file_state(&lock_path)
            == cut_core::store::PlainFileState::NotPlain
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is a link or not an ordinary file", lock_path.display()),
            ));
        }

        let mut options = std::fs::OpenOptions::new();
        options.create(true).read(true).write(true).truncate(false);
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(&lock_path)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not an ordinary file", lock_path.display()),
            ));
        }

        let deadline = Instant::now() + timeout;
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(Self { file }),
                Err(error) if is_lock_contended(&error) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(io::Error::new(io::ErrorKind::TimedOut, IndexLockBusy));
                    }
                    std::thread::sleep(LOCK_RETRY_PAUSE.min(deadline - now));
                }
                Err(error) => return Err(error),
            }
        }
    }
}

/// One tracked project. `path` is the absolute `<name>.cutproj` directory; `id` is a
/// stable opaque hash of that path (so re-creating the same dir re-uses the entry,
/// and ids never collide with caller-controlled strings). Optional `duration_ms`/
/// `clip_count`/`thumb` enrich the recent-grid when known (filled by the dispatch
/// `note_opened` path, which has typed access to the live project).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProjectEntry {
    pub id: String,
    pub name: String,
    pub path: String,
    pub created_ms: u64,
    pub last_opened_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_count: Option<u64>,
    /// Set by `reconcile` when the `.cutproj` no longer exists on disk (the entry is
    /// kept — the user may remount/restore — but flagged so the UI can grey it out).
    #[serde(default, skip_serializing_if = "is_false")]
    pub missing: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The on-disk index document. Versioned for forward migration.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProjectsIndex {
    pub version: u32,
    pub entries: Vec<ProjectEntry>,
}

impl Default for ProjectsIndex {
    fn default() -> Self {
        Self {
            version: 1,
            entries: Vec::new(),
        }
    }
}

/// Strip the Windows verbatim prefix (`\\?\`, or `\\?\UNC\` for network paths) so a
/// project path compares + displays cleanly and matches whether the caller passes
/// the canonical or the plain form. `std::fs::canonicalize` emits `\\?\C:\…` on
/// Windows, but `project.forget{path}` / `project.open{path}` are called with the
/// plain `C:\…` — without this they'd never match the stored entry. No-op elsewhere.
pub fn normalize_path(p: &str) -> String {
    if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = p.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        p.to_string()
    }
}

/// Stable opaque id for a project path: first 16 hex chars of sha256(normalized
/// path) — prefix-insensitive so the id is the same canonical-or-plain.
pub fn id_for(path: &str) -> String {
    let mut h = Sha256::new();
    h.update(normalize_path(path).as_bytes());
    let digest = h.finalize();
    hex16(&digest)
}

fn hex16(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(16);
    for b in bytes.iter().take(8) {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Current wall-clock in ms since epoch (0 on the impossible pre-epoch error).
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---- PURE manifest ops (unit-tested; no I/O, no clock) ---------------------

impl ProjectsIndex {
    /// Insert a new entry or update the existing one with the same id. Preserves the
    /// original `created_ms` on update (only metadata + last_opened move forward).
    pub fn upsert(&mut self, mut entry: ProjectEntry) {
        if let Some(existing) = self.entries.iter_mut().find(|e| e.id == entry.id) {
            entry.created_ms = existing.created_ms; // creation time is immutable
            *existing = entry;
        } else {
            self.entries.push(entry);
        }
    }

    /// Bump `last_opened_ms` (and clear `missing`) for an existing entry. Returns
    /// false if no entry has that id.
    pub fn touch(&mut self, id: &str, now_ms: u64) -> bool {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.last_opened_ms = now_ms;
            e.missing = false;
            true
        } else {
            false
        }
    }

    /// Update the display name of the entry for `path` (rename does not move the
    /// dir, so we key by path). Prefix-insensitive. Returns false if not found.
    pub fn rename_path(&mut self, path: &str, new_name: &str) -> bool {
        let np = normalize_path(path);
        if let Some(e) = self.entries.iter_mut().find(|e| e.path == np) {
            e.name = new_name.to_string();
            true
        } else {
            false
        }
    }

    /// Remove the entry matching `id` OR `path` (path match is prefix-insensitive).
    /// Returns true if one was removed. Does NOT delete the `.cutproj` (forget ≠ delete).
    pub fn remove(&mut self, id_or_path: &str) -> bool {
        let np = normalize_path(id_or_path);
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id_or_path && e.path != np);
        self.entries.len() != before
    }

    /// Filter + sort a snapshot for the UI. `q` matches the name case-insensitively;
    /// `sort` ∈ "recent" (default, last_opened desc) | "name" (asc) | "created" (desc).
    pub fn query(&self, sort: &str, q: Option<&str>) -> Vec<ProjectEntry> {
        let needle = q.map(|s| s.to_lowercase());
        let mut out: Vec<ProjectEntry> = self
            .entries
            .iter()
            .filter(|e| match &needle {
                Some(n) => e.name.to_lowercase().contains(n),
                None => true,
            })
            .cloned()
            .collect();
        match sort {
            "name" => out.sort_by_key(|a| a.name.to_lowercase()),
            "created" => out.sort_by_key(|a| std::cmp::Reverse(a.created_ms)),
            _ => out.sort_by_key(|a| std::cmp::Reverse(a.last_opened_ms)), // "recent"
        }
        out
    }
}

/// Tolerant migration from a raw JSON value (missing/old/junk → a valid index).
/// Unknown shapes degrade to an empty index rather than failing a session.
pub fn migrate(raw: serde_json::Value) -> ProjectsIndex {
    serde_json::from_value::<ProjectsIndex>(raw).unwrap_or_default()
}

/// Build an entry from its parts (id derived from the path).
pub fn make_entry(
    name: &str,
    path: &str,
    created_ms: u64,
    last_opened_ms: u64,
    duration_ms: Option<u64>,
    clip_count: Option<u64>,
) -> ProjectEntry {
    ProjectEntry {
        id: id_for(path),
        name: name.to_string(),
        path: normalize_path(path),
        created_ms,
        last_opened_ms,
        thumb: None,
        duration_ms,
        clip_count,
        missing: false,
    }
}

// ---- I/O + reconcile (filesystem) ------------------------------------------

/// Load the index (migrating on read). Missing file or no home → empty index.
/// A damaged file (empty, cut short, not a list) is read from the copy kept at
/// the last save; without a usable copy it is moved aside (never deleted) and
/// the list starts again from the projects folder, with a one-time notice. A
/// file that cannot be read at all returns an error and is left as it is.
pub fn try_load() -> io::Result<ProjectsIndex> {
    let Some(p) = userdata::projects_index_path() else {
        return Ok(ProjectsIndex::default());
    };
    try_load_readonly_or_repair(&p)
}

/// The copy of the list kept by every save beside the index. It is read only
/// when the index itself is damaged, so a damaged index never looks empty and
/// the next save replaces it with the kept list.
fn last_good_path(index_path: &Path) -> PathBuf {
    index_path.with_extension("last-good.json")
}

fn try_load_existing_or_kept(path: &Path) -> io::Result<ProjectsIndex> {
    let damaged = match try_load_at_with(path, crate::replace_temporary::read_optional_text) {
        Err(error) if error.kind() == io::ErrorKind::InvalidData => error,
        other => return other,
    };
    let kept = crate::replace_temporary::read_optional_text(&last_good_path(path))
        .ok()
        .flatten()
        .and_then(|text| parse_index(&text).ok());
    if let Some(idx) = kept {
        tracing::warn!(
            index = %path.display(),
            error = %damaged,
            "recent projects list was damaged; using the copy kept at the last save"
        );
        return Ok(idx);
    }
    Err(damaged)
}

fn try_load_readonly_or_repair(path: &Path) -> io::Result<ProjectsIndex> {
    match try_load_existing_or_kept(path) {
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            let lock = IndexLock::acquire(path)?;
            try_load_locked(&lock, path)
        }
        other => other,
    }
}

fn try_load_locked(_lock: &IndexLock, path: &Path) -> io::Result<ProjectsIndex> {
    let damaged = match try_load_existing_or_kept(path) {
        Err(error) if error.kind() == io::ErrorKind::InvalidData => error,
        other => return other,
    };
    let aside = set_damaged_aside(path)?;
    tracing::warn!(
        index = %path.display(),
        kept_as = %aside.display(),
        error = %damaged,
        "recent projects list was damaged and had no usable copy; moved aside and rebuilt"
    );
    let mut pending = rebuilt_notices().lock().expect("rebuilt notices lock");
    pending.retain(|notice| notice.path != path);
    pending.push(PendingRebuiltNotice {
        path: path.to_path_buf(),
        aside,
        agent_seen: false,
        ui_seen: false,
    });
    Ok(ProjectsIndex::default())
}

/// Move a damaged list to `projects.damaged.json` (or `-2`, `-3`, ...), never
/// over an earlier one, so nothing a person had is ever deleted.
fn set_damaged_aside(path: &Path) -> io::Result<PathBuf> {
    for n in 1..=1000u32 {
        let aside = if n == 1 {
            path.with_extension("damaged.json")
        } else {
            path.with_extension(format!("damaged-{n}.json"))
        };
        if std::fs::symlink_metadata(&aside).is_err() {
            std::fs::rename(path, &aside)?;
            return Ok(aside);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many damaged copies of the project list are kept beside it",
    ))
}

const REBUILT_CODE: &str = "project_list_rebuilt";
const REBUILT_MESSAGE: &str = "Your project list was damaged and has been rebuilt from your projects folder. Projects saved in other folders appear again when you open them.";

#[derive(Clone)]
struct PendingRebuiltNotice {
    path: PathBuf,
    aside: PathBuf,
    agent_seen: bool,
    ui_seen: bool,
}

type RebuiltNotices = Mutex<Vec<PendingRebuiltNotice>>;
static REBUILT_NOTICES: OnceLock<RebuiltNotices> = OnceLock::new();

fn rebuilt_notices() -> &'static RebuiltNotices {
    REBUILT_NOTICES.get_or_init(|| Mutex::new(Vec::new()))
}

/// True only for the live browser/webview window (`human:ui-<32hex>:ui`),
/// distinct from CLI/REST/MCP agents and external scripts.
pub(crate) fn is_ui_window_actor(actor: &cut_core::Actor) -> bool {
    actor.kind == cut_core::ActorKind::Human
        && actor.via == "ui"
        && actor
            .name
            .strip_prefix("ui-")
            .is_some_and(|hex| hex.len() == 32 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// The rebuilt-list notice for `actor`, delivered once to the first agent/script
/// call and once independently to the UI window so automation never steals it
/// before the window mounts.
pub fn rebuilt_notice_for(actor: &cut_core::Actor) -> Option<cut_core::VerbWarning> {
    rebuilt_notice_for_at(&userdata::projects_index_path()?, actor)
}

#[cfg(test)]
fn take_rebuilt_notice_at(index_path: &Path) -> Option<cut_core::VerbWarning> {
    rebuilt_notice_for_at(index_path, &cut_core::Actor::system())
}

fn rebuilt_notice_for_at(
    index_path: &Path,
    actor: &cut_core::Actor,
) -> Option<cut_core::VerbWarning> {
    let mut pending = rebuilt_notices().lock().expect("rebuilt notices lock");
    let at = pending.iter().position(|entry| entry.path == index_path)?;
    let is_ui = is_ui_window_actor(actor);
    let aside = {
        let entry = &mut pending[at];
        if is_ui {
            if entry.ui_seen {
                return None;
            }
            entry.ui_seen = true;
        } else {
            if entry.agent_seen {
                return None;
            }
            entry.agent_seen = true;
        }
        entry.aside.clone()
    };
    pending.retain(|entry| entry.path != index_path || !entry.agent_seen || !entry.ui_seen);
    let mut detail = serde_json::Map::new();
    detail.insert(
        "damaged_copy".into(),
        serde_json::Value::String(aside.to_string_lossy().into_owned()),
    );
    detail.insert(
        "suggested_action".into(),
        serde_json::Value::String(
            "open a project saved outside the projects folder by its path (project.open) to list it again"
                .into(),
        ),
    );
    Some(cut_core::VerbWarning {
        code: REBUILT_CODE.into(),
        message: REBUILT_MESSAGE.into(),
        detail,
    })
}

/// Attach any pending rebuilt-list notice for `actor` without disturbing existing warnings.
pub fn with_rebuilt_notice_for(
    result: cut_core::VerbResult,
    actor: &cut_core::Actor,
) -> cut_core::VerbResult {
    result.with_warnings(rebuilt_notice_for(actor).into_iter().collect())
}

/// A project list that cannot be read or saved (locked, no permission, a
/// folder in its place) as a plain refusal with the way forward.
pub fn refusal(error: io::Error) -> cut_core::CutError {
    let index = userdata::projects_index_path();
    if error
        .get_ref()
        .is_some_and(|inner| inner.is::<IndexLockBusy>())
    {
        let lock = index
            .as_deref()
            .map(lock_path_for)
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| "projects.json.lock".into());
        return cut_core::CutError::new(
            cut_core::error_codes::IO,
            LOCK_BUSY_MESSAGE,
            format!("{lock}: another ShellX Cut process held the project list lock"),
        )
        .with_suggested_action(
            "Wait for the other ShellX Cut to finish saving your project list, then try again.",
        );
    }
    let file = index
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "projects.json".into());
    cut_core::CutError::new(
        cut_core::error_codes::IO,
        "Cut could not read or save your project list.",
        format!("{file}: {error}"),
    )
    .with_suggested_action(format!(
        "Make sure {file} is a file you may change and is not held open by another program, then try again."
    ))
}

fn try_load_at_with(
    path: &Path,
    read: impl FnOnce(&Path) -> io::Result<Option<String>>,
) -> io::Result<ProjectsIndex> {
    let Some(text) = read(path)? else {
        return Ok(ProjectsIndex::default());
    };
    parse_index(&text)
}

fn parse_index(text: &str) -> io::Result<ProjectsIndex> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("saved projects index is damaged: {error}"),
        )
    })?;
    if !value.is_object() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "saved projects index is damaged: expected a JSON object",
        ));
    }
    Ok(migrate(value))
}

/// Read a test assertion's snapshot under the same lock as dispatch writers.
/// Raw unserialized reads can otherwise race a writer's replacement window.
#[cfg(test)]
pub(crate) fn load_for_test() -> ProjectsIndex {
    let _guard = projects_index_io_lock()
        .lock()
        .expect("projects index lock");
    try_load().expect("load projects index for test")
}

#[cfg(test)]
pub(crate) fn save_at(path: &Path, idx: &ProjectsIndex) -> io::Result<()> {
    save_before_replace(path, idx, || Ok(()))
}

fn save_locked(_lock: &IndexLock, path: &Path, idx: &ProjectsIndex) -> io::Result<()> {
    save_before_replace(path, idx, || Ok(()))
}

fn save_before_replace(
    path: &Path,
    idx: &ProjectsIndex,
    before_replace: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let body = serde_json::to_vec_pretty(idx)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    // The kept copy is written first: a save that fails here leaves the index
    // untouched, and a stop between the two leaves both files readable.
    write_replacing(parent, &last_good_path(path), &body, || Ok(()))?;
    write_replacing(parent, path, &body, before_replace)
}

fn write_replacing(
    parent: &Path,
    path: &Path,
    body: &[u8],
    before_replace: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let mut file = tempfile::Builder::new()
        .prefix(".projects-index-")
        .tempfile_in(parent)?;
    file.write_all(body)?;
    file.as_file().sync_all()?;
    before_replace()?;
    // tempfile uses a same-directory rename on Unix and MoveFileExW with
    // replacement on Windows. Never truncate or remove the previous index
    // before replacement so concurrent readers never observe an empty list.
    crate::replace_temporary::replace_temporary(file, path)?;
    #[cfg(unix)]
    std::fs::File::open(parent).and_then(|directory| directory.sync_all())?;
    Ok(())
}

/// A `.cutproj` is "live" if it still has its log or cache on disk.
fn cutproj_exists(path: &str) -> bool {
    let dir = Path::new(path);
    dir.join("ops.jsonl").exists() || dir.join("project.json").exists()
}

/// Read a project's display name from its `project.json` (falls back to the dir
/// stem). Best-effort; never fails the scan.
fn read_project_name(dir: &Path) -> String {
    let pj = dir.join("project.json");
    // Discovery runs before a project is opened. Bound both a known large file
    // and one that grows after the metadata check, then use the usual fallback.
    let name = (|| -> io::Result<Option<String>> {
        // A FIFO could block in File::open before a byte limit can apply.
        let entry = std::fs::symlink_metadata(&pj)?;
        if !entry.file_type().is_file() || entry.len() > MAX_PROJECT_NAME_METADATA_BYTES {
            return Ok(None);
        }
        let file = std::fs::File::open(&pj)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > MAX_PROJECT_NAME_METADATA_BYTES {
            return Ok(None);
        }
        let mut contents = String::new();
        file.take(MAX_PROJECT_NAME_METADATA_BYTES + 1)
            .read_to_string(&mut contents)?;
        if contents.len() as u64 > MAX_PROJECT_NAME_METADATA_BYTES {
            return Ok(None);
        }
        let value: serde_json::Value = match serde_json::from_str(&contents) {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };
        Ok(value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .filter(|name| !name.is_empty())
            .map(str::to_owned))
    })();
    if let Ok(Some(name)) = name {
        return name;
    }
    dir.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_string()
}

/// File mtime of `ops.jsonl` in ms (a reasonable "last touched" for a scanned
/// project we never saw opened); `fallback` when unavailable.
fn ops_mtime_ms(dir: &Path, fallback: u64) -> u64 {
    std::fs::metadata(dir.join("ops.jsonl"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(fallback)
}

/// Reconcile the index against the filesystem: (1) flag entries whose `.cutproj`
/// vanished, (2) scan `managed_dir` for `.cutproj`s not yet indexed and add them
/// (so projects created before this feature, or by another tool, still appear).
pub fn reconcile(idx: &mut ProjectsIndex, managed_dir: &Path, now_ms: u64) {
    for e in &mut idx.entries {
        e.missing = !cutproj_exists(&e.path);
    }
    let Ok(rd) = std::fs::read_dir(managed_dir) else {
        return;
    };
    for ent in rd.flatten() {
        let p = ent.path();
        if !p.is_dir() || p.extension().and_then(|s| s.to_str()) != Some("cutproj") {
            continue;
        }
        let abs = p.canonicalize().unwrap_or(p.clone());
        let path_s = normalize_path(&abs.to_string_lossy());
        if idx.entries.iter().any(|e| e.path == path_s) {
            continue; // already tracked
        }
        if !cutproj_exists(&path_s) {
            continue; // a stray dir, not a real project
        }
        let name = read_project_name(&abs);
        let mtime = ops_mtime_ms(&abs, now_ms);
        idx.entries
            .push(make_entry(&name, &path_s, mtime, mtime, None, None));
    }
}

// ---- note_* wrappers (load → mutate → save) for the dispatch handlers -------

/// Record a freshly created project (or refresh an existing entry for the path).
pub fn note_created(name: &str, path: &str) -> io::Result<()> {
    let _guard = projects_index_io_lock()
        .lock()
        .expect("projects index lock");
    let Some(index_path) = userdata::projects_index_path() else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no HOME/USERPROFILE for ~/.shellx-cut/projects.json",
        ));
    };
    note_created_at(&index_path, name, path)
}

fn note_created_at(index_path: &Path, name: &str, path: &str) -> io::Result<()> {
    note_created_at_within(index_path, name, path, LOCK_WAIT_TIMEOUT)
}

fn note_created_at_within(
    index_path: &Path,
    name: &str,
    path: &str,
    timeout: Duration,
) -> io::Result<()> {
    let lock = IndexLock::acquire_within(index_path, timeout)?;
    let n = now_ms();
    let mut idx = try_load_locked(&lock, index_path)?;
    idx.upsert(make_entry(name, path, n, n, None, None));
    save_locked(&lock, index_path, &idx)
}

/// Record a project being opened: touch its `last_opened_ms`, or register it if it
/// is not yet indexed (externally-created project opened by path). Enriches
/// duration/clip_count when the caller knows them.
pub fn note_opened(
    name: &str,
    path: &str,
    duration_ms: Option<u64>,
    clip_count: Option<u64>,
) -> io::Result<()> {
    let _guard = projects_index_io_lock()
        .lock()
        .expect("projects index lock");
    let Some(index_path) = userdata::projects_index_path() else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no HOME/USERPROFILE for ~/.shellx-cut/projects.json",
        ));
    };
    note_opened_at(&index_path, name, path, duration_ms, clip_count)
}

fn note_opened_at(
    index_path: &Path,
    name: &str,
    path: &str,
    duration_ms: Option<u64>,
    clip_count: Option<u64>,
) -> io::Result<()> {
    note_opened_at_within(
        index_path,
        name,
        path,
        duration_ms,
        clip_count,
        LOCK_WAIT_TIMEOUT,
    )
}

fn note_opened_at_within(
    index_path: &Path,
    name: &str,
    path: &str,
    duration_ms: Option<u64>,
    clip_count: Option<u64>,
    timeout: Duration,
) -> io::Result<()> {
    let lock = IndexLock::acquire_within(index_path, timeout)?;
    let n = now_ms();
    let mut idx = try_load_locked(&lock, index_path)?;
    let id = id_for(path);
    if idx.touch(&id, n) {
        // refresh metadata on the existing entry
        if let Some(e) = idx.entries.iter_mut().find(|e| e.id == id) {
            e.name = name.to_string();
            if duration_ms.is_some() {
                e.duration_ms = duration_ms;
            }
            if clip_count.is_some() {
                e.clip_count = clip_count;
            }
        }
    } else {
        idx.upsert(make_entry(name, path, n, n, duration_ms, clip_count));
    }
    save_locked(&lock, index_path, &idx)
}

/// Record a rename (display name only; the dir is unchanged so we key by path).
pub fn note_renamed(path: &str, new_name: &str) -> io::Result<bool> {
    let _guard = projects_index_io_lock()
        .lock()
        .expect("projects index lock");
    let Some(index_path) = userdata::projects_index_path() else {
        return Ok(false);
    };
    note_renamed_at(&index_path, path, new_name)
}

fn note_renamed_at(index_path: &Path, path: &str, new_name: &str) -> io::Result<bool> {
    note_renamed_at_within(index_path, path, new_name, LOCK_WAIT_TIMEOUT)
}

fn note_renamed_at_within(
    index_path: &Path,
    path: &str,
    new_name: &str,
    timeout: Duration,
) -> io::Result<bool> {
    let lock = IndexLock::acquire_within(index_path, timeout)?;
    let mut idx = try_load_locked(&lock, index_path)?;
    if idx.rename_path(path, new_name) {
        save_locked(&lock, index_path, &idx)?;
        return Ok(true);
    }
    Ok(false)
}

/// The `project.list` read: load → reconcile against `managed_dir` → query → save
/// (persist any newly-scanned entries + missing flags) → return the sorted view.
pub fn list(managed_dir: &Path, sort: &str, q: Option<&str>) -> io::Result<Vec<ProjectEntry>> {
    let _guard = projects_index_io_lock()
        .lock()
        .expect("projects index lock");
    let Some(index_path) = userdata::projects_index_path() else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no HOME/USERPROFILE for ~/.shellx-cut/projects.json",
        ));
    };
    list_at(&index_path, managed_dir, sort, q)
}

fn list_at(
    index_path: &Path,
    managed_dir: &Path,
    sort: &str,
    q: Option<&str>,
) -> io::Result<Vec<ProjectEntry>> {
    list_at_within(index_path, managed_dir, sort, q, LOCK_WAIT_TIMEOUT)
}

fn list_at_within(
    index_path: &Path,
    managed_dir: &Path,
    sort: &str,
    q: Option<&str>,
    timeout: Duration,
) -> io::Result<Vec<ProjectEntry>> {
    let lock = IndexLock::acquire_within(index_path, timeout)?;
    let n = now_ms();
    let mut idx = try_load_locked(&lock, index_path)?;
    reconcile(&mut idx, managed_dir, n);
    save_locked(&lock, index_path, &idx)?;
    Ok(idx.query(sort, q))
}

/// The `project.forget` write: drop the entry (does NOT delete the `.cutproj`).
pub fn forget(id_or_path: &str) -> io::Result<bool> {
    let _guard = projects_index_io_lock()
        .lock()
        .expect("projects index lock");
    let Some(index_path) = userdata::projects_index_path() else {
        return Ok(false);
    };
    forget_at(&index_path, id_or_path)
}

fn forget_at(index_path: &Path, id_or_path: &str) -> io::Result<bool> {
    forget_at_within(index_path, id_or_path, LOCK_WAIT_TIMEOUT)
}

fn forget_at_within(index_path: &Path, id_or_path: &str, timeout: Duration) -> io::Result<bool> {
    let lock = IndexLock::acquire_within(index_path, timeout)?;
    let mut idx = try_load_locked(&lock, index_path)?;
    let removed = idx.remove(id_or_path);
    if removed {
        save_locked(&lock, index_path, &idx)?;
    }
    Ok(removed)
}

/// The `project.forget{missing:true}` write: drop EVERY entry whose `.cutproj`
/// does not exist on disk RIGHT NOW (a fresh `cutproj_exists` stat per entry —
/// never the persisted `missing` flag, which reflects the last reconcile: a
/// project on a remounted drive must survive this sweep even if it was flagged
/// missing while unmounted). Does NOT delete anything on disk (forget ≠ delete).
/// Returns how many entries were dropped. This is the bulk-hygiene path for
/// registries containing deleted test or scratch projects.
pub fn forget_missing() -> io::Result<usize> {
    let _guard = projects_index_io_lock()
        .lock()
        .expect("projects index lock");
    let Some(index_path) = userdata::projects_index_path() else {
        return Ok(0);
    };
    forget_missing_at(&index_path)
}

fn forget_missing_at(index_path: &Path) -> io::Result<usize> {
    forget_missing_at_within(index_path, LOCK_WAIT_TIMEOUT)
}

fn forget_missing_at_within(index_path: &Path, timeout: Duration) -> io::Result<usize> {
    let lock = IndexLock::acquire_within(index_path, timeout)?;
    let mut idx = try_load_locked(&lock, index_path)?;
    let before = idx.entries.len();
    idx.entries.retain(|e| cutproj_exists(&e.path));
    let removed = before - idx.entries.len();
    if removed > 0 {
        save_locked(&lock, index_path, &idx)?;
    }
    Ok(removed)
}

/// Collect the ids of every index entry that refers to the same on-disk directory
/// as `canon`, comparing by CANONICALIZED path so a stale "missing" ghost can never
/// linger after `project.delete` because of a path-form mismatch (Windows
/// casing/8.3 short names / the `\\?\` verbatim prefix, or symlink/`..` differences
/// between the stored creation path and the canonicalized delete path). Entries
/// whose stored path no longer canonicalizes (already gone) fall back to a
/// normalized-string comparison against `canon`. MUST be called while `canon` still
/// exists on disk — `project.delete` captures these ids BEFORE removing the files,
/// then forgets them with `forget_ids` afterwards (post-delete the path won't
/// resolve, so the canonicalize comparison would no longer match).
pub fn ids_for_dir(canon: &Path) -> io::Result<Vec<String>> {
    let _guard = projects_index_io_lock()
        .lock()
        .expect("projects index lock");
    let canon_norm = normalize_path(&canon.to_string_lossy());
    let idx = try_load()?;
    Ok(idx
        .entries
        .iter()
        .filter(|e| {
            // Best: both paths canonicalize to the same real directory.
            if let Ok(ep) = Path::new(&e.path).canonicalize() {
                if ep == canon {
                    return true;
                }
            }
            // Fallback (entry path already gone, or canonicalize unsupported):
            // normalized-string equality, prefix-insensitive on both sides.
            normalize_path(&e.path) == canon_norm
        })
        .map(|e| e.id.clone())
        .collect())
}

/// Drop every entry whose id is in `ids` (used by `project.delete` after the files
/// are removed). Returns how many were dropped. Saves only if something changed.
pub fn forget_ids(ids: &[String]) -> io::Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let _guard = projects_index_io_lock()
        .lock()
        .expect("projects index lock");
    let Some(index_path) = userdata::projects_index_path() else {
        return Ok(0);
    };
    forget_ids_at(&index_path, ids)
}

fn forget_ids_at(index_path: &Path, ids: &[String]) -> io::Result<usize> {
    forget_ids_at_within(index_path, ids, LOCK_WAIT_TIMEOUT)
}

fn forget_ids_at_within(index_path: &Path, ids: &[String], timeout: Duration) -> io::Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let lock = IndexLock::acquire_within(index_path, timeout)?;
    let mut idx = try_load_locked(&lock, index_path)?;
    let before = idx.entries.len();
    idx.entries.retain(|e| !ids.iter().any(|id| id == &e.id));
    let removed = before - idx.entries.len();
    if removed > 0 {
        save_locked(&lock, index_path, &idx)?;
    }
    Ok(removed)
}

/// Resolve a project's `.cutproj` path from its index id OR a path string (matches
/// by id, then by normalized path). Returns None when no index entry matches.
/// Used by `project.delete` to find the directory to remove from an id.
pub fn path_for(id_or_path: &str) -> io::Result<Option<String>> {
    let _guard = projects_index_io_lock()
        .lock()
        .expect("projects index lock");
    let idx = try_load()?;
    let key_norm = normalize_path(id_or_path);
    Ok(idx
        .entries
        .iter()
        .find(|e| e.id == id_or_path || normalize_path(&e.path) == key_norm)
        .map(|e| e.path.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn entry(name: &str, path: &str, created: u64, opened: u64) -> ProjectEntry {
        make_entry(name, path, created, opened, None, None)
    }

    #[test]
    fn discovery_bounds_project_metadata_and_keeps_projects_listed() {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let ordinary = root_path.join("ordinary.cutproj");
        let oversized = root_path.join("oversized.cutproj");
        let malformed = root_path.join("malformed.cutproj");
        for dir in [&ordinary, &oversized, &malformed] {
            std::fs::create_dir(dir).unwrap();
        }
        std::fs::write(ordinary.join("project.json"), r#"{"name":"Readable name"}"#).unwrap();
        std::fs::write(malformed.join("project.json"), b"{").unwrap();

        // Trailing JSON whitespace keeps this valid: the old unbounded reader
        // would return the supplied name after reading the whole large file.
        let mut file = std::fs::File::create(oversized.join("project.json")).unwrap();
        file.write_all(br#"{"name":"untrusted name"}"#).unwrap();
        let spaces = [b' '; 64 * 1024];
        for _ in 0..(MAX_PROJECT_NAME_METADATA_BYTES / spaces.len() as u64) {
            file.write_all(&spaces).unwrap();
        }
        drop(file);

        let mut index = ProjectsIndex::default();
        reconcile(&mut index, &root_path, 1);
        assert_eq!(index.entries.len(), 3);
        let name_for = |dir: &Path| {
            index
                .entries
                .iter()
                .find(|entry| {
                    Path::new(&entry.path).canonicalize().unwrap() == dir.canonicalize().unwrap()
                })
                .unwrap()
                .name
                .as_str()
        };
        assert_eq!(name_for(&ordinary), "Readable name");
        assert_eq!(name_for(&oversized), "oversized");
        assert_eq!(name_for(&malformed), "malformed");
    }

    #[cfg(unix)]
    #[test]
    fn discovery_does_not_follow_linked_project_metadata() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("linked.cutproj");
        std::fs::create_dir(&project).unwrap();
        let outside = root.path().join("outside.json");
        std::fs::write(&outside, r#"{"name":"outside name"}"#).unwrap();
        std::os::unix::fs::symlink(&outside, project.join("project.json")).unwrap();

        assert_eq!(read_project_name(&project), "linked");
    }

    #[test]
    fn upsert_inserts_then_updates_preserving_created() {
        let mut idx = ProjectsIndex::default();
        idx.upsert(entry("A", "/p/A.cutproj", 100, 100));
        assert_eq!(idx.entries.len(), 1);
        // same path → same id → update, but created_ms stays 100
        idx.upsert(entry("A renamed", "/p/A.cutproj", 999, 500));
        assert_eq!(idx.entries.len(), 1);
        assert_eq!(idx.entries[0].name, "A renamed");
        assert_eq!(idx.entries[0].created_ms, 100);
        assert_eq!(idx.entries[0].last_opened_ms, 500);
    }

    #[test]
    fn touch_bumps_last_opened_and_clears_missing() {
        let mut idx = ProjectsIndex::default();
        let mut e = entry("A", "/p/A.cutproj", 1, 1);
        e.missing = true;
        let id = e.id.clone();
        idx.upsert(e);
        assert!(idx.touch(&id, 42));
        assert_eq!(idx.entries[0].last_opened_ms, 42);
        assert!(!idx.entries[0].missing);
        assert!(!idx.touch("nope", 99));
    }

    #[test]
    fn rename_path_updates_name() {
        let mut idx = ProjectsIndex::default();
        idx.upsert(entry("Old", "/p/A.cutproj", 1, 1));
        assert!(idx.rename_path("/p/A.cutproj", "New"));
        assert_eq!(idx.entries[0].name, "New");
        assert!(!idx.rename_path("/p/missing.cutproj", "X"));
    }

    #[test]
    fn remove_by_id_or_path() {
        let mut idx = ProjectsIndex::default();
        let e = entry("A", "/p/A.cutproj", 1, 1);
        let id = e.id.clone();
        idx.upsert(e);
        idx.upsert(entry("B", "/p/B.cutproj", 1, 1));
        assert!(idx.remove(&id)); // by id
        assert!(idx.remove("/p/B.cutproj")); // by path
        assert!(idx.entries.is_empty());
        assert!(!idx.remove("/p/none"));
    }

    #[test]
    fn query_filters_and_sorts() {
        let mut idx = ProjectsIndex::default();
        idx.upsert(entry("Alpha", "/p/Alpha.cutproj", 10, 30));
        idx.upsert(entry("Beta", "/p/Beta.cutproj", 20, 10));
        idx.upsert(entry("Gamma", "/p/Gamma.cutproj", 30, 20));
        // recent = last_opened desc
        let recent = idx.query("recent", None);
        assert_eq!(
            recent.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["Alpha", "Gamma", "Beta"]
        );
        // name asc
        let by_name = idx.query("name", None);
        assert_eq!(by_name[0].name, "Alpha");
        assert_eq!(by_name[2].name, "Gamma");
        // created desc
        let by_created = idx.query("created", None);
        assert_eq!(by_created[0].name, "Gamma");
        // filter
        let filtered = idx.query("recent", Some("bet"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "Beta");
    }

    #[test]
    fn migrate_tolerates_junk_and_missing() {
        assert_eq!(migrate(serde_json::json!(null)).entries.len(), 0);
        assert_eq!(
            migrate(serde_json::json!({"garbage": true})).entries.len(),
            0
        );
        let good = migrate(serde_json::json!({
            "version": 1,
            "entries": [{"id":"x","name":"A","path":"/p/A.cutproj","created_ms":1,"last_opened_ms":2}]
        }));
        assert_eq!(good.entries.len(), 1);
        assert_eq!(good.entries[0].name, "A");
        assert!(!good.entries[0].missing); // default
    }

    #[test]
    fn id_is_stable_and_path_derived() {
        assert_eq!(id_for("/p/A.cutproj"), id_for("/p/A.cutproj"));
        assert_ne!(id_for("/p/A.cutproj"), id_for("/p/B.cutproj"));
        assert_eq!(id_for("/p/A.cutproj").len(), 16);
    }

    #[test]
    fn forget_ids_drops_only_named_entries() {
        let mut idx = ProjectsIndex::default();
        let a = entry("A", "/p/A.cutproj", 1, 1);
        let b = entry("B", "/p/B.cutproj", 1, 1);
        let (ida, idb) = (a.id.clone(), b.id.clone());
        idx.upsert(a);
        idx.upsert(b);
        // pure retain mirror of forget_ids (no I/O): drop only A
        let ids = [ida.clone()];
        idx.entries.retain(|e| !ids.iter().any(|id| id == &e.id));
        assert_eq!(idx.entries.len(), 1);
        assert_eq!(idx.entries[0].id, idb);
    }

    #[test]
    fn ids_for_dir_string_fallback_matches_prefixed_canon() {
        // The canonicalize path can't run without real dirs in a unit test, so this
        // exercises the normalized-string fallback: a stored PLAIN Windows path must
        // match a `\\?\`-verbatim canon (the exact mismatch that left ghosts).
        let stored = make_entry("W", r"C:\u\w.cutproj", 1, 1, None, None);
        let canon_norm = normalize_path(r"\\?\C:\u\w.cutproj");
        assert_eq!(normalize_path(&stored.path), canon_norm);
        // and a different dir must NOT match
        let other = make_entry("X", r"C:\u\x.cutproj", 1, 1, None, None);
        assert_ne!(normalize_path(&other.path), canon_norm);
    }

    #[test]
    fn windows_verbatim_prefix_is_normalized() {
        // canonicalize() emits \\?\C:\… on Windows; the plain form must match it.
        assert_eq!(normalize_path(r"\\?\C:\u\w.cutproj"), r"C:\u\w.cutproj");
        assert_eq!(
            normalize_path(r"\\?\UNC\srv\share\p.cutproj"),
            r"\\srv\share\p.cutproj"
        );
        assert_eq!(normalize_path("/p/A.cutproj"), "/p/A.cutproj"); // no-op elsewhere
                                                                    // id + stored path are prefix-insensitive, so forget-by-plain-path matches.
        assert_eq!(id_for(r"\\?\C:\u\w.cutproj"), id_for(r"C:\u\w.cutproj"));
        let mut idx = ProjectsIndex::default();
        idx.upsert(make_entry("W", r"\\?\C:\u\w.cutproj", 1, 1, None, None));
        assert_eq!(idx.entries[0].path, r"C:\u\w.cutproj"); // stored normalized
        assert!(idx.remove(r"C:\u\w.cutproj")); // forget by the plain path matches
        assert!(idx.entries.is_empty());
    }

    #[test]
    fn concurrent_reader_never_observes_empty_or_partial_index_during_save() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("projects.json");
        let build_index = |tick: u64| {
            let mut idx = ProjectsIndex::default();
            for i in 0..300u64 {
                idx.upsert(entry(
                    &format!("Project {i} tick {tick}"),
                    &format!("/projects/p_{i:03}.cutproj"),
                    1_000 + i,
                    2_000 + tick + i,
                ));
            }
            idx
        };
        let initial = build_index(0);
        save_at(&path, &initial).unwrap();

        let done = Arc::new(AtomicBool::new(false));
        let path_reader_a = path.clone();
        let done_reader_a = Arc::clone(&done);
        let reader_a = std::thread::spawn(move || -> Result<usize, String> {
            let mut reads = 0usize;
            while !done_reader_a.load(Ordering::Acquire) {
                // The same bounded read the product uses: a short Windows
                // replacement gap is retried there, a half-written file is not.
                let raw = crate::replace_temporary::read_optional_text(&path_reader_a)
                    .map_err(|e| format!("read error at iteration {reads}: {e}"))?
                    .ok_or_else(|| format!("index missing at iteration {reads}"))?;
                let parsed: ProjectsIndex = serde_json::from_str(&raw).map_err(|e| {
                    format!(
                        "partial/empty JSON at iteration {reads} (len={}): {e}",
                        raw.len()
                    )
                })?;
                if parsed.entries.len() != 300 {
                    return Err(format!(
                        "unexpected entry count {} at iteration {reads}",
                        parsed.entries.len()
                    ));
                }
                reads += 1;
            }
            Ok(reads)
        });

        for tick in 1..=600u64 {
            save_at(&path, &build_index(tick)).unwrap();
        }
        done.store(true, Ordering::Release);
        let reads = reader_a.join().unwrap().unwrap();
        assert!(reads > 0, "reader should execute concurrent reads");
    }

    #[test]
    fn interrupted_save_preserves_previous_index_and_leaves_no_temp_files() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("projects.json");
        let mut before = ProjectsIndex::default();
        before.upsert(entry("Keep", "/p/Keep.cutproj", 10, 20));
        save_at(&path, &before).unwrap();
        let before_bytes = std::fs::read(&path).unwrap();

        let mut after = ProjectsIndex::default();
        after.upsert(entry("Replaced", "/p/Replaced.cutproj", 30, 40));
        let err = save_before_replace(&path, &after, || {
            Err(io::Error::other("simulated crash before replace"))
        });
        assert!(err.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before_bytes);
        let loaded: ProjectsIndex = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(loaded.entries, before.entries);
        let mut names: Vec<String> = std::fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["projects.json", "projects.last-good.json"]);
    }

    #[test]
    fn damaged_index_uses_the_last_good_copy_and_is_never_saved_as_empty() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("projects.json");
        let managed = temp.path().join("managed");
        std::fs::create_dir(&managed).unwrap();
        let kept_entries = || {
            let mut idx = ProjectsIndex::default();
            idx.upsert(entry("Keep One", "/p/Keep1.cutproj", 10, 20));
            idx.upsert(entry("Keep Two", "/p/Keep2.cutproj", 30, 40));
            idx
        };

        let damaged = b"{\"version\":1,\"entries\":[{\"id\":";
        for bad in [b"".as_slice(), damaged.as_slice(), b"null".as_slice()] {
            save_at(&path, &kept_entries()).unwrap();
            std::fs::write(&path, bad).unwrap();

            let listed = list_at(&path, &managed, "recent", None)
                .expect("a damaged list must open with the kept copy");
            assert_eq!(listed.len(), 2, "{bad:?}: {listed:?}");

            std::fs::write(&path, bad).unwrap();
            note_created_at(&path, "New", "/p/New.cutproj")
                .expect("creating a project must not fail on a damaged list");
            let saved: ProjectsIndex =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let mut names: Vec<&str> = saved.entries.iter().map(|e| e.name.as_str()).collect();
            names.sort();
            assert_eq!(names, ["Keep One", "Keep Two", "New"], "{bad:?}");
        }

        // A read that fails (not damage) still refuses and changes nothing.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(note_created_at(&path, "Other", "/p/Other.cutproj").is_err());
        assert!(path.is_dir());
    }

    #[test]
    fn damaged_index_without_a_kept_copy_is_set_aside_rebuilt_and_noticed_once() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("projects.json");
        let managed = temp.path().join("managed");
        let found = managed.join("Found.cutproj");
        std::fs::create_dir_all(&found).unwrap();
        std::fs::write(found.join("project.json"), r#"{"name":"Found"}"#).unwrap();

        let damaged = b"{\"version\":1,\"entries\":[{\"id\":";
        let cases: [(&[u8], &str); 3] = [
            (b"", "projects.damaged.json"),
            (damaged, "projects.damaged-2.json"),
            (b"null", "projects.damaged-3.json"),
        ];
        for (bad, aside) in cases {
            let _ = std::fs::remove_file(temp.path().join("projects.last-good.json"));
            std::fs::write(&path, bad).unwrap();
            note_created_at(&path, "New", "/p/New.cutproj")
                .expect("a damaged list without a kept copy must not block creating a project");
            assert_eq!(std::fs::read(temp.path().join(aside)).unwrap(), bad);
            let notice = take_rebuilt_notice_at(&path).expect("one notice after the rebuild");
            assert_eq!(notice.code, "project_list_rebuilt");
            assert!(notice.message.starts_with("Your project list was damaged"));
            assert!(
                take_rebuilt_notice_at(&path).is_none(),
                "the notice is given once"
            );

            let listed = list_at(&path, &managed, "recent", None).unwrap();
            let mut names: Vec<&str> = listed.iter().map(|e| e.name.as_str()).collect();
            names.sort();
            assert_eq!(names, ["Found", "New"], "{bad:?}");
        }
        assert!(take_rebuilt_notice_at(&path).is_none());
    }

    #[test]
    fn agent_project_list_does_not_steal_rebuilt_notice_from_ui_window() {
        const CHILD_ENV: &str = "CUT_TEST_REBUILT_NOTICE_CHILD";
        const TEST_NAME: &str =
            "projects_index::tests::agent_project_list_does_not_steal_rebuilt_notice_from_ui_window";
        if std::env::var_os(CHILD_ENV).is_none() {
            let temp = tempfile::tempdir().unwrap();
            let projects_dir = temp.path().join("Projects");
            std::fs::create_dir_all(&projects_dir).unwrap();
            std::fs::write(temp.path().join("projects.json"), b"{broken").unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .arg("--exact")
                .arg(TEST_NAME)
                .arg("--nocapture")
                .env(CHILD_ENV, "1")
                .env("SHELLX_CUT_HOME", temp.path())
                .env("SHELLX_CUT_PROJECTS_DIR", &projects_dir)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "child failed:\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let state = crate::state::AppState::new();
            let agent = cut_core::Actor {
                kind: cut_core::ActorKind::Agent,
                name: "mcp".into(),
                via: "mcp".into(),
                request: None,
            };
            let ui_window = cut_core::Actor {
                kind: cut_core::ActorKind::Human,
                name: "ui-0123456789abcdef0123456789abcdef".into(),
                via: "ui".into(),
                request: None,
            };

            let first_agent = crate::dispatch::dispatch(
                &state,
                "project.list",
                serde_json::json!({}),
                agent.clone(),
            )
            .await;
            assert!(first_agent.ok, "{first_agent:?}");
            assert!(
                first_agent
                    .warnings
                    .iter()
                    .flatten()
                    .any(|w| w.code == "project_list_rebuilt"),
                "agent receives rebuilt notice once: {first_agent:?}"
            );
            let second_agent =
                crate::dispatch::dispatch(&state, "project.list", serde_json::json!({}), agent)
                    .await;
            assert!(second_agent.ok, "{second_agent:?}");
            assert!(second_agent.warnings.as_deref().unwrap_or(&[]).is_empty());

            let first_ui = crate::dispatch::dispatch(
                &state,
                "project.list",
                serde_json::json!({}),
                ui_window.clone(),
            )
            .await;
            assert!(first_ui.ok, "{first_ui:?}");
            assert!(
                first_ui
                    .warnings
                    .iter()
                    .flatten()
                    .any(|w| w.code == "project_list_rebuilt"),
                "UI window must still receive project_list_rebuilt after agent call: {first_ui:?}"
            );
            let second_ui = crate::dispatch::dispatch(
                &state,
                "project.list",
                serde_json::json!({}),
                ui_window.clone(),
            )
            .await;
            assert!(second_ui.ok, "{second_ui:?}");
            assert!(second_ui.warnings.as_deref().unwrap_or(&[]).is_empty());

            // Verify project.open also carries the rebuilt notice for the UI window when an agent listed first.
            let projects_dir =
                std::path::PathBuf::from(std::env::var_os("SHELLX_CUT_PROJECTS_DIR").unwrap());
            let open_project =
                cut_core::ProjectStore::create(&projects_dir, "OpenCheck", None).unwrap();
            let open_path = open_project.dir.clone();
            drop(open_project);
            let cut_home = std::path::PathBuf::from(std::env::var_os("SHELLX_CUT_HOME").unwrap());
            let _ = std::fs::remove_file(cut_home.join("projects.last-good.json"));
            std::fs::write(cut_home.join("projects.json"), b"{broken-open").unwrap();
            let agent2 = cut_core::Actor {
                kind: cut_core::ActorKind::Agent,
                name: "cli".into(),
                via: "cli".into(),
                request: None,
            };
            let agent_open_pre = crate::dispatch::dispatch(
                &state,
                "project.list",
                serde_json::json!({}),
                agent2.clone(),
            )
            .await;
            assert!(
                agent_open_pre
                    .warnings
                    .iter()
                    .flatten()
                    .any(|w| w.code == "project_list_rebuilt"),
                "agent must still see rebuild before project.open: {agent_open_pre:?}"
            );
            let ui_open = crate::dispatch::dispatch(
                &state,
                "project.open",
                serde_json::json!({"path": open_path}),
                ui_window.clone(),
            )
            .await;
            assert!(ui_open.ok, "{ui_open:?}");
            assert!(
                ui_open
                    .warnings
                    .iter()
                    .flatten()
                    .any(|w| w.code == "project_list_rebuilt"),
                "UI window project.open must still receive project_list_rebuilt: {ui_open:?}"
            );

            // Verify project.state carries it for the UI window (not agents) when --project opened at start.
            let _ = std::fs::remove_file(cut_home.join("projects.last-good.json"));
            std::fs::write(cut_home.join("projects.json"), b"{broken-state").unwrap();
            let _ = crate::projects_index::list(&projects_dir, "recent", None).unwrap();
            let agent_state =
                crate::dispatch::dispatch(&state, "project.state", serde_json::json!({}), agent2)
                    .await;
            assert!(agent_state.ok, "{agent_state:?}");
            assert!(
                agent_state
                    .warnings
                    .iter()
                    .flatten()
                    .all(|w| w.code != "project_list_rebuilt"),
                "agent project.state must not receive/consume project_list_rebuilt: {agent_state:?}"
            );
            let ui_state = crate::dispatch::dispatch(
                &state,
                "project.state",
                serde_json::json!({}),
                ui_window,
            )
            .await;
            assert!(ui_state.ok, "{ui_state:?}");
            assert!(
                ui_state
                    .warnings
                    .iter()
                    .flatten()
                    .any(|w| w.code == "project_list_rebuilt"),
                "UI window project.state must still receive project_list_rebuilt: {ui_state:?}"
            );
        });
    }

    #[test]
    fn unreadable_index_refuses_mutations_and_is_left_as_it_is() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("projects.json");
        let managed = temp.path().join("managed");
        std::fs::create_dir(&managed).unwrap();
        save_at(&path, &ProjectsIndex::default()).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(note_created_at(&path, "New", "/p/New.cutproj").is_err());
        assert!(note_opened_at(&path, "New", "/p/New.cutproj", None, None).is_err());
        assert!(note_renamed_at(&path, "/p/Keep.cutproj", "Renamed").is_err());
        assert!(list_at(&path, &managed, "recent", None).is_err());
        assert!(forget_at(&path, "/p/Keep.cutproj").is_err());
        assert!(forget_missing_at(&path).is_err());
        assert!(forget_ids_at(&path, &["id1".to_string()]).is_err());
        assert!(path.is_dir());
        assert!(take_rebuilt_notice_at(&path).is_none());
        let refused = refusal(io::Error::from(io::ErrorKind::PermissionDenied));
        assert_eq!(
            refused.message,
            "Cut could not read or save your project list."
        );
        assert!(refused.suggested_action.is_some());
    }

    #[test]
    fn windows_transient_index_errors_retry_and_exhausted_refusals_do_not_wipe() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("projects.json");
        let mut initial = ProjectsIndex::default();
        initial.upsert(entry("Keep", "/p/Keep.cutproj", 10, 20));
        save_at(&path, &initial).unwrap();
        let serialized = std::fs::read_to_string(&path).unwrap();

        for make_err in [
            || io::Error::new(io::ErrorKind::NotFound, "replacement gap"),
            || io::Error::from_raw_os_error(5),
            || io::Error::from_raw_os_error(32),
        ] {
            let mut attempts = 0;
            let loaded = try_load_at_with(&path, |p| {
                crate::replace_temporary::read_optional_text_with(
                    p,
                    true,
                    |_| {
                        attempts += 1;
                        if attempts <= 2 {
                            Err(make_err())
                        } else {
                            Ok(serialized.clone())
                        }
                    },
                    |_| {},
                )
            })
            .unwrap();
            assert_eq!(loaded.entries, initial.entries);
            assert_eq!(attempts, 3);
        }

        let refused = try_load_at_with(&path, |p| {
            crate::replace_temporary::read_optional_text_with(
                p,
                true,
                |_| Err(io::Error::from_raw_os_error(5)),
                |_| {},
            )
        });
        assert!(
            refused.is_err(),
            "exhausted access refusal must error instead of returning an empty index"
        );
    }

    #[cfg(windows)]
    #[test]
    fn transient_sharing_violation_retries_without_wiping_projects_index() {
        use std::os::windows::fs::OpenOptionsExt;
        use std::sync::{Arc, Barrier};

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("projects.json");
        let mut initial = ProjectsIndex::default();
        initial.upsert(entry("Keep One", "/p/Keep1.cutproj", 10, 20));
        initial.upsert(entry("Keep Two", "/p/Keep2.cutproj", 30, 40));
        save_at(&path, &initial).unwrap();

        let locked = Arc::new(Barrier::new(2));
        let holder = {
            let path = path.clone();
            let locked = Arc::clone(&locked);
            std::thread::spawn(move || {
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .share_mode(0)
                    .open(&path)
                    .unwrap();
                locked.wait();
                std::thread::sleep(std::time::Duration::from_millis(60));
                drop(file);
            })
        };

        locked.wait();
        note_created_at(&path, "New One", "/p/New1.cutproj")
            .expect("transient sharing violation must retry instead of failing or wiping");
        holder.join().unwrap();

        let saved: ProjectsIndex = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            saved.entries.len(),
            3,
            "existing entries were wiped: {saved:?}"
        );
    }

    struct TestChildGuard(Option<std::process::Child>);

    impl TestChildGuard {
        fn kill_and_wait(&mut self) {
            if let Some(mut child) = self.0.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    impl Drop for TestChildGuard {
        fn drop(&mut self) {
            self.kill_and_wait();
        }
    }

    fn wait_for_flag_or_child_exit(
        flag: &Path,
        children: &mut [std::process::Child],
        timeout: std::time::Duration,
    ) {
        let deadline = std::time::Instant::now() + timeout;
        while !flag.exists() {
            for child in children.iter_mut() {
                if let Some(status) = child.try_wait().unwrap() {
                    panic!(
                        "child exited early with status {status} before creating {}",
                        flag.display()
                    );
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {}",
                flag.display()
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn concurrent_processes_preserve_all_entries() {
        const WORKER_ENV: &str = "SHELLX_CUT_INDEX_CONCURRENT_WORKER";
        const BARRIER_ENV: &str = "SHELLX_CUT_INDEX_CONCURRENT_BARRIER";
        const TEST_NAME: &str = "projects_index::tests::concurrent_processes_preserve_all_entries";
        const PER_WORKER: u32 = 200;

        if let Some(worker) = std::env::var_os(WORKER_ENV) {
            let worker = worker.to_string_lossy().into_owned();
            let barrier = PathBuf::from(
                std::env::var_os(BARRIER_ENV).expect("barrier dir env must be set for worker"),
            );
            std::fs::write(barrier.join(format!("ready-{worker}")), b"1").unwrap();
            let go = barrier.join("go");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !go.exists() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "worker {worker} timed out waiting for go signal"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            // Each worker changes the list back to back, so on a disk with slow flushes (macOS full
            // fsync) the other worker can hold the lock past one bounded wait. The refusal says "try
            // again": do that, so the test still proves that no entry is ever lost.
            let mut retries = 0u32;
            for i in 0..PER_WORKER {
                let name = format!("Worker {worker} Project {i:03}");
                let path = format!("/projects/worker-{worker}/p-{i:03}.cutproj");
                loop {
                    match note_created(&name, &path) {
                        Ok(()) => break,
                        Err(error) if error.kind() == io::ErrorKind::TimedOut && retries < 120 => {
                            retries += 1;
                        }
                        Err(error) => panic!("worker {worker} entry {i}: {error:?}"),
                    }
                }
            }
            eprintln!("worker {worker}: {retries} bounded waits ran out and were retried");
            return;
        }

        let home = tempfile::tempdir().unwrap();
        let barrier = tempfile::tempdir().unwrap();
        let exe = std::env::current_exe().unwrap();
        let mut children: Vec<std::process::Child> = ["0", "1"]
            .into_iter()
            .map(|worker| {
                std::process::Command::new(&exe)
                    .args(["--exact", TEST_NAME, "--nocapture"])
                    .env("SHELLX_CUT_HOME", home.path())
                    .env(WORKER_ENV, worker)
                    .env(BARRIER_ENV, barrier.path())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .expect("spawn concurrent projects_index worker")
            })
            .collect();

        for worker in ["0", "1"] {
            wait_for_flag_or_child_exit(
                &barrier.path().join(format!("ready-{worker}")),
                &mut children,
                std::time::Duration::from_secs(15),
            );
        }
        std::fs::write(barrier.path().join("go"), b"go").unwrap();

        for (idx, child) in children.into_iter().enumerate() {
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "worker {idx} failed ({:?}):\nstdout:\n{}\nstderr:\n{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let index_path = home.path().join("projects.json");
        let saved: ProjectsIndex =
            serde_json::from_slice(&std::fs::read(&index_path).unwrap()).unwrap();
        let expected_total = (PER_WORKER * 2) as usize;
        assert_eq!(
            saved.entries.len(),
            expected_total,
            "lost {} of {} concurrent entries (got {})",
            expected_total.saturating_sub(saved.entries.len()),
            expected_total,
            saved.entries.len()
        );
        for worker in ["0", "1"] {
            for i in 0..PER_WORKER {
                let expected_path = format!("/projects/worker-{worker}/p-{i:03}.cutproj");
                assert!(
                    saved.entries.iter().any(|e| e.path == expected_path),
                    "missing entry {expected_path}"
                );
            }
        }
    }

    #[test]
    fn locked_index_refuses_change_after_timeout_without_modifying_file() {
        use fs2::FileExt;

        const HOLDER_DIR_ENV: &str = "SHELLX_CUT_INDEX_LOCK_HOLDER_DIR";
        const TEST_NAME: &str =
            "projects_index::tests::locked_index_refuses_change_after_timeout_without_modifying_file";
        const EXPECTED_MSG: &str =
            "Cut could not save your project list because another ShellX Cut is saving it. Try again.";

        if let Some(dir) = std::env::var_os(HOLDER_DIR_ENV) {
            let dir = PathBuf::from(dir);
            let lock_path = dir.join("projects.json.lock");
            let file = std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .open(&lock_path)
                .unwrap();
            file.lock_exclusive().unwrap();
            std::fs::write(dir.join("lock-held"), b"1").unwrap();
            std::thread::sleep(std::time::Duration::from_secs(30));
            let _ = FileExt::unlock(&file);
            return;
        }

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("projects.json");
        let mut initial = ProjectsIndex::default();
        initial.upsert(entry("Keep", "/p/Keep.cutproj", 10, 20));
        save_at(&path, &initial).unwrap();
        let before_bytes = std::fs::read(&path).unwrap();
        let before_kept_bytes = std::fs::read(last_good_path(&path)).unwrap();

        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(HOLDER_DIR_ENV, temp.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn lock holder child");
        let mut guard = TestChildGuard(Some(child));

        let held_flag = temp.path().join("lock-held");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !held_flag.exists() {
            if let Some(status) = guard.0.as_mut().unwrap().try_wait().unwrap() {
                panic!("lock holder exited early with {status}");
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for lock-held sentinel"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let started = std::time::Instant::now();
        let err = note_created_at(&path, "Blocked", "/p/Blocked.cutproj")
            .expect_err("held lock must refuse the change after bounded wait");
        let elapsed = started.elapsed();
        guard.kill_and_wait();

        assert!(
            elapsed >= std::time::Duration::from_secs(4)
                && elapsed < std::time::Duration::from_secs(15),
            "expected ~5s bounded wait, got {elapsed:?}"
        );
        assert_eq!(err.to_string(), EXPECTED_MSG);
        let cut_err = refusal(err);
        assert_eq!(cut_err.code, cut_core::error_codes::IO);
        assert_eq!(cut_err.message, EXPECTED_MSG);
        assert!(cut_err.suggested_action.is_some());
        assert_eq!(std::fs::read(&path).unwrap(), before_bytes);
        assert_eq!(
            std::fs::read(last_good_path(&path)).unwrap(),
            before_kept_bytes
        );
    }

    #[test]
    fn plain_reads_succeed_and_all_mutating_callers_refuse_while_lock_is_held() {
        use fs2::FileExt;

        const HOLDER_DIR_ENV: &str = "SHELLX_CUT_INDEX_ALL_LOCK_HOLDER_DIR";
        const TEST_NAME: &str =
            "projects_index::tests::plain_reads_succeed_and_all_mutating_callers_refuse_while_lock_is_held";

        if let Some(dir) = std::env::var_os(HOLDER_DIR_ENV) {
            let dir = PathBuf::from(dir);
            let file = std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .open(dir.join("projects.json.lock"))
                .unwrap();
            file.lock_exclusive().unwrap();
            std::fs::write(dir.join("lock-held"), b"1").unwrap();
            std::thread::sleep(std::time::Duration::from_secs(30));
            let _ = FileExt::unlock(&file);
            return;
        }

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("projects.json");
        let managed = temp.path().join("managed");
        std::fs::create_dir(&managed).unwrap();
        let mut initial = ProjectsIndex::default();
        let keep = entry("Keep", "/p/Keep.cutproj", 10, 20);
        let keep_id = keep.id.clone();
        initial.upsert(keep);
        save_at(&path, &initial).unwrap();
        let before_bytes = std::fs::read(&path).unwrap();
        let before_kept_bytes = std::fs::read(last_good_path(&path)).unwrap();

        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(HOLDER_DIR_ENV, temp.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn lock holder child");
        let mut guard = TestChildGuard(Some(child));

        let held_flag = temp.path().join("lock-held");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !held_flag.exists() {
            if let Some(status) = guard.0.as_mut().unwrap().try_wait().unwrap() {
                panic!("lock holder exited early with {status}");
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for lock-held sentinel"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        // Plain read succeeds immediately without acquiring the lock.
        let read_start = std::time::Instant::now();
        let loaded = try_load_readonly_or_repair(&path).expect("plain read must not block");
        assert!(read_start.elapsed() < std::time::Duration::from_secs(1));
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].id, keep_id);

        // Every mutating caller refuses cleanly without touching the files.
        let short = Duration::from_millis(25);
        let errors = [
            note_created_at_within(&path, "Blocked", "/p/Blocked.cutproj", short).unwrap_err(),
            note_opened_at_within(&path, "Keep", "/p/Keep.cutproj", None, None, short).unwrap_err(),
            note_renamed_at_within(&path, "/p/Keep.cutproj", "Renamed", short).unwrap_err(),
            list_at_within(&path, &managed, "recent", None, short).unwrap_err(),
            forget_at_within(&path, &keep_id, short).unwrap_err(),
            forget_missing_at_within(&path, short).unwrap_err(),
            forget_ids_at_within(&path, std::slice::from_ref(&keep_id), short).unwrap_err(),
        ];
        guard.kill_and_wait();

        for error in errors {
            assert_eq!(error.kind(), io::ErrorKind::TimedOut);
            assert_eq!(error.to_string(), LOCK_BUSY_MESSAGE);
            let cut_err = refusal(error);
            assert_eq!(cut_err.code, cut_core::error_codes::IO);
            assert_eq!(cut_err.message, LOCK_BUSY_MESSAGE);
            let recording_reply =
                crate::screen_record::created_project_rollback::running_take_failure_reply(
                    cut_err, "cap-lock", None,
                );
            let reply_err = recording_reply.error.unwrap();
            assert_eq!(reply_err.message, LOCK_BUSY_MESSAGE);
            assert!(reply_err.suggested_action.is_some());
        }
        assert_eq!(std::fs::read(&path).unwrap(), before_bytes);
        assert_eq!(
            std::fs::read(last_good_path(&path)).unwrap(),
            before_kept_bytes
        );
    }

    #[test]
    fn non_plain_lock_file_refuses_without_modifying_index() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("projects.json");
        let mut initial = ProjectsIndex::default();
        initial.upsert(entry("Keep", "/p/Keep.cutproj", 10, 20));
        save_at(&path, &initial).unwrap();
        let before_bytes = std::fs::read(&path).unwrap();

        let lock_path = lock_path_for(&path);
        std::fs::create_dir(&lock_path).unwrap();
        let err = note_created_at(&path, "Blocked", "/p/Blocked.cutproj").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(std::fs::read(&path).unwrap(), before_bytes);
        std::fs::remove_dir(&lock_path).unwrap();

        #[cfg(unix)]
        {
            let target = temp.path().join("outside-target.txt");
            std::fs::write(&target, b"untouched").unwrap();
            std::os::unix::fs::symlink(&target, &lock_path).unwrap();
            let err = note_created_at(&path, "Blocked", "/p/Blocked.cutproj").unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
            assert_eq!(std::fs::read(&path).unwrap(), before_bytes);
            assert_eq!(std::fs::read(&target).unwrap(), b"untouched");
        }
    }
}

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
use std::io;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static PROJECTS_INDEX_IO_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn projects_index_io_lock() -> &'static Mutex<()> {
    PROJECTS_INDEX_IO_LOCK.get_or_init(|| Mutex::new(()))
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
pub fn load() -> ProjectsIndex {
    let Some(p) = userdata::projects_index_path() else {
        return ProjectsIndex::default();
    };
    match std::fs::read_to_string(&p) {
        Ok(s) => match serde_json::from_str::<serde_json::Value>(&s) {
            Ok(v) => migrate(v),
            Err(_) => ProjectsIndex::default(),
        },
        Err(_) => ProjectsIndex::default(),
    }
}

/// Persist the index (mkdir -p the dotdir; pretty JSON).
pub fn save(idx: &ProjectsIndex) -> io::Result<()> {
    let Some(p) = userdata::projects_index_path() else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no HOME/USERPROFILE for ~/.shellx-cut/projects.json",
        ));
    };
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(idx).unwrap_or_else(|_| "{}".into());
    std::fs::write(&p, json)
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
    if let Ok(s) = std::fs::read_to_string(&pj) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
            if let Some(n) = v.get("name").and_then(|n| n.as_str()) {
                if !n.is_empty() {
                    return n.to_string();
                }
            }
        }
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
    let n = now_ms();
    let mut idx = load();
    idx.upsert(make_entry(name, path, n, n, None, None));
    save(&idx)
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
    let n = now_ms();
    let mut idx = load();
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
    save(&idx)
}

/// Record a rename (display name only; the dir is unchanged so we key by path).
pub fn note_renamed(path: &str, new_name: &str) -> io::Result<bool> {
    let _guard = projects_index_io_lock()
        .lock()
        .expect("projects index lock");
    let mut idx = load();
    if idx.rename_path(path, new_name) {
        save(&idx)?;
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
    let n = now_ms();
    let mut idx = load();
    reconcile(&mut idx, managed_dir, n);
    save(&idx)?;
    Ok(idx.query(sort, q))
}

/// The `project.forget` write: drop the entry (does NOT delete the `.cutproj`).
pub fn forget(id_or_path: &str) -> io::Result<bool> {
    let _guard = projects_index_io_lock()
        .lock()
        .expect("projects index lock");
    let mut idx = load();
    let removed = idx.remove(id_or_path);
    if removed {
        save(&idx)?;
    }
    Ok(removed)
}

/// The `project.forget{missing:true}` write: drop EVERY entry whose `.cutproj`
/// does not exist on disk RIGHT NOW (a fresh `cutproj_exists` stat per entry —
/// never the persisted `missing` flag, which reflects the last reconcile: a
/// project on a remounted drive must survive this sweep even if it was flagged
/// missing while unmounted). Does NOT delete anything on disk (forget ≠ delete).
/// Returns how many entries were dropped. This is the bulk-hygiene path for
/// registries polluted by deleted test/scratch projects (3.7k found.
pub fn forget_missing() -> io::Result<usize> {
    let _guard = projects_index_io_lock()
        .lock()
        .expect("projects index lock");
    let mut idx = load();
    let before = idx.entries.len();
    idx.entries.retain(|e| cutproj_exists(&e.path));
    let removed = before - idx.entries.len();
    if removed > 0 {
        save(&idx)?;
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
pub fn ids_for_dir(canon: &Path) -> Vec<String> {
    let _guard = projects_index_io_lock()
        .lock()
        .expect("projects index lock");
    let canon_norm = normalize_path(&canon.to_string_lossy());
    let idx = load();
    idx.entries
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
        .collect()
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
    let mut idx = load();
    let before = idx.entries.len();
    idx.entries.retain(|e| !ids.iter().any(|id| id == &e.id));
    let removed = before - idx.entries.len();
    if removed > 0 {
        save(&idx)?;
    }
    Ok(removed)
}

/// Resolve a project's `.cutproj` path from its index id OR a path string (matches
/// by id, then by normalized path). Returns None when no index entry matches.
/// Used by `project.delete` to find the directory to remove from an id.
pub fn path_for(id_or_path: &str) -> Option<String> {
    let _guard = projects_index_io_lock()
        .lock()
        .expect("projects index lock");
    let idx = load();
    let key_norm = normalize_path(id_or_path);
    idx.entries
        .iter()
        .find(|e| e.id == id_or_path || normalize_path(&e.path) == key_norm)
        .map(|e| e.path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, path: &str, created: u64, opened: u64) -> ProjectEntry {
        make_entry(name, path, created, opened, None, None)
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
}

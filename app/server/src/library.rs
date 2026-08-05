//! Role: the GLOBAL ASSET LIBRARY — media (video/audio/image) that lives across
//! projects at `~/.shellx-cut/library/`, organized by folders + tags, and pulled
//! into any project on demand. The Cut analogue of the ShellX Canvas asset library
//! (LIBRARY_V2_DESIGN); adapted to MEDIA (no SVG/icons/components → no sanitizer).
//!
//! Storage model = HYBRID: a library item LINKS its original by
//! `src_path` by default (tiny disk, like an NLE's linked media); the per-asset
//! "Copy into library (portable)" path content-addresses a managed copy into
//! `blobs/<sha256>.<ext>` (survives the original moving). Exactly one of
//! `src_path` / `blob` is set. Every add is ffprobe-VALIDATED (corrupt → rejected)
//! and the asset KIND is derived from the probe (not a caller hint) — magic-byte
//! honesty, no spoofing.
//!
//! Layering mirrors projects_index.rs: PURE manifest ops (add/remove/move/tag/
//! favorite/use/folders/query/migrate) take an injected `now_ms` and are unit-
//! tested with no I/O; `load`/`save`/`store_blob` do the filesystem work;
//! `add_from_path` composes probe → hash → (link|copy) → upsert → save for dispatch.
//!
//! Callers: dispatch.rs (the `library.*` verbs); http.rs (`/api/library-blob`).
//! Deps: serde, cut_media::probe (validate+classify), cut_core::hash_file, userdata.

use crate::userdata;
use cut_core::CutError;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static LIBRARY_IO_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn library_io_lock() -> &'static Mutex<()> {
    LIBRARY_IO_LOCK.get_or_init(|| Mutex::new(()))
}

/// Slim probe facts kept on a library item for the UI (the full ffprobe raw is not
/// stored — the library is an index, not a cache).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct LibProbe {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_audio: Option<bool>,
}

/// One library asset. `id` = first 16 hex of the content sha256 (so the same file
/// added twice — or linked then copied — is ONE item; ids never collide with
/// caller strings). `kind` ∈ "video"|"audio"|"image" (probe-derived). Exactly one
/// of `src_path` (linked original) / `blob` (content-addressed copy filename) is set.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LibItem {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub favorite: bool,
    pub added_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uses: Option<u64>,
    pub source: String, // "user" | "agent"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<LibProbe>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The on-disk library document. Versioned for forward migration.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LibraryManifest {
    pub version: u32,
    pub items: Vec<LibItem>,
    pub folders: Vec<String>,
}

impl Default for LibraryManifest {
    fn default() -> Self {
        Self {
            version: 1,
            items: Vec::new(),
            folders: Vec::new(),
        }
    }
}

/// Current wall-clock ms since epoch (0 on the impossible pre-epoch error).
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 16-hex id from a `sha256:<hex>` digest string (the content hash).
pub fn id_from_hash(hash: &str) -> String {
    hash.strip_prefix("sha256:")
        .unwrap_or(hash)
        .chars()
        .take(16)
        .collect()
}

// ---- PURE manifest ops (unit-tested; no I/O, no clock) ---------------------

impl LibraryManifest {
    /// Insert a new item or update the existing one with the same id (preserving the
    /// original `added_ms` + usage counters — re-adding a known asset is idempotent).
    pub fn upsert(&mut self, mut item: LibItem) {
        if let Some(existing) = self.items.iter_mut().find(|i| i.id == item.id) {
            item.added_ms = existing.added_ms;
            item.used_ms = existing.used_ms;
            item.uses = existing.uses;
            // keep favorite/tags/folder the user already set unless the caller set them
            if item.tags.is_empty() {
                item.tags = existing.tags.clone();
            }
            if item.folder.is_none() {
                item.folder = existing.folder.clone();
            }
            item.favorite = existing.favorite || item.favorite;
            *existing = item;
        } else {
            self.items.push(item);
        }
    }

    /// Remove the item with `id`; returns it (so the caller can unlink an orphaned
    /// blob). The `.cutproj`/original file is never touched.
    pub fn remove(&mut self, id: &str) -> Option<LibItem> {
        if let Some(pos) = self.items.iter().position(|i| i.id == id) {
            Some(self.items.remove(pos))
        } else {
            None
        }
    }

    /// True if any (other) item still references this blob filename — guards unlink.
    pub fn blob_referenced(&self, blob: &str) -> bool {
        self.items.iter().any(|i| i.blob.as_deref() == Some(blob))
    }

    pub fn move_to(&mut self, id: &str, folder: Option<String>) -> bool {
        if let Some(i) = self.items.iter_mut().find(|i| i.id == id) {
            i.folder = folder.filter(|f| !f.is_empty());
            true
        } else {
            false
        }
    }

    pub fn set_tags(&mut self, id: &str, tags: Vec<String>) -> bool {
        if let Some(i) = self.items.iter_mut().find(|i| i.id == id) {
            i.tags = tags;
            true
        } else {
            false
        }
    }

    /// Stable global tag navigation. This intentionally ignores active list
    /// filters so choosing one tag does not make every other tag disappear.
    pub fn tag_facets(&self) -> Vec<String> {
        let mut tags: Vec<String> = self
            .items
            .iter()
            .flat_map(|item| item.tags.iter().cloned())
            .collect();
        tags.sort_by_key(|tag| tag.to_lowercase());
        tags.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        tags
    }

    pub fn set_favorite(&mut self, id: &str, on: bool) -> bool {
        if let Some(i) = self.items.iter_mut().find(|i| i.id == id) {
            i.favorite = on;
            true
        } else {
            false
        }
    }

    pub fn bump_use(&mut self, id: &str, now_ms: u64) -> bool {
        if let Some(i) = self.items.iter_mut().find(|i| i.id == id) {
            i.used_ms = Some(now_ms);
            i.uses = Some(i.uses.unwrap_or(0) + 1);
            true
        } else {
            false
        }
    }

    /// Repoint a missing linked item without changing its content-addressed
    /// identity or user-authored organization. The caller has already probed and
    /// hashed the replacement file.
    pub fn relink(
        &mut self,
        id: &str,
        new_path: String,
        content_id: &str,
        kind: &str,
        probe: LibProbe,
    ) -> Result<LibItem, CutError> {
        let item = self
            .items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| {
                CutError::new(
                    cut_core::error_codes::NOT_FOUND,
                    format!("no Library item '{id}'"),
                    "the requested Library item does not exist",
                )
            })?;
        if item.blob.is_some() || item.src_path.is_none() {
            return Err(CutError::new(
                cut_core::error_codes::INVALID_ARGS,
                format!("Library item '{id}' is a managed copy"),
                "library.relink only repairs a linked source path",
            )
            .with_suggested_action(
                "re-add the original with library.add{copy:true} to repair a managed copy",
            ));
        }
        if content_id != id {
            return Err(CutError::new(
                cut_core::error_codes::CONFLICT,
                "the selected file is different media",
                format!("expected content id '{id}', got '{content_id}'"),
            )
            .with_suggested_action(
                "choose the same media at its new location, or add this file as a new Library item",
            ));
        }
        if kind != item.kind {
            return Err(CutError::new(
                cut_core::error_codes::CONFLICT,
                "the selected file has a different media type",
                format!("expected '{}', got '{kind}'", item.kind),
            )
            .with_suggested_action("choose the original media file at its new location"));
        }
        item.src_path = Some(new_path);
        item.probe = Some(probe);
        Ok(item.clone())
    }

    pub fn add_folder(&mut self, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() || self.folders.iter().any(|f| f == name) {
            return false;
        }
        self.folders.push(name.to_string());
        true
    }

    /// Rename a folder + re-point every item in it. False if `old` is unknown or
    /// `new` is empty/taken.
    pub fn rename_folder(&mut self, old: &str, new: &str) -> bool {
        let new = new.trim();
        if new.is_empty() || self.folders.iter().any(|f| f == new) {
            return false;
        }
        let Some(slot) = self.folders.iter_mut().find(|f| f.as_str() == old) else {
            return false;
        };
        *slot = new.to_string();
        for i in self.items.iter_mut() {
            if i.folder.as_deref() == Some(old) {
                i.folder = Some(new.to_string());
            }
        }
        true
    }

    /// Remove a folder + un-folder its items (they move to the root, not deleted).
    pub fn remove_folder(&mut self, name: &str) -> bool {
        let before = self.folders.len();
        self.folders.retain(|f| f != name);
        if self.folders.len() == before {
            return false;
        }
        for i in self.items.iter_mut() {
            if i.folder.as_deref() == Some(name) {
                i.folder = None;
            }
        }
        true
    }

    /// Filter + sort borrowed items. Keeping the result borrowed matters for large
    /// libraries: callers can page first, then clone/serialize only the requested
    /// window instead of cloning every match.
    pub fn query_refs(
        &self,
        kind: Option<&str>,
        folder: Option<&str>,
        tag: Option<&str>,
        q: Option<&str>,
        favorite_only: bool,
        ids: Option<&HashSet<String>>,
        sort: &str,
    ) -> Vec<&LibItem> {
        let needle = q.map(|s| s.to_lowercase());
        let mut out: Vec<&LibItem> = self
            .items
            .iter()
            .filter(|i| kind.is_none_or(|k| i.kind == k))
            .filter(|i| folder.is_none_or(|f| i.folder.as_deref() == Some(f)))
            .filter(|i| tag.is_none_or(|t| i.tags.iter().any(|x| x == t)))
            .filter(|i| !favorite_only || i.favorite)
            .filter(|i| ids.is_none_or(|wanted| wanted.contains(&i.id)))
            .filter(|i| match &needle {
                // Free-text search matches the NAME or any TAG (a tag chip is
                // also a one-click exact filter via the `tag` arg; this is the
                // typed-search path so "hero" finds tag:hero items by name OR tag).
                Some(n) => {
                    i.name.to_lowercase().contains(n)
                        || i.tags.iter().any(|t| t.to_lowercase().contains(n))
                }
                None => true,
            })
            .collect();
        match sort {
            "name" => out.sort_by_key(|a| a.name.to_lowercase()),
            "recent" => out.sort_by_key(|a| std::cmp::Reverse(a.used_ms.unwrap_or(0))),
            "uses" => out.sort_by_key(|a| std::cmp::Reverse(a.uses.unwrap_or(0))),
            _ => out.sort_by_key(|a| std::cmp::Reverse(a.added_ms)), // "added"
        }
        out
    }

    /// Compatibility helper for internal callers/tests that need an owned complete
    /// snapshot. The public `library.list` handler uses `query_refs` and pages
    /// before cloning.
    #[cfg(test)]
    pub fn query(
        &self,
        kind: Option<&str>,
        folder: Option<&str>,
        tag: Option<&str>,
        q: Option<&str>,
        sort: &str,
    ) -> Vec<LibItem> {
        self.query_refs(kind, folder, tag, q, false, None, sort)
            .into_iter()
            .cloned()
            .collect()
    }
}

pub const LIBRARY_LIST_DEFAULT_LIMIT: usize = 100;
pub const LIBRARY_LIST_MAX_LIMIT: usize = 500;
pub const LIBRARY_LIST_MAX_IDS: usize = 500;

/// Clamp a requested page to the result range and report the next page offset.
/// The caller validates `limit` before invoking this helper.
pub fn page_bounds(total: usize, offset: usize, limit: usize) -> (usize, usize, Option<usize>) {
    let start = offset.min(total);
    let end = start.saturating_add(limit).min(total);
    let next = (end < total).then_some(end);
    (start, end, next)
}

/// Tolerant migration from a raw JSON value (missing/old/junk → empty library).
pub fn migrate(raw: serde_json::Value) -> LibraryManifest {
    serde_json::from_value::<LibraryManifest>(raw).unwrap_or_default()
}

// ---- I/O + blob store (filesystem) -----------------------------------------

/// Load the manifest (migrating on read). Missing file or no home → empty.
pub fn load() -> LibraryManifest {
    let Some(p) = userdata::library_manifest_path() else {
        return LibraryManifest::default();
    };
    match std::fs::read_to_string(&p) {
        Ok(s) => match serde_json::from_str::<serde_json::Value>(&s) {
            Ok(v) => migrate(v),
            Err(_) => LibraryManifest::default(),
        },
        Err(_) => LibraryManifest::default(),
    }
}

/// Run a read-only operation against a manifest loaded under the library lock.
pub fn with_manifest<T>(f: impl FnOnce(&LibraryManifest) -> T) -> T {
    let _guard = library_io_lock().lock().expect("library lock");
    let m = load();
    f(&m)
}

/// Run a load → mutate → save operation under the library lock.
pub fn mutate_manifest<T>(
    f: impl FnOnce(&mut LibraryManifest) -> Result<T, CutError>,
) -> Result<T, CutError> {
    let _guard = library_io_lock().lock().expect("library lock");
    let mut m = load();
    let out = f(&mut m)?;
    save(&m)?;
    Ok(out)
}

/// Persist the manifest (mkdir -p the library dir; pretty JSON).
pub fn save(m: &LibraryManifest) -> Result<(), CutError> {
    let Some(p) = userdata::library_manifest_path() else {
        return Err(CutError::new(
            cut_core::error_codes::IO,
            "no home dir for the media library".to_string(),
            "set HOME/USERPROFILE so ShellX Cut can persist ~/.shellx-cut/library/library.json"
                .to_string(),
        ));
    };
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_err("create library dir", e))?;
    }
    let json = serde_json::to_string_pretty(m).unwrap_or_else(|_| "{}".into());
    std::fs::write(&p, json).map_err(|e| io_err("write library manifest", e))
}

/// Absolute path of a stored blob (`~/.shellx-cut/library/blobs/<file>`).
pub fn blob_path(file: &str) -> Option<PathBuf> {
    userdata::library_blobs_dir().map(|d| d.join(file))
}

/// Copy `src` into the content-addressed blob store as `<hash_hex>.<ext>` (skipping
/// the copy if it already exists — free dedup). Returns the blob filename.
fn store_blob(src: &Path, hash: &str, ext: &str) -> Result<String, CutError> {
    let hex = hash.strip_prefix("sha256:").unwrap_or(hash);
    let file = if ext.is_empty() {
        hex.to_string()
    } else {
        format!("{hex}.{ext}")
    };
    let dir = userdata::library_blobs_dir().ok_or_else(|| {
        CutError::new(
            cut_core::error_codes::IO,
            "no home dir for the library blob store".to_string(),
            "set HOME/USERPROFILE".to_string(),
        )
    })?;
    std::fs::create_dir_all(&dir).map_err(|e| io_err("create blobs dir", e))?;
    let dest = dir.join(&file);
    if !dest.exists() {
        std::fs::copy(src, &dest).map_err(|e| io_err("copy into library blobs", e))?;
    }
    Ok(file)
}

/// Unlink a blob file (best-effort; missing = ok).
pub fn remove_blob(file: &str) {
    if let Some(p) = blob_path(file) {
        let _ = std::fs::remove_file(p);
    }
}

fn io_err(ctx: &str, e: std::io::Error) -> CutError {
    CutError::new(
        cut_core::error_codes::IO,
        format!("{ctx}: {e}"),
        "filesystem error".to_string(),
    )
}

/// Lowercased file extension (no dot), or "" if none.
fn ext_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default()
}

fn slim_probe(probe: &cut_media::MediaProbe) -> LibProbe {
    LibProbe {
        duration_ms: probe.duration_ms,
        width: probe.width,
        height: probe.height,
        has_audio: Some(probe.has_audio),
    }
}

/// Add a media file to the library: ffprobe-VALIDATE (corrupt → error) + classify,
/// content-hash, then LINK (`copy=false`, default) or COPY into the blob store
/// (`copy=true`). Idempotent by content id. `known_hash` skips re-hashing when the
/// caller already has it (an in-project asset). Returns the stored item.
#[allow(clippy::too_many_arguments)]
pub fn add_from_path(
    path: &Path,
    name: Option<String>,
    tags: Vec<String>,
    folder: Option<String>,
    copy: bool,
    source: &str,
    known_hash: Option<String>,
    now_ms: u64,
) -> Result<LibItem, CutError> {
    if !path.exists() {
        return Err(CutError::new(
            cut_core::error_codes::INVALID_ARGS,
            format!("no such file: {}", path.display()),
            "the path to add does not exist".to_string(),
        ));
    }
    // VALIDATE + classify via ffprobe (rejects corrupt; derives kind, not a caller hint).
    let probe = cut_media::probe(path)?;
    let hash = match known_hash {
        Some(h) => h,
        None => cut_core::hash_file(path)?,
    };
    let id = id_from_hash(&hash);
    let ext = ext_of(path);
    let display = name.unwrap_or_else(|| {
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("asset")
            .to_string()
    });

    let (src_path, blob) = if copy {
        (None, Some(store_blob(path, &hash, &ext)?))
    } else {
        (Some(path.to_string_lossy().to_string()), None)
    };

    let item = LibItem {
        id,
        kind: probe.kind.clone(),
        name: display,
        src_path,
        blob,
        tags,
        folder: folder.filter(|f| !f.is_empty()),
        favorite: false,
        added_ms: now_ms,
        used_ms: None,
        uses: None,
        source: source.to_string(),
        probe: Some(slim_probe(&probe)),
    };

    mutate_manifest(|m| {
        m.upsert(item.clone());
        Ok(())
    })?;
    Ok(item)
}

/// Validate and relink a linked Library item to the same media bytes at a new
/// filesystem path. Different bytes are rejected so the content-derived id and
/// cross-project membership badges remain honest.
pub fn relink_from_path(id: &str, path: &Path) -> Result<LibItem, CutError> {
    if !path.is_file() {
        return Err(CutError::new(
            cut_core::error_codes::INVALID_ARGS,
            format!("no such media file: {}", path.display()),
            "the selected relink path is not a file",
        ));
    }
    let probe = cut_media::probe(path)?;
    let hash = cut_core::hash_file(path)?;
    let content_id = id_from_hash(&hash);
    let new_path = path.to_string_lossy().into_owned();
    let kind = probe.kind.clone();
    let slim = slim_probe(&probe);
    mutate_manifest(|manifest| manifest.relink(id, new_path, &content_id, &kind, slim))
}

pub(crate) fn item_media_path_from_item(
    item: &LibItem,
    blobs_dir: Option<&Path>,
) -> Option<PathBuf> {
    let path = if let Some(blob) = &item.blob {
        blobs_dir?.join(blob)
    } else {
        PathBuf::from(item.src_path.as_ref()?)
    };
    path.is_file().then_some(path)
}

/// Resolve a library item's absolute media path (its linked original, or its
/// managed copy). None if the item is unknown or either backing file is gone.
pub fn item_media_path(m: &LibraryManifest, id: &str) -> Option<PathBuf> {
    let item = m.items.iter().find(|i| i.id == id)?;
    let blobs_dir = userdata::library_blobs_dir();
    item_media_path_from_item(item, blobs_dir.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(id_seed: &str, name: &str, added: u64) -> LibItem {
        LibItem {
            id: id_seed.to_string(),
            kind: "image".into(),
            name: name.into(),
            src_path: Some(format!("/m/{name}")),
            blob: None,
            tags: vec![],
            folder: None,
            favorite: false,
            added_ms: added,
            used_ms: None,
            uses: None,
            source: "user".into(),
            probe: None,
        }
    }

    #[test]
    fn upsert_dedups_by_id_and_preserves_added() {
        let mut m = LibraryManifest::default();
        m.upsert(img("a", "one.png", 100));
        let mut again = img("a", "one-renamed.png", 999);
        again.tags = vec!["k".into()];
        m.upsert(again);
        assert_eq!(m.items.len(), 1);
        assert_eq!(m.items[0].name, "one-renamed.png");
        assert_eq!(m.items[0].added_ms, 100); // immutable
        assert_eq!(m.items[0].tags, vec!["k".to_string()]);
    }

    #[test]
    fn remove_returns_item_and_blob_guard() {
        let mut m = LibraryManifest::default();
        let mut b = img("a", "x.png", 1);
        b.src_path = None;
        b.blob = Some("hash.png".into());
        m.upsert(b);
        m.upsert({
            let mut c = img("b", "y.png", 1);
            c.src_path = None;
            c.blob = Some("hash.png".into()); // shares the blob
            c
        });
        assert!(m.blob_referenced("hash.png"));
        let removed = m.remove("a").unwrap();
        assert_eq!(removed.blob.as_deref(), Some("hash.png"));
        assert!(m.blob_referenced("hash.png")); // 'b' still references → don't unlink
        m.remove("b");
        assert!(!m.blob_referenced("hash.png")); // now safe to unlink
    }

    #[test]
    fn move_tag_favorite_use() {
        let mut m = LibraryManifest::default();
        m.upsert(img("a", "x.png", 1));
        assert!(m.move_to("a", Some("Logos".into())));
        assert_eq!(m.items[0].folder.as_deref(), Some("Logos"));
        assert!(m.move_to("a", Some("".into()))); // empty clears
        assert_eq!(m.items[0].folder, None);
        assert!(m.set_tags("a", vec!["brand".into(), "blue".into()]));
        assert_eq!(m.items[0].tags.len(), 2);
        assert!(m.set_favorite("a", true));
        assert!(m.items[0].favorite);
        assert!(m.bump_use("a", 500));
        assert!(m.bump_use("a", 600));
        assert_eq!(m.items[0].uses, Some(2));
        assert_eq!(m.items[0].used_ms, Some(600));
        assert!(!m.bump_use("missing", 1));
    }

    #[test]
    fn folders_add_rename_remove_repoint() {
        let mut m = LibraryManifest::default();
        assert!(m.add_folder("A"));
        assert!(!m.add_folder("A")); // dup
        assert!(!m.add_folder("  ")); // empty
        m.upsert({
            let mut i = img("x", "x.png", 1);
            i.folder = Some("A".into());
            i
        });
        assert!(m.rename_folder("A", "B"));
        assert_eq!(m.items[0].folder.as_deref(), Some("B"));
        assert!(!m.rename_folder("nope", "C"));
        assert!(m.remove_folder("B"));
        assert_eq!(m.items[0].folder, None); // un-foldered, not deleted
        assert!(m.items.len() == 1);
    }

    #[test]
    fn query_filters_and_sorts() {
        let mut m = LibraryManifest::default();
        let mut a = img("a", "Alpha", 10);
        a.kind = "video".into();
        a.tags = vec!["hero".into()];
        let mut b = img("b", "Beta", 30);
        b.folder = Some("F".into());
        let c = img("c", "Gamma", 20);
        m.upsert(a);
        m.upsert(b);
        m.upsert(c);
        m.bump_use("c", 999);
        // added (default) = newest first
        assert_eq!(m.query(None, None, None, None, "added")[0].name, "Beta");
        // name asc
        assert_eq!(m.query(None, None, None, None, "name")[0].name, "Alpha");
        // recent = used desc → Gamma (used) first
        assert_eq!(m.query(None, None, None, None, "recent")[0].name, "Gamma");
        // kind filter
        let vids = m.query(Some("video"), None, None, None, "added");
        assert_eq!(vids.len(), 1);
        assert_eq!(vids[0].name, "Alpha");
        // folder filter
        assert_eq!(m.query(None, Some("F"), None, None, "added").len(), 1);
        // tag filter
        assert_eq!(m.query(None, None, Some("hero"), None, "added").len(), 1);
        // name search
        assert_eq!(m.query(None, None, None, Some("amm"), "added").len(), 1);
        // free-text search also matches a TAG substring (Alpha is tagged "hero"):
        // "her" is in no NAME, so a name-only filter would return 0.
        let by_tag = m.query(None, None, None, Some("her"), "added");
        assert_eq!(by_tag.len(), 1);
        assert_eq!(by_tag[0].name, "Alpha");
        let beta = m.items.iter_mut().find(|item| item.id == "b").unwrap();
        beta.tags = vec!["Hero".into(), "secondary".into()];
        assert_eq!(
            m.tag_facets(),
            vec!["hero".to_string(), "secondary".to_string()]
        );
    }

    #[test]
    fn query_refs_filters_favorites_and_exact_ids_before_paging() {
        let mut m = LibraryManifest::default();
        for n in 0..1_000_u64 {
            let mut item = img(&format!("id-{n:04}"), &format!("asset-{n:04}.png"), n);
            item.favorite = n % 10 == 0;
            m.upsert(item);
        }

        let favorites = m.query_refs(None, None, None, None, true, None, "added");
        assert_eq!(favorites.len(), 100);
        assert_eq!(favorites[0].id, "id-0990");

        let wanted = HashSet::from([
            "id-0001".to_string(),
            "id-0501".to_string(),
            "id-0999".to_string(),
        ]);
        let exact = m.query_refs(None, None, None, None, false, Some(&wanted), "added");
        assert_eq!(
            exact
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["id-0999", "id-0501", "id-0001"]
        );
    }

    #[test]
    fn relink_preserves_identity_and_organization_and_rejects_wrong_media() {
        let mut m = LibraryManifest::default();
        let mut item = img("same-id", "Original name.png", 42);
        item.tags = vec!["hero".into()];
        item.folder = Some("Campaign".into());
        item.favorite = true;
        item.uses = Some(7);
        m.upsert(item);
        let replacement_probe = LibProbe {
            width: Some(1920),
            height: Some(1080),
            ..LibProbe::default()
        };

        let conflict = m
            .relink(
                "same-id",
                "/new/different.png".into(),
                "different-id",
                "image",
                replacement_probe.clone(),
            )
            .unwrap_err();
        assert_eq!(conflict.code, cut_core::error_codes::CONFLICT);
        assert_eq!(m.items[0].src_path.as_deref(), Some("/m/Original name.png"));

        let repaired = m
            .relink(
                "same-id",
                "/new/original.png".into(),
                "same-id",
                "image",
                replacement_probe.clone(),
            )
            .unwrap();
        assert_eq!(repaired.src_path.as_deref(), Some("/new/original.png"));
        assert_eq!(repaired.name, "Original name.png");
        assert_eq!(repaired.tags, vec!["hero".to_string()]);
        assert_eq!(repaired.folder.as_deref(), Some("Campaign"));
        assert!(repaired.favorite);
        assert_eq!(repaired.uses, Some(7));
        assert_eq!(repaired.probe, Some(replacement_probe));
    }

    #[test]
    fn relink_refuses_managed_copies() {
        let mut m = LibraryManifest::default();
        let mut item = img("managed", "managed.png", 1);
        item.src_path = None;
        item.blob = Some("managed.png".into());
        m.upsert(item);
        let error = m
            .relink(
                "managed",
                "/replacement.png".into(),
                "managed",
                "image",
                LibProbe::default(),
            )
            .unwrap_err();
        assert_eq!(error.code, cut_core::error_codes::INVALID_ARGS);
    }

    #[test]
    fn page_bounds_handles_empty_boundary_and_large_fixtures() {
        assert_eq!(page_bounds(0, 0, 100), (0, 0, None));
        assert_eq!(page_bounds(20, 0, 100), (0, 20, None));
        assert_eq!(page_bounds(1_000, 0, 100), (0, 100, Some(100)));
        assert_eq!(page_bounds(1_000, 900, 100), (900, 1_000, None));
        assert_eq!(page_bounds(10_000, 9_900, 100), (9_900, 10_000, None));
        assert_eq!(page_bounds(10_000, 20_000, 100), (10_000, 10_000, None));
    }

    #[test]
    fn media_path_requires_a_real_file_for_links_and_managed_copies() {
        let dir = tempfile::tempdir().unwrap();
        let linked_path = dir.path().join("linked.mp4");
        let blob_path = dir.path().join("managed.mp4");
        std::fs::write(&linked_path, b"linked").unwrap();
        std::fs::write(&blob_path, b"managed").unwrap();

        let mut linked = img("linked", "linked.mp4", 1);
        linked.src_path = Some(linked_path.to_string_lossy().into_owned());
        let mut managed = img("managed", "managed.mp4", 2);
        managed.src_path = None;
        managed.blob = Some("managed.mp4".into());

        assert_eq!(
            item_media_path_from_item(&linked, Some(dir.path())),
            Some(linked_path.clone())
        );
        assert_eq!(
            item_media_path_from_item(&managed, Some(dir.path())),
            Some(blob_path.clone())
        );

        std::fs::remove_file(linked_path).unwrap();
        std::fs::remove_file(blob_path).unwrap();
        assert_eq!(item_media_path_from_item(&linked, Some(dir.path())), None);
        assert_eq!(item_media_path_from_item(&managed, Some(dir.path())), None);
    }

    #[test]
    fn id_from_hash_strips_prefix_and_truncates() {
        let id = id_from_hash("sha256:0123456789abcdef0123456789abcdef");
        assert_eq!(id, "0123456789abcdef");
        assert_eq!(id_from_hash("deadbeef").len(), 8);
    }

    #[test]
    fn migrate_tolerates_junk() {
        assert_eq!(migrate(serde_json::json!(null)).items.len(), 0);
        assert_eq!(migrate(serde_json::json!({"x": 1})).items.len(), 0);
        let good = migrate(serde_json::json!({
            "version": 1, "folders": ["A"],
            "items": [{"id":"a","type":"video","name":"v.mp4","src_path":"/m/v.mp4","added_ms":1,"source":"user"}]
        }));
        assert_eq!(good.items.len(), 1);
        assert_eq!(good.items[0].kind, "video");
        assert_eq!(good.folders, vec!["A".to_string()]);
    }
}

//! nest.rs — server-side NEST / COMPOUND-CLIP bake.
//!
//! Role: render a nest's sub-timeline to a content-addressed cache file so the MAIN
//! render consumes it as the nest clip's source — mirroring `matte.rs` (which bakes a
//! content-addressed alpha the renderer reads). LAZY (baked at render time) + cached
//! by sub-timeline hash, so a nest is re-rendered only when its contents change.
//!
//! The main renderer (cut-media) stays UNCHANGED and nest-blind: [`bake_and_flatten`]
//! returns a project CLONE with a synthetic source asset (keyed by the nest id, the id
//! the nest clips' `asset` field already references) added for each nest, so the
//! existing filtergraph resolves the nest clips' source to the baked file. A project
//! with NO nest flattens to itself (`project.clone()`) → byte-identical render.
//!
//! Why a SERVER-side bake (like matte) and not a cut-media change: rendering a
//! sub-timeline is itself a `render_final` call, which lives in cut-media; doing it
//! HERE keeps cut-media a single non-recursive render path and guarantees the non-nest
//! render byte-identical (the renderer never sees a nest). The bake reuses the exact
//! `render_final` path on the sub-project, so a nested clip's grade/effects/timing
//! render identically inside the nest.
//!
//! Dependencies: cut_core (Project/Nest/Asset/Edl), cut_media (render_final, PathFence),
//! sha2. Primary caller: dispatch.rs `render_final` (inside the render job).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use cut_core::{edl_from_project, error_codes, Asset, CutError, Edl, Nest, Project};
use cut_media::render::{render_final, RenderOptions, RenderPreset};
use cut_media::PathFence;
use sha2::{Digest, Sha256};

static BAKE_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
static BAKE_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// The result of baking one nest's sub-timeline.
pub struct BakedNest {
    /// The baked mp4 (content-addressed under `<project>/cache/nest/`).
    pub path: PathBuf,
    /// Its duration (= the realized sub-timeline span), ms.
    pub duration_ms: u64,
    /// Content-address tag (the cache key) — also the synthetic asset's `hash`.
    pub tag: String,
    /// True when served from the cache (no re-render this call).
    pub cached: bool,
}

fn io_err(ctx: &str, e: impl std::fmt::Display) -> CutError {
    CutError::new(
        error_codes::IO,
        format!("nest bake: {ctx}: {e}"),
        "failed writing/reading the nest bake cache",
    )
}

fn cache_ready(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
}

fn bake_lock(path: &Path) -> Result<Arc<Mutex<()>>, CutError> {
    let locks = BAKE_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut locks = locks
        .lock()
        .map_err(|_| io_err("acquire cache-lock registry", "lock poisoned"))?;
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(path).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(path.to_path_buf(), Arc::downgrade(&lock));
    Ok(lock)
}

/// Build the self-contained sub-project for a nest: the parent's render SETTINGS +
/// ASSETS (the real sources the nested clips reference) + the nest's own TRACKS. The
/// sub-EDL is then derived by the SAME `edl_from_project` as the main timeline.
fn sub_project(parent: &Project, nest: &Nest) -> Project {
    let mut sub = Project::new(
        &format!("{}::{}", parent.name, nest.id),
        parent.settings.clone(),
    );
    sub.assets = parent.assets.clone();
    sub.tracks = nest.tracks.clone();
    sub
}

/// Content-address tag for a nest's bake: a sha256 over the render SETTINGS, the
/// referenced source assets' CONTENT HASHES (a changed source re-bakes), and the
/// derived sub-EDL — i.e. exactly the inputs that determine the rendered pixels. The
/// tag changes iff the bake would change, so a warm cache is always safe to reuse.
fn nest_cache_tag(sub: &Project, edl: &Edl) -> String {
    let mut h = Sha256::new();
    h.update(serde_json::to_vec(&sub.settings).unwrap_or_default());
    // Referenced source assets, sorted + deduped, by (id, content-hash).
    let mut refd: Vec<(&str, &str)> = edl
        .segments
        .iter()
        .filter_map(|s| s.asset.as_deref())
        .filter_map(|id| sub.assets.get(id).map(|a| (id, a.hash.as_str())))
        .collect();
    refd.sort_unstable();
    refd.dedup();
    for (id, hash) in refd {
        h.update(id.as_bytes());
        h.update(b"\0");
        h.update(hash.as_bytes());
        h.update(b"\0");
    }
    h.update(serde_json::to_vec(edl).unwrap_or_default());
    format!("{:x}", h.finalize())
}

/// Ensure the nest's baked mp4 exists, rendering it on a cache miss. Idempotent +
/// content-addressed: a warm cache returns instantly (`cached: true`).
pub fn ensure_baked(
    parent: &Project,
    project_dir: &Path,
    nest: &Nest,
) -> Result<BakedNest, CutError> {
    let sub = sub_project(parent, nest);
    let edl = edl_from_project(&sub);
    if edl.duration_ms == 0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("nest '{}' is empty — nothing to bake", nest.id),
            "a nest must contain at least one clip to render",
        ));
    }
    let tag = nest_cache_tag(&sub, &edl);
    let dir = project_dir.join("cache").join("nest");
    let out = dir.join(format!("{tag}.mp4"));
    std::fs::create_dir_all(&dir).map_err(|e| io_err("create cache dir", e))?;

    // Preview, frame, and export requests can arrive together immediately after
    // edit.nest. Serialize the first bake for this exact output and re-check the
    // cache after taking the lock. Without this, same-process callers shared the
    // same `.part-<pid>` path and one FFmpeg process corrupted the other's write.
    let lock = bake_lock(&out)?;
    let _bake_guard = lock
        .lock()
        .map_err(|_| io_err("acquire output bake lock", "lock poisoned"))?;
    if cache_ready(&out) {
        return Ok(BakedNest {
            path: out,
            duration_ms: edl.duration_ms,
            tag,
            cached: true,
        });
    }
    // A zero-byte/non-file cache entry can only be stale: writers render to a
    // unique temp and publish by rename. Remove it before finalization so Windows
    // does not reject rename because a dead destination already exists.
    if out.exists() {
        std::fs::remove_file(&out).map_err(|e| io_err("remove stale cache entry", e))?;
    }
    // Render the sub-timeline at HIGH quality — this is an INTERMEDIATE, so minimise
    // generation loss before the final encode. `render_final` fences its own output;
    // `cache/nest/` is inside the project dir, so the fence allows it.
    let fence = PathFence::new(project_dir)?;
    let preset = RenderPreset::named("high").unwrap_or_default();
    // Render to a unique temp then atomically rename, so a concurrent or aborted bake
    // never leaves a half-written file at the cache path that a later run would trust.
    let temp_sequence = BAKE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(
        "{tag}.part-{}-{temp_sequence}.mp4",
        std::process::id()
    ));
    if let Err(error) = render_final(
        &sub,
        &edl,
        &fence,
        &tmp,
        &preset,
        RenderOptions::default(),
        None,
    ) {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&tmp, &out) {
        // A different process may have published the identical content-addressed
        // output while this process rendered its unique temp. Accept only a
        // complete destination; otherwise retain the original finalize error.
        if cache_ready(&out) {
            let _ = std::fs::remove_file(&tmp);
            return Ok(BakedNest {
                path: out,
                duration_ms: edl.duration_ms,
                tag,
                cached: true,
            });
        }
        let _ = std::fs::remove_file(&tmp);
        return Err(io_err("finalise baked nest", error));
    }
    Ok(BakedNest {
        path: out,
        duration_ms: edl.duration_ms,
        tag,
        cached: false,
    })
}

/// Bake every nest and return a CLONE of `project` with a synthetic source asset
/// (keyed by the nest id — the id the nest clips' `asset` field already points at)
/// added for each nest, so the existing renderer resolves the nest clips' source to
/// the baked file. A project with NO nest returns `project.clone()` unchanged → the
/// render is byte-identical to a pre-nest render.
pub fn bake_and_flatten(project: &Project, project_dir: &Path) -> Result<Project, CutError> {
    if !project.has_nests() {
        return Ok(project.clone());
    }
    let mut flat = project.clone();
    for nest in &project.nests {
        let baked = ensure_baked(project, project_dir, nest)?;
        tracing::info!(
            nest = %nest.id,
            cached = baked.cached,
            duration_ms = baked.duration_ms,
            tag = %baked.tag,
            "baked nest for render"
        );
        // Add the baked file as a synthetic asset keyed by the nest id. No `probe` ⇒
        // collect_graph_inputs treats it as a normal video (not a looped still). The
        // same derived MP4 is also its preview proxy, allowing incremental draft
        // preview without a redundant transcode. The nest clip's src window
        // [0, span) trims it exactly (span == baked duration).
        let baked_path = baked.path.to_string_lossy().into_owned();
        flat.assets.insert(
            nest.id.clone(),
            Asset {
                path: baked_path.clone(),
                hash: format!("sha256:{}", baked.tag),
                probe: None,
                transcript: None,
                perception: None,
                proxy: Some(baked_path),
                filmstrip: None,
            },
        );
    }
    Ok(flat)
}

/// Resolve the project snapshot used by any renderer or timeline interchange
/// serializer. Non-nested projects retain the caller's exact project/EDL pair;
/// nested projects receive an ephemeral clone whose synthetic baked assets are
/// visible only to media I/O. The returned project must never be saved.
pub fn flatten_for_media_io(
    project: &Project,
    edl: &Edl,
    project_dir: &Path,
) -> Result<(Project, Edl), CutError> {
    if !project.has_nests() {
        return Ok((project.clone(), edl.clone()));
    }
    let flat = bake_and_flatten(project, project_dir)?;
    let flat_edl = edl_from_project(&flat);
    Ok((flat, flat_edl))
}

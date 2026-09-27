//! vissearch.rs — on-device visual search engine.
//!
//! Find the MOMENTS in a clip that match a text query ("the slide with the
//! chart") by matching a query embedding against per-FRAME image embeddings and
//! merging the best adjacent frames into time RANGES. The engine is MODEL-
//! AGNOSTIC: the app stays decoupled from the encoder so it
//! can swap models. The default SigLIP2 indexer uses the local perception
//! runtime and model, but any same-dim image-text embedder works: it operates on
//! stored vectors, so the ranking is fully testable without the model. The
//! SigLIP2 INDEXER (py/siglip_index.py) produces the vectors.
//!
//! Storage: one content-addressed index per asset under the project
//! (`<proj>/embeddings/<asset>.json`): {schema, model, dim, frames:[{ms, v[]}]}.
//! JSON keeps it inspectable; a binary f32 store is a future size optimization.
//!
//! Primary callers: dispatch.rs (`media.search` + the indexing path). Pure +
//! deterministic given the stored vectors.

use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

// At 1 fps a 768-dimensional SigLIP JSON index is about 60 MiB per hour.
// This admits long recordings while keeping a supplied project cache finite.
pub(crate) const MAX_VISUAL_INDEX_BYTES: u64 = 512 * 1024 * 1024;

pub(crate) fn read_bounded_file(path: &Path, max_bytes: u64) -> io::Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !plain_regular(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a regular file",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let opened = file.metadata()?;
    if !plain_regular(&opened) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a regular file",
        ));
    }
    if opened.len() > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds byte limit",
        ));
    }
    let mut bytes = Vec::new();
    file.take(max_bytes + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds byte limit",
        ));
    }
    Ok(bytes)
}

fn plain_regular(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_file() && !metadata.file_type().is_symlink() && !is_reparse(metadata)
}

#[cfg(windows)]
fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

#[cfg(not(windows))]
fn is_reparse(_metadata: &std::fs::Metadata) -> bool {
    false
}

mod runtime;
#[cfg(test)]
mod runtime_tests;
pub use runtime::{native_runtime, runtime, NativeRuntimeProvenance, Runtime};
pub(crate) use runtime::{visual_cache_runtime, VisualCacheRuntime};

/// One frame's embedding at a timeline/source instant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameEmbedding {
    /// Frame time in milliseconds (source time).
    pub ms: u64,
    /// The L2-normalizable embedding vector (raw; we normalize at compare time).
    pub v: Vec<f32>,
}

/// A per-asset embedding index (the indexer writes this; search reads it).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingIndex {
    /// Schema tag for forward-compat.
    #[serde(default = "default_schema")]
    pub schema: String,
    /// The encoder that produced the vectors (e.g. "siglip2-base-patch16-224").
    pub model: String,
    /// Embedding dimensionality (all frames + the query must match this).
    pub dim: usize,
    /// The source asset id this index belongs to.
    #[serde(default)]
    pub asset: String,
    /// Per-frame embeddings, ascending by `ms` (the indexer guarantees order).
    pub frames: Vec<FrameEmbedding>,
    /// Prepared runtime identity when the index was built through the admitted
    /// native context. Existing installed indexes deliberately omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_runtime: Option<NativeRuntimeProvenance>,
}

fn default_schema() -> String {
    "shellx-cut/vissearch/1".to_string()
}

/// One search result: a time RANGE whose frames best match the query, with the
/// match `score` (peak cosine similarity in `[−1,1]`) and the peak frame.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SearchHit {
    /// Range start (ms) — the first frame of the merged run.
    pub start_ms: u64,
    /// Range end (ms) — the last frame of the merged run.
    pub end_ms: u64,
    /// The single best-matching frame time in the range.
    pub peak_ms: u64,
    /// Peak cosine similarity across the range's frames.
    pub score: f32,
}

/// Cosine similarity of two equal-length vectors. Returns 0 for a zero vector or
/// a length mismatch (defensive — never panics).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    // Reject zero norms AND non-finite ones: a corrupt index with huge values can
    // overflow f32 to Inf, making na.sqrt()*nb.sqrt() = Inf and dot/Inf = NaN — a
    // NaN cosine would silently poison the threshold math downstream. Fall to 0.
    if na <= 0.0 || nb <= 0.0 || !na.is_finite() || !nb.is_finite() {
        return 0.0;
    }
    let c = dot / (na.sqrt() * nb.sqrt());
    if c.is_finite() {
        c
    } else {
        0.0
    }
}

/// Search an index for the moments matching `query` (a same-dim embedding).
///
/// 1. Score every frame by cosine similarity to the query.
/// 2. Take the strongest candidate frames (the top `3*top_k`, capped), so noise
///    frames don't seed spurious ranges.
/// 3. Sort the candidates by time and MERGE ones within `max_gap_ms` into ranges
///    (a sustained match = a moment); each range's score is its peak frame's,
///    `peak_ms` the argmax.
/// 4. Return the `top_k` ranges by score (desc), ties broken by earlier start.
///
/// Returns an error string if the query dim ≠ the index dim, or the index empty.
pub fn search(
    index: &EmbeddingIndex,
    query: &[f32],
    top_k: usize,
    max_gap_ms: u64,
) -> Result<Vec<SearchHit>, String> {
    if index.frames.is_empty() {
        return Err("index has no frames".into());
    }
    if query.len() != index.dim {
        return Err(format!(
            "query dim {} != index dim {}",
            query.len(),
            index.dim
        ));
    }
    let top_k = top_k.clamp(1, 50);

    // 1. score every frame.
    let mut scored: Vec<(u64, f32)> = index
        .frames
        .iter()
        .map(|f| (f.ms, cosine(query, &f.v)))
        .collect();

    // 2. candidate set = the strongest frames by a SCALE-FREE relative threshold:
    //    keep frames in the top fraction of the score SPAN, so non-matching
    //    frames never seed a range regardless of the absolute cosine scale (real
    //    SigLIP scores cluster ~0.1–0.3; a fixed cutoff would over/under-select).
    //    A ~flat span (the whole clip matches uniformly) keeps everything.
    const KEEP_FRACTION: f32 = 0.6; // keep scores in the top 40% of the span
                                    // Absolute floor (the H1 fix): a candidate must ACTUALLY resemble the query —
                                    // not merely be the best frame of a uniformly-NON-matching clip. Without it, a
                                    // query that matches nothing (all frames near-equal/orthogonal → span≈0) passes
                                    // the relative cutoff and the whole clip is returned as a false "match" with
                                    // score ~0. SigLIP2 real matches score ≳0.1 (modality-gap cosines cluster
                                    // ~0.1–0.3); non-matches sit near 0 or negative. 0.05 separates them and is the
                                    // one ENCODER-DEPENDENT knob — retune if model_id()'s similarity scale changes.
    const MIN_MATCH_COSINE: f32 = 0.05;
    let max_s = scored.iter().map(|(_, s)| *s).fold(f32::MIN, f32::max);
    // Nothing in the clip clears the floor → no matches at all (empty, not noise).
    if max_s < MIN_MATCH_COSINE {
        return Ok(Vec::new());
    }
    let min_s = scored.iter().map(|(_, s)| *s).fold(f32::MAX, f32::min);
    let span = max_s - min_s;
    // Candidate cutoff = the scale-free span gate, but NEVER below the absolute
    // floor: a flat-but-matching clip keeps everything ≥ floor; a flat NON-match
    // (already returned empty above) keeps nothing.
    let cutoff = if span > 1e-6 {
        (min_s + span * KEEP_FRACTION).max(MIN_MATCH_COSINE)
    } else {
        MIN_MATCH_COSINE
    };
    scored.retain(|(_, s)| *s >= cutoff);
    scored.sort_by_key(|(ms, _)| *ms);

    // 3. merge time-adjacent candidates within max_gap_ms into ranges.
    let mut hits: Vec<SearchHit> = Vec::new();
    for (ms, score) in scored {
        match hits.last_mut() {
            Some(h) if ms.saturating_sub(h.end_ms) <= max_gap_ms => {
                h.end_ms = ms;
                if score > h.score {
                    h.score = score;
                    h.peak_ms = ms;
                }
            }
            _ => hits.push(SearchHit {
                start_ms: ms,
                end_ms: ms,
                peak_ms: ms,
                score,
            }),
        }
    }

    // 4. rank by score desc, then earlier start; return top_k.
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.start_ms.cmp(&b.start_ms))
    });
    hits.truncate(top_k);
    Ok(hits)
}

/// The embeddings index path for an asset under a project dir.
pub fn index_path(proj_dir: &Path, asset_id: &str) -> PathBuf {
    proj_dir.join("embeddings").join(format!("{asset_id}.json"))
}

pub(crate) fn prepare_index_output_path(proj_dir: &Path, asset_id: &str) -> io::Result<PathBuf> {
    let path = index_path(proj_dir, asset_id);
    let parent = path.parent().expect("index has embeddings parent");
    match std::fs::symlink_metadata(parent) {
        Ok(metadata) if !plain_directory(&metadata) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "embeddings is not a plain directory",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            std::fs::create_dir(parent)?;
        }
        Err(error) => return Err(error),
    }
    if !plain_directory(&std::fs::symlink_metadata(parent)?) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "embeddings is not a plain directory",
        ));
    }
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if !plain_regular(&metadata) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "index is not a plain file",
        )),
        Ok(_) => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(path),
        Err(error) => Err(error),
    }
}

fn plain_directory(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_dir() && !metadata.file_type().is_symlink() && !is_reparse(metadata)
}

/// Load an asset's embedding index, or None if not indexed / unreadable.
pub fn load_index(proj_dir: &Path, asset_id: &str) -> Option<EmbeddingIndex> {
    load_index_checked(proj_dir, asset_id).ok()
}

/// Load and validate a just-produced index, retaining a bounded reason suitable
/// for a `media.index` setup error. Cache consumers deliberately keep the
/// Option-returning wrapper above: a stale or unreadable cache is a cache miss.
pub fn load_index_checked(proj_dir: &Path, asset_id: &str) -> Result<EmbeddingIndex, String> {
    let p = index_path(proj_dir, asset_id);
    let bytes = read_bounded_file(&p, MAX_VISUAL_INDEX_BYTES)
        .map_err(|error| format!("could not read index: {error}"))?;
    let index: EmbeddingIndex = serde_json::from_slice(&bytes)
        .map_err(|error| format!("index JSON does not match the embedding contract: {error}"))?;
    if index.asset != asset_id {
        return Err("index asset does not match the requested asset".into());
    }
    if index.dim == 0 {
        return Err("index dimension is zero".into());
    }
    if index.frames.is_empty() {
        return Err("index has no frame embeddings".into());
    }
    for (position, frame) in index.frames.iter().enumerate() {
        if frame.v.len() != index.dim {
            return Err(format!(
                "frame {position} dimension {} does not match index dimension {}",
                frame.v.len(),
                index.dim
            ));
        }
        if frame.v.iter().any(|value| !value.is_finite()) {
            return Err(format!(
                "frame {position} contains a non-finite embedding value"
            ));
        }
    }
    Ok(index)
}

/// Load a cache only when the prepared runtime that produced it remains exact.
/// Normal installed mode retains its historical model-agnostic cache behavior.
pub fn load_index_for_runtime(
    proj_dir: &Path,
    asset_id: &str,
    runtime: Option<&Runtime>,
) -> Option<EmbeddingIndex> {
    let index = load_index(proj_dir, asset_id)?;
    runtime
        .is_none_or(|runtime| runtime.accepts_index(&index))
        .then_some(index)
}

/// Persist an asset's embedding index (creates the embeddings/ dir). The native
/// caller uses this after it stamps its selected runtime identity.
pub fn save_index(proj_dir: &Path, index: &EmbeddingIndex) -> std::io::Result<PathBuf> {
    save_index_with_limit(proj_dir, index, MAX_VISUAL_INDEX_BYTES)
}

fn save_index_with_limit(
    proj_dir: &Path,
    index: &EmbeddingIndex,
    max_bytes: u64,
) -> std::io::Result<PathBuf> {
    let payload = serde_json::to_vec(index).map_err(std::io::Error::other)?;
    if payload.len() as u64 > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "index exceeds byte limit",
        ));
    }
    let p = prepare_index_output_path(proj_dir, &index.asset)?;
    let mut temporary = tempfile::NamedTempFile::new_in(p.parent().expect("index parent"))?;
    temporary.write_all(&payload)?;
    temporary.persist(&p).map_err(|error| error.error)?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx(frames: Vec<(u64, Vec<f32>)>) -> EmbeddingIndex {
        EmbeddingIndex {
            schema: default_schema(),
            model: "test".into(),
            dim: frames.first().map(|f| f.1.len()).unwrap_or(0),
            asset: "a1".into(),
            frames: frames
                .into_iter()
                .map(|(ms, v)| FrameEmbedding { ms, v })
                .collect(),
            native_runtime: None,
        }
    }

    #[test]
    fn cosine_basics() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert!((cosine(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-6);
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0); // mismatch → 0
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0); // zero vec → 0
                                                           // L2: a corrupt index with huge values must NOT yield NaN (overflow→Inf→
                                                           // dot/Inf=NaN would poison the threshold math). Falls to a finite 0.
        let huge = cosine(&[1e20, 1e20], &[1e20, 1e20]);
        assert!(
            huge.is_finite(),
            "pathological large vectors stay finite (got {huge})"
        );
    }

    #[test]
    fn search_no_match_returns_empty() {
        // H1: every frame points "east", the query points "north" (orthogonal →
        // cosine 0). The clip contains NOTHING resembling the query, so search must
        // return NO hits — not the whole clip as a false ~0-score "match".
        let north = vec![0.0f32, 1.0];
        let east = vec![1.0f32, 0.0];
        let frames: Vec<(u64, Vec<f32>)> = (0..8).map(|i| (i * 1000, east.clone())).collect();
        let hits = search(&idx(frames), &north, 3, 1500).expect("search ok");
        assert!(
            hits.is_empty(),
            "a query that matches nothing returns no hits, got {hits:?}"
        );
    }

    #[test]
    fn search_uniform_match_keeps_the_clip() {
        // H1 counterpart: every frame DOES match the query (all "north", cosine ~1).
        // span≈0 but the absolute floor is cleared, so the whole clip is one hit
        // (the "uniformly matching clip" case must NOT be suppressed by the floor).
        let north = vec![0.0f32, 1.0];
        let frames: Vec<(u64, Vec<f32>)> = (0..6).map(|i| (i * 1000, north.clone())).collect();
        let hits = search(&idx(frames), &north, 3, 1500).expect("search ok");
        assert_eq!(
            hits.len(),
            1,
            "a uniformly-matching clip is one merged range"
        );
        assert_eq!(hits[0].start_ms, 0);
        assert_eq!(hits[0].end_ms, 5000);
    }

    #[test]
    fn search_finds_the_matching_moment() {
        // 10 frames at 1s spacing. Frames 5000–7000 point "north" (the chart);
        // the rest point "east". A north query must return the 5000–7000 range.
        let north = vec![0.0f32, 1.0];
        let east = vec![1.0f32, 0.0];
        let frames: Vec<(u64, Vec<f32>)> = (0..10)
            .map(|i| {
                let ms = i * 1000;
                let v = if (5..=7).contains(&i) {
                    north.clone()
                } else {
                    east.clone()
                };
                (ms, v)
            })
            .collect();
        let index = idx(frames);
        let hits = search(&index, &north, 3, 1500).expect("search ok");
        assert!(!hits.is_empty());
        let top = &hits[0];
        assert!(
            top.score > 0.99,
            "north query matches north frames (score {})",
            top.score
        );
        assert_eq!(
            top.start_ms, 5000,
            "range starts at the first matching frame"
        );
        assert_eq!(top.end_ms, 7000, "range ends at the last matching frame");
        assert!((5000..=7000).contains(&top.peak_ms));
    }

    #[test]
    fn search_merges_within_gap_and_splits_beyond() {
        // Matching frames at 1000, 2000 (gap 1000) and a far one at 9000.
        let q = vec![0.0f32, 1.0];
        let m = vec![0.0f32, 1.0]; // matches
        let n = vec![1.0f32, 0.0]; // no
        let frames = vec![
            (1000, m.clone()),
            (2000, m.clone()),
            (3000, n.clone()),
            (4000, n.clone()),
            (9000, m.clone()),
        ];
        let index = idx(frames);
        let hits = search(&index, &q, 5, 1500).expect("ok");
        // 1000–2000 merge into one range; 9000 is its own.
        let merged = hits
            .iter()
            .find(|h| h.start_ms == 1000)
            .expect("merged range");
        assert_eq!(merged.end_ms, 2000);
        assert!(hits.iter().any(|h| h.start_ms == 9000 && h.end_ms == 9000));
    }

    #[test]
    fn dim_mismatch_errors() {
        let index = idx(vec![(0, vec![1.0, 0.0])]);
        assert!(search(&index, &[1.0, 0.0, 0.0], 3, 1000).is_err());
        let empty = EmbeddingIndex {
            schema: default_schema(),
            model: "x".into(),
            dim: 2,
            asset: "a".into(),
            frames: vec![],
            native_runtime: None,
        };
        assert!(search(&empty, &[1.0, 0.0], 3, 1000).is_err());
    }

    #[test]
    fn index_roundtrips_on_disk() {
        let dir = std::env::temp_dir().join(format!("cut-vissearch-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let index = idx(vec![(0, vec![1.0, 0.0]), (1000, vec![0.0, 1.0])]);
        let p = save_index(&dir, &index).expect("save");
        assert!(p.exists());
        let back = load_index(&dir, "a1").expect("load");
        assert_eq!(back.dim, 2);
        assert_eq!(back.frames.len(), 2);
        assert_eq!(back.frames[1].ms, 1000);
        assert!(load_index(&dir, "missing").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checked_index_reports_a_nested_vector_contract_error() {
        let dir = std::env::temp_dir().join(format!("cut-vissearch-nested-{}", std::process::id()));
        let embeddings = dir.join("embeddings");
        std::fs::create_dir_all(&embeddings).unwrap();
        std::fs::write(
            embeddings.join("a1.json"),
            r#"{"schema":"shellx-cut/vissearch/1","model":"siglip","dim":1,"asset":"a1","frames":[{"ms":0,"v":[[0.1,0.2]]}]}"#,
        )
        .unwrap();
        let error = load_index_checked(&dir, "a1").unwrap_err();
        assert!(error.contains("does not match the embedding contract"));
        assert!(load_index(&dir, "a1").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oversized_sparse_visual_index_is_a_cache_miss_before_parse() {
        let dir = tempfile::tempdir().unwrap();
        let embeddings = dir.path().join("embeddings");
        std::fs::create_dir(&embeddings).unwrap();
        let file = std::fs::File::create(embeddings.join("a1.json")).unwrap();
        file.set_len(MAX_VISUAL_INDEX_BYTES + 1).unwrap();

        let error = load_index_checked(dir.path(), "a1").unwrap_err();
        assert!(error.contains("exceeds byte limit"));
        assert!(load_index(dir.path(), "a1").is_none());
    }

    #[test]
    fn visual_index_writer_refuses_oversize_without_replacing_cache() {
        let dir = tempfile::tempdir().unwrap();
        let index = idx(vec![(0, vec![1.0, 0.0])]);
        let bytes = serde_json::to_vec(&index).unwrap();
        let path = save_index_with_limit(dir.path(), &index, bytes.len() as u64).unwrap();
        let before = std::fs::read(&path).unwrap();
        let error = save_index_with_limit(dir.path(), &index, bytes.len() as u64 - 1).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[cfg(unix)]
    #[test]
    fn linked_visual_index_is_a_cache_miss() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let embeddings = dir.path().join("embeddings");
        std::fs::create_dir(&embeddings).unwrap();
        let outside = dir.path().join("outside.json");
        std::fs::write(
            &outside,
            serde_json::to_vec(&idx(vec![(0, vec![1.0])])).unwrap(),
        )
        .unwrap();
        symlink(&outside, embeddings.join("a1.json")).unwrap();

        assert!(load_index(dir.path(), "a1").is_none());
        assert!(load_index_checked(dir.path(), "a1")
            .unwrap_err()
            .contains("not a regular file"));
    }

    #[cfg(unix)]
    #[test]
    fn visual_index_writer_refuses_linked_parent_and_leaf_without_touching_outside() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let sentinel = outside.join("a1.json");
        std::fs::write(&sentinel, b"outside-sentinel").unwrap();
        let embeddings = dir.path().join("embeddings");
        symlink(&outside, &embeddings).unwrap();
        assert!(save_index(dir.path(), &idx(vec![(0, vec![1.0])])).is_err());
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside-sentinel");

        std::fs::remove_file(&embeddings).unwrap();
        std::fs::create_dir(&embeddings).unwrap();
        symlink(&sentinel, embeddings.join("a1.json")).unwrap();
        assert!(save_index(dir.path(), &idx(vec![(0, vec![1.0])])).is_err());
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside-sentinel");
        std::fs::remove_file(embeddings.join("a1.json")).unwrap();
        let path = save_index(dir.path(), &idx(vec![(0, vec![1.0])])).unwrap();
        assert_eq!(
            load_index_checked(dir.path(), "a1").unwrap().frames.len(),
            1
        );
        assert!(path.is_file());
        save_index(dir.path(), &idx(vec![(0, vec![1.0]), (1000, vec![0.0])])).unwrap();
        assert_eq!(
            load_index_checked(dir.path(), "a1").unwrap().frames.len(),
            2
        );
    }
}

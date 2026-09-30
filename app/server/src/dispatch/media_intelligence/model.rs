use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

// The index stores short excerpts rather than media; a 128 MiB JSON file can
// hold hundreds of thousands of citations. Receipts can cover long footage,
// but their authority hash must never require loading them whole.
pub(super) const MAX_EVIDENCE_INDEX_BYTES: u64 = 128 * 1024 * 1024;
pub(super) const MAX_EVIDENCE_RECEIPT_BYTES: u64 = 64 * 1024 * 1024;

pub(super) const INDEX_SCHEMA: &str = "shellx-cut/media-evidence-index/1";
pub(super) const SEARCH_SCHEMA: &str = "shellx-cut/evidence-search-result/1";
pub(super) const ALL_KINDS: [&str; 6] = [
    "transcript",
    "visual",
    "scene",
    "beat",
    "marker",
    "metadata",
];

pub(super) fn generator_identity() -> String {
    format!("shellx-cut/{}", env!("CARGO_PKG_VERSION"))
}

#[derive(Debug, Clone)]
pub(super) struct ProjectSnapshot {
    pub(super) dir: PathBuf,
    pub(super) revision: Option<String>,
    pub(super) project: cut_core::Project,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SourceBinding {
    pub(super) asset_id: String,
    pub(super) asset_hash: String,
    pub(super) available: bool,
    pub(super) video: bool,
    pub(super) audio: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) transcript_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) perception_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) visual_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct EvidenceEntry {
    pub(super) evidence_id: String,
    pub(super) asset_id: String,
    pub(super) kind: String,
    pub(super) source_start_ms: u64,
    pub(super) source_end_ms: u64,
    pub(super) anchor_ms: u64,
    pub(super) excerpt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) speaker: Option<String>,
    pub(super) provenance_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) sequence_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) timeline_anchor_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) marker_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct MediaEvidenceIndex {
    pub(super) schema: String,
    pub(super) generator: String,
    pub(super) index_id: String,
    pub(super) project_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) project_revision: Option<String>,
    pub(super) selected_kinds: Vec<String>,
    pub(super) bindings: Vec<SourceBinding>,
    #[serde(default)]
    pub(super) coverage: BTreeMap<String, Vec<String>>,
    pub(super) entries: Vec<EvidenceEntry>,
}

pub(super) fn digest_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub(super) fn opaque_id(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    format!("ev_{}", &format!("{:x}", hash.finalize())[..24])
}

pub(super) fn index_path(project_dir: &Path) -> PathBuf {
    project_dir.join("indexes").join("media-evidence-v1.json")
}

pub(super) fn load_index(project_dir: &Path) -> Option<MediaEvidenceIndex> {
    crate::output_paths::existing_plain_project_relative_dir(project_dir, Path::new("indexes"))
        .ok()?;
    let bytes =
        crate::vissearch::read_bounded_file(&index_path(project_dir), MAX_EVIDENCE_INDEX_BYTES)
            .ok()?;
    let index: MediaEvidenceIndex = serde_json::from_slice(&bytes).ok()?;
    (index.schema == INDEX_SCHEMA).then_some(index)
}

fn relative_receipt_hash(project_dir: &Path, relative: Option<&str>) -> Option<String> {
    let relative = relative?;
    let path = resolve_existing_project_file(
        project_dir,
        relative,
        "media evidence receipt",
        "re-run the matching analysis so Cut can publish a project-local receipt",
    )
    .ok()?;
    hash_bounded_file(&path, MAX_EVIDENCE_RECEIPT_BYTES).ok()
}

fn hash_bounded_file(path: &Path, max_bytes: u64) -> io::Result<String> {
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a regular file",
        ));
    }
    let mut file = std::fs::File::open(path)?;
    if file.metadata()?.len() > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds byte limit",
        ));
    }
    let mut digest = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        if bytes > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "file exceeds byte limit",
            ));
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn source_available(project_dir: &Path, source: &str) -> bool {
    let path = PathBuf::from(source);
    let resolved = if path.is_relative() {
        project_dir.join(path)
    } else {
        path
    };
    resolved.is_file()
}

pub(super) fn current_bindings(
    snapshot: &ProjectSnapshot,
    selected: Option<&[String]>,
) -> Result<Vec<SourceBinding>, CutError> {
    // In prepared mode, the visual cache is only authority when the exact
    // current runtime accepts its model and provenance. This is also the
    // status/rebuild boundary, so a malformed supplied context fails before
    // reporting a stale cache as ready.
    let visual_runtime = crate::vissearch::visual_cache_runtime()?;
    current_bindings_for_visual_runtime(snapshot, selected, &visual_runtime)
}

pub(super) fn current_bindings_for_visual_runtime(
    snapshot: &ProjectSnapshot,
    selected: Option<&[String]>,
    visual_runtime: &crate::vissearch::VisualCacheRuntime,
) -> Result<Vec<SourceBinding>, CutError> {
    let selected = selected.map(|ids| ids.iter().collect::<std::collections::BTreeSet<_>>());
    if let Some(ids) = &selected {
        for id in ids {
            if !snapshot.project.assets.contains_key(id.as_str()) {
                return Err(CutError::new(
                    error_codes::NOT_FOUND,
                    format!("unknown asset '{id}'"),
                    "pass asset ids from project.state, or omit asset_ids",
                ));
            }
        }
    }
    let mut bindings = Vec::new();
    for (id, asset) in &snapshot.project.assets {
        if selected.as_ref().is_some_and(|ids| !ids.contains(id)) {
            continue;
        }
        let probe = asset.probe.as_ref();
        let kind = probe
            .and_then(|value| value.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let video = kind == "video"
            || probe
                .and_then(|value| value.get("has_video"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
        let audio = kind == "audio"
            || probe
                .and_then(|value| value.get("has_audio"))
                .and_then(Value::as_bool)
                .unwrap_or(video);
        bindings.push(SourceBinding {
            asset_id: id.clone(),
            asset_hash: asset.hash.clone(),
            available: source_available(&snapshot.dir, &asset.path),
            video,
            audio,
            transcript_sha256: relative_receipt_hash(&snapshot.dir, asset.transcript.as_deref()),
            perception_sha256: relative_receipt_hash(&snapshot.dir, asset.perception.as_deref()),
            visual_sha256: visual_index_sha256(&snapshot.dir, id, visual_runtime),
        });
    }
    Ok(bindings)
}

fn visual_index_sha256(
    project_dir: &Path,
    asset_id: &str,
    visual_runtime: &crate::vissearch::VisualCacheRuntime,
) -> Option<String> {
    match visual_runtime {
        crate::vissearch::VisualCacheRuntime::Legacy => {
            crate::vissearch::load_index_for_runtime(project_dir, asset_id, None)
        }
        crate::vissearch::VisualCacheRuntime::Prepared(runtime) => {
            crate::vissearch::load_index_for_runtime(project_dir, asset_id, Some(runtime))
        }
        crate::vissearch::VisualCacheRuntime::Unavailable => None,
    }?;
    hash_bounded_file(
        &crate::vissearch::index_path(project_dir, asset_id),
        crate::vissearch::MAX_VISUAL_INDEX_BYTES,
    )
    .ok()
}

pub(super) fn binding_map(bindings: &[SourceBinding]) -> BTreeMap<&str, &SourceBinding> {
    bindings
        .iter()
        .map(|binding| (binding.asset_id.as_str(), binding))
        .collect()
}

pub(super) fn authority_hash<'a>(binding: &'a SourceBinding, kind: &str) -> Option<&'a str> {
    match kind {
        "transcript" => binding.transcript_sha256.as_deref(),
        "visual" => binding.visual_sha256.as_deref(),
        "scene" | "beat" => binding.perception_sha256.as_deref(),
        "marker" | "metadata" => Some(&binding.asset_hash),
        _ => None,
    }
}

pub(super) fn finalize_index(
    mut index: MediaEvidenceIndex,
) -> Result<MediaEvidenceIndex, CutError> {
    index.index_id.clear();
    let bytes = serde_json::to_vec(&index)?;
    index.index_id = format!("idx_{}", &digest_bytes(&bytes)[7..31]);
    Ok(index)
}

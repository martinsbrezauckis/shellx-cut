use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
    let bytes = std::fs::read(index_path(project_dir)).ok()?;
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
    std::fs::read(path).ok().map(|bytes| digest_bytes(&bytes))
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
        let visual_path = crate::vissearch::index_path(&snapshot.dir, id);
        bindings.push(SourceBinding {
            asset_id: id.clone(),
            asset_hash: asset.hash.clone(),
            available: source_available(&snapshot.dir, &asset.path),
            video,
            audio,
            transcript_sha256: relative_receipt_hash(&snapshot.dir, asset.transcript.as_deref()),
            perception_sha256: relative_receipt_hash(&snapshot.dir, asset.perception.as_deref()),
            visual_sha256: std::fs::read(visual_path)
                .ok()
                .map(|bytes| digest_bytes(&bytes)),
        });
    }
    Ok(bindings)
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

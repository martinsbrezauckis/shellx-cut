//! Durable generated-media identity, registered references, and history reads.
//!
//! `assets.generate` owns provider execution. This module owns the smaller,
//! provider-independent contract around it: content-addressed variant ids,
//! project-registered reference validation/copying, and a path-light history
//! projection over immutable generated sources plus their provenance sidecars.

use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::Read;

const MAX_REFERENCES: usize = 4;
const MAX_VARIATION_LEN: usize = 128;
const MAX_PROVENANCE_BYTES: u64 = 256 * 1024;
const MAX_PROMPT_LEN: usize = 32 * 1024;
const MAX_MODEL_LEN: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GenerationReference {
    pub asset_id: String,
    pub content_hash: String,
    pub kind: String,
    source_path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GenerationProvenanceIssue {
    Missing,
    Unsafe,
    Invalid,
}

impl GenerationProvenanceIssue {
    fn integrity(self) -> &'static str {
        match self {
            Self::Missing => "missing_provenance",
            Self::Unsafe => "unsafe_source",
            Self::Invalid => "invalid_provenance",
        }
    }

    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Unsafe => "a symbolic link or non-regular file",
            Self::Invalid => "invalid, oversized, or internally inconsistent",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GenerationProvenance {
    pub schema: String,
    pub generation_id: String,
    pub family_id: String,
    pub provider: String,
    pub kind: String,
    pub model: Option<String>,
    pub prompt: String,
    pub variation: Option<String>,
    pub references: Vec<GenerationReference>,
    pub created_at_ms: Option<u64>,
    pub cost_note: String,
    pub content_hash: String,
}

fn hash_fields<'a>(values: impl IntoIterator<Item = &'a str>) -> String {
    let mut hash = Sha256::new();
    for value in values {
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    }
    format!("{:x}", hash.finalize())[..24].to_string()
}

/// Preserve the original no-reference generation id, while making references
/// part of a stable family identity when present.
pub(crate) fn generation_family_id(
    provider: &str,
    kind: &str,
    model: Option<&str>,
    prompt: &str,
    references: &[GenerationReference],
) -> String {
    if references.is_empty() {
        return hash_fields([provider, kind, model.unwrap_or(""), prompt]);
    }
    let mut fields = vec![provider, kind, model.unwrap_or(""), prompt];
    for reference in references {
        fields.push(reference.content_hash.as_str());
        fields.push(reference.kind.as_str());
    }
    hash_fields(fields)
}

pub(crate) fn normalize_variation(value: Option<&str>) -> Result<Option<String>, CutError> {
    let Some(raw) = value else { return Ok(None) };
    let value = raw.trim();
    if value.is_empty() || value.len() > MAX_VARIATION_LEN || value.chars().any(char::is_control) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "variation must be a non-empty value of at most 128 bytes",
            "use a short stable take label such as take-2",
        ));
    }
    Ok(Some(value.to_string()))
}

/// The base take remains byte-for-byte compatible with the original id. An
/// explicit variation creates a distinct immutable child within that family.
pub(crate) fn generation_id(family_id: &str, variation: Option<&str>) -> String {
    match variation {
        Some(value) => hash_fields(["shellx-cut/generated-variation/1", family_id, value]),
        None => family_id.to_string(),
    }
}

fn valid_content_hash(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn optional_bounded_string(document: &Value, key: &str, max_len: usize) -> Option<Option<String>> {
    match document.get(key) {
        None | Some(Value::Null) => Some(None),
        Some(Value::String(value)) if value.chars().count() <= max_len => Some(Some(value.clone())),
        _ => None,
    }
}

/// Read and fully validate a generated-media sidecar without following links or
/// accepting unbounded data. Identity is recomputed from the recorded request,
/// so a sidecar cannot describe a different request while retaining the file id.
pub(crate) fn read_generation_provenance(
    path: &Path,
) -> Result<GenerationProvenance, GenerationProvenanceIssue> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(GenerationProvenanceIssue::Missing)
        }
        Err(_) => return Err(GenerationProvenanceIssue::Invalid),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(GenerationProvenanceIssue::Unsafe);
    }
    if metadata.len() > MAX_PROVENANCE_BYTES {
        return Err(GenerationProvenanceIssue::Invalid);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    std::fs::File::open(path)
        .map_err(|_| GenerationProvenanceIssue::Invalid)?
        .take(MAX_PROVENANCE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| GenerationProvenanceIssue::Invalid)?;
    if bytes.len() as u64 > MAX_PROVENANCE_BYTES {
        return Err(GenerationProvenanceIssue::Invalid);
    }
    let document: Value =
        serde_json::from_slice(&bytes).map_err(|_| GenerationProvenanceIssue::Invalid)?;
    let object = document
        .as_object()
        .ok_or(GenerationProvenanceIssue::Invalid)?;
    let required_string = |key: &str, max_len: usize| {
        object
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.chars().count() <= max_len)
            .map(str::to_string)
            .ok_or(GenerationProvenanceIssue::Invalid)
    };
    let schema = required_string("schema", 64)?;
    if !matches!(
        schema.as_str(),
        "shellx-cut/generated-asset/1" | "shellx-cut/generated-asset/2"
    ) {
        return Err(GenerationProvenanceIssue::Invalid);
    }
    let recorded_generation_id = required_string("generation_id", 64)?;
    let provider = required_string("provider", 16)?;
    let kind = required_string("kind", 16)?;
    if !matches!(provider.as_str(), "codex" | "grok") || !matches!(kind.as_str(), "image" | "video")
    {
        return Err(GenerationProvenanceIssue::Invalid);
    }
    let prompt = required_string("prompt", MAX_PROMPT_LEN)?;
    if prompt.trim() != prompt {
        return Err(GenerationProvenanceIssue::Invalid);
    }
    let model = optional_bounded_string(&document, "model", MAX_MODEL_LEN)
        .ok_or(GenerationProvenanceIssue::Invalid)?;
    let content_hash = required_string("content_hash", 80)?;
    if !valid_content_hash(&content_hash) {
        return Err(GenerationProvenanceIssue::Invalid);
    }

    let (family_id, variation, references, created_at_ms) = if schema.ends_with("/2") {
        let recorded_family_id = required_string("family_id", 64)?;
        let variation = optional_bounded_string(&document, "variation", MAX_VARIATION_LEN)
            .ok_or(GenerationProvenanceIssue::Invalid)?;
        let variation = match variation {
            Some(raw) => {
                let normalized = normalize_variation(Some(&raw))
                    .map_err(|_| GenerationProvenanceIssue::Invalid)?;
                if normalized.as_deref() != Some(raw.as_str()) {
                    return Err(GenerationProvenanceIssue::Invalid);
                }
                normalized
            }
            None => None,
        };
        let entries = object
            .get("references")
            .and_then(Value::as_array)
            .filter(|entries| entries.len() <= MAX_REFERENCES)
            .ok_or(GenerationProvenanceIssue::Invalid)?;
        let mut seen = BTreeSet::new();
        let mut references = Vec::with_capacity(entries.len());
        for entry in entries {
            let asset_id = entry
                .get("asset_id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty() && value.len() <= 256)
                .ok_or(GenerationProvenanceIssue::Invalid)?;
            let reference_hash = entry
                .get("content_hash")
                .and_then(Value::as_str)
                .filter(|value| valid_content_hash(value))
                .ok_or(GenerationProvenanceIssue::Invalid)?;
            let reference_kind = entry
                .get("kind")
                .and_then(Value::as_str)
                .filter(|value| matches!(*value, "image" | "video"))
                .ok_or(GenerationProvenanceIssue::Invalid)?;
            if !seen.insert(asset_id) {
                return Err(GenerationProvenanceIssue::Invalid);
            }
            references.push(GenerationReference {
                asset_id: asset_id.to_string(),
                content_hash: reference_hash.to_string(),
                kind: reference_kind.to_string(),
                source_path: String::new(),
            });
        }
        let computed_family_id =
            generation_family_id(&provider, &kind, model.as_deref(), &prompt, &references);
        if recorded_family_id != computed_family_id {
            return Err(GenerationProvenanceIssue::Invalid);
        }
        let created_at_ms = match object.get("created_at_ms") {
            None | Some(Value::Null) => None,
            Some(value) => Some(value.as_u64().ok_or(GenerationProvenanceIssue::Invalid)?),
        };
        (computed_family_id, variation, references, created_at_ms)
    } else {
        (
            generation_family_id(&provider, &kind, model.as_deref(), &prompt, &[]),
            None,
            Vec::new(),
            None,
        )
    };
    if recorded_generation_id != generation_id(&family_id, variation.as_deref()) {
        return Err(GenerationProvenanceIssue::Invalid);
    }
    let cost_note = object
        .get("cost_note")
        .and_then(Value::as_str)
        .filter(|value| value.len() <= 512)
        .unwrap_or("provider price was not recorded; check the provider account")
        .to_string();
    Ok(GenerationProvenance {
        schema,
        generation_id: recorded_generation_id,
        family_id,
        provider,
        kind,
        model,
        prompt,
        variation,
        references,
        created_at_ms,
        cost_note,
        content_hash,
    })
}

pub(crate) fn resolve_generation_references(
    project: &cut_core::Project,
    ids: &[String],
) -> Result<Vec<GenerationReference>, CutError> {
    if ids.len() > MAX_REFERENCES {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("generation accepts at most {MAX_REFERENCES} reference assets"),
            "remove extra references and submit again",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut resolved = Vec::with_capacity(ids.len());
    for id in ids {
        if !seen.insert(id.as_str()) {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("reference asset '{id}' was supplied more than once"),
                "each registered project asset may be referenced once",
            ));
        }
        let asset = project.assets.get(id).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("reference asset '{id}' is not registered in the open project"),
                "project.state lists the asset ids that may be used as references",
            )
        })?;
        let kind = asset
            .probe
            .as_ref()
            .and_then(|probe| probe.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if !matches!(kind, "image" | "video") {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("reference asset '{id}' is not a probed image or video"),
                "probe the asset first and select visual reference media",
            ));
        }
        resolved.push(GenerationReference {
            asset_id: id.clone(),
            content_hash: asset.hash.clone(),
            kind: kind.to_string(),
            source_path: asset.path.clone(),
        });
    }
    Ok(resolved)
}

/// Re-resolve and hash each registered source immediately before the provider
/// run, then copy it under a neutral scratch filename. Provider prompts never
/// receive caller-supplied or arbitrary filesystem paths.
fn generation_reference_source(
    project: &cut_core::Project,
    expected: &GenerationReference,
    project_dir: &Path,
) -> Result<PathBuf, CutError> {
    let asset = project.assets.get(&expected.asset_id).ok_or_else(|| {
        CutError::new(
            error_codes::CONFLICT,
            format!(
                "reference asset '{}' was removed after generation was submitted",
                expected.asset_id
            ),
            "review the open project and submit the generation again",
        )
    })?;
    if asset.hash != expected.content_hash || asset.path != expected.source_path {
        return Err(CutError::new(
            error_codes::CONFLICT,
            format!(
                "reference asset '{}' changed after generation was submitted",
                expected.asset_id
            ),
            "review the relinked or replaced asset and submit a new generation",
        ));
    }
    let source = PathBuf::from(&asset.path);
    let source = if source.is_absolute() {
        source
    } else {
        project_dir.join(source)
    };
    if !source.is_file() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("reference asset '{}' is offline", expected.asset_id),
            "relink the source before generating from it",
        ));
    }
    if std::fs::symlink_metadata(&source)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "reference asset '{}' resolves through a symbolic link",
                expected.asset_id
            ),
            "relink or import the real source file before using it as a generation reference",
        ));
    }
    Ok(source)
}

pub(crate) fn validate_generation_references(
    project: &cut_core::Project,
    references: &[GenerationReference],
    project_dir: &Path,
) -> Result<(), CutError> {
    for expected in references {
        let source = generation_reference_source(project, expected, project_dir)?;
        if cut_core::hash_file(&source)? != expected.content_hash {
            return Err(CutError::new(
                error_codes::CONFLICT,
                format!("reference asset '{}' changed on disk", expected.asset_id),
                "relink or re-import the changed source before using it as a reference",
            ));
        }
    }
    Ok(())
}

pub(crate) fn copy_generation_references(
    project: &cut_core::Project,
    references: &[GenerationReference],
    project_dir: &Path,
    workspace: &Path,
) -> Result<Vec<String>, CutError> {
    let mut copied = Vec::with_capacity(references.len());
    for (index, expected) in references.iter().enumerate() {
        let source = generation_reference_source(project, expected, project_dir)?;
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| value.len() <= 8 && value.chars().all(|ch| ch.is_ascii_alphanumeric()))
            .unwrap_or(if expected.kind == "video" {
                "mp4"
            } else {
                "png"
            });
        let destination = workspace.join(format!("reference-{}.{}", index + 1, extension));
        std::fs::copy(&source, &destination).map_err(|error| {
            CutError::new(
                error_codes::IO,
                format!(
                    "copy reference asset '{}' into generation workspace",
                    expected.asset_id
                ),
                error.to_string(),
            )
        })?;
        let copied_hash = cut_core::hash_file(&destination)?;
        if copied_hash != expected.content_hash {
            let _ = std::fs::remove_file(&destination);
            return Err(CutError::new(
                error_codes::CONFLICT,
                format!("reference asset '{}' changed on disk", expected.asset_id),
                "relink or re-import the changed source before using it as a reference",
            ));
        }
        copied.push(destination.display().to_string());
    }
    Ok(copied)
}

fn generation_sidecar_record(
    project: &cut_core::Project,
    asset_id: &str,
    asset: &cut_core::Asset,
    media_path: &Path,
) -> Value {
    let sidecar = media_path.with_extension("json");
    let stem = media_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let media_is_symlink = std::fs::symlink_metadata(media_path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false);
    let provenance = read_generation_provenance(&sidecar);
    let mut integrity = if media_is_symlink {
        "unsafe_source"
    } else {
        match provenance.as_ref() {
            Ok(_) => "verified",
            Err(issue) => issue.integrity(),
        }
    };
    if integrity == "verified" {
        if let Ok(document) = provenance.as_ref() {
            if document.generation_id != stem || document.content_hash != asset.hash {
                integrity = "provenance_mismatch";
            }
        }
    }
    if integrity == "verified" {
        match cut_core::hash_file(media_path) {
            Ok(actual) if actual == asset.hash => {}
            Ok(_) => integrity = "changed",
            Err(_) => integrity = "offline",
        }
    }
    let reference_asset_ids = provenance
        .as_ref()
        .map(|document| {
            document
                .references
                .iter()
                .filter(|reference| project.assets.contains_key(&reference.asset_id))
                .map(|reference| reference.asset_id.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let document = provenance.as_ref().ok();
    let generation_id = document
        .map(|document| document.generation_id.as_str())
        .unwrap_or(stem);
    json!({
        "asset_id": asset_id,
        "generation_id": generation_id,
        "family_id": document.map(|document| document.family_id.as_str()).unwrap_or(generation_id),
        "provider": document.map(|document| document.provider.as_str()),
        "kind": document.map(|document| document.kind.as_str()).or_else(|| asset.probe.as_ref().and_then(|probe| probe.get("kind")).and_then(Value::as_str)),
        "model": document.and_then(|document| document.model.as_deref()),
        "prompt": document.map(|document| document.prompt.as_str()).unwrap_or(""),
        "variation": document.and_then(|document| document.variation.as_deref()),
        "reference_asset_ids": reference_asset_ids,
        "created_at_ms": document.and_then(|document| document.created_at_ms),
        "cost_usd": Value::Null,
        "cost_note": document.map(|document| document.cost_note.as_str()).unwrap_or("provider price was not recorded; check the provider account"),
        "content_hash": asset.hash,
        "provenance_schema": document.map(|document| document.schema.as_str()),
        "integrity": integrity,
    })
}

/// Read project-bound generated media without exposing source or provenance
/// paths. Sidecars are checked against both the registered asset hash and the
/// current immutable file before an item is reported as verified.
pub(super) async fn assets_generated_list(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        kind: Option<String>,
        limit: Option<usize>,
    }
    let args: Args = parse_args(args)?;
    if let Some(kind) = args.kind.as_deref() {
        if !matches!(kind, "image" | "video") {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unsupported generated-media kind '{kind}'"),
                "kind is image | video",
            ));
        }
    }
    let limit = args.limit.unwrap_or(100).clamp(1, 200);
    let (project, dir) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        (store.project.clone(), store.dir.clone())
    };
    let generated_dir = dir.join("assets/generated");
    if !generated_dir.is_dir() {
        return Ok(VerbResult::ok(
            json!({"items": [], "total": 0, "verified": 0}),
        ));
    }
    let generated_root = generated_dir.canonicalize().map_err(|error| {
        CutError::new(
            error_codes::IO,
            "open generated-media history",
            error.to_string(),
        )
    })?;
    let mut items = Vec::new();
    for (asset_id, asset) in &project.assets {
        let path = PathBuf::from(&asset.path);
        let path = if path.is_absolute() {
            path
        } else {
            dir.join(path)
        };
        let parent = match path.parent().and_then(|parent| parent.canonicalize().ok()) {
            Some(parent) => parent,
            None => continue,
        };
        if parent != generated_root {
            continue;
        }
        let record = generation_sidecar_record(&project, asset_id, asset, &path);
        if args
            .kind
            .as_deref()
            .is_some_and(|kind| record["kind"] != kind)
        {
            continue;
        }
        items.push(record);
    }
    items.sort_by(|left, right| {
        right["created_at_ms"]
            .as_u64()
            .unwrap_or(0)
            .cmp(&left["created_at_ms"].as_u64().unwrap_or(0))
            .then_with(|| {
                right["generation_id"]
                    .as_str()
                    .cmp(&left["generation_id"].as_str())
            })
    });
    let total = items.len();
    let verified = items
        .iter()
        .filter(|item| item["integrity"] == "verified")
        .count();
    items.truncate(limit);
    Ok(VerbResult::ok(
        json!({"items": items, "total": total, "verified": verified}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_generation_identity_is_compatible_and_variations_are_distinct() {
        let family = generation_family_id("codex", "image", None, "blue card", &[]);
        assert_eq!(family, generation_id(&family, None));
        assert_ne!(family, generation_id(&family, Some("take-2")));
        assert_ne!(
            generation_id(&family, Some("take-2")),
            generation_id(&family, Some("take-3"))
        );
    }

    #[test]
    fn variation_rejects_blank_control_and_oversized_values() {
        assert!(normalize_variation(Some(" ")).is_err());
        assert!(normalize_variation(Some("take\n2")).is_err());
        assert!(normalize_variation(Some(&"x".repeat(129))).is_err());
        assert_eq!(
            normalize_variation(Some(" take-2 ")).unwrap().as_deref(),
            Some("take-2")
        );
    }

    #[test]
    fn provenance_is_bounded_and_identity_checked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("generated.json");
        let references = vec![GenerationReference {
            asset_id: "asset-reference".into(),
            content_hash: format!("sha256:{}", "a".repeat(64)),
            kind: "image".into(),
            source_path: String::new(),
        }];
        let family_id = generation_family_id("codex", "image", None, "blue card", &references);
        let generation_id = generation_id(&family_id, Some("take-2"));
        let mut document = json!({
            "schema": "shellx-cut/generated-asset/2",
            "generation_id": generation_id,
            "family_id": family_id,
            "provider": "codex",
            "kind": "image",
            "model": null,
            "prompt": "blue card",
            "variation": "take-2",
            "references": [{
                "asset_id": references[0].asset_id.clone(),
                "content_hash": references[0].content_hash.clone(),
                "kind": references[0].kind.clone(),
            }],
            "created_at_ms": 1234,
            "cost_note": "price unavailable",
            "content_hash": format!("sha256:{}", "b".repeat(64)),
        });
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        let parsed = read_generation_provenance(&path).unwrap();
        assert_eq!(parsed.family_id, family_id);
        assert_eq!(parsed.variation.as_deref(), Some("take-2"));
        assert_eq!(parsed.references.len(), 1);

        document["prompt"] = json!("different request");
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        assert_eq!(
            read_generation_provenance(&path),
            Err(GenerationProvenanceIssue::Invalid)
        );

        std::fs::write(&path, vec![b' '; (MAX_PROVENANCE_BYTES + 1) as usize]).unwrap();
        assert_eq!(
            read_generation_provenance(&path),
            Err(GenerationProvenanceIssue::Invalid)
        );
    }

    #[test]
    fn legacy_provenance_remains_readable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.json");
        let generation_id = generation_family_id("codex", "image", None, "blue card", &[]);
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({
                "schema": "shellx-cut/generated-asset/1",
                "generation_id": generation_id,
                "provider": "codex",
                "kind": "image",
                "model": null,
                "prompt": "blue card",
                "cost_note": "price unavailable",
                "content_hash": format!("sha256:{}", "c".repeat(64)),
            }))
            .unwrap(),
        )
        .unwrap();
        let parsed = read_generation_provenance(&path).unwrap();
        assert_eq!(parsed.schema, "shellx-cut/generated-asset/1");
        assert_eq!(parsed.family_id, generation_id);
        assert!(parsed.references.is_empty());
        assert_eq!(parsed.created_at_ms, None);
    }
}

//! Stored and candidate probe facts, kept separate from review admission.

use super::{CandidateMetadata, StoredMetadata};
use cut_core::{error_codes, CutError};
use serde_json::Value;
use std::{fs, path::Path};

#[derive(Debug, Clone, Default)]
pub(super) struct ProbeFacts {
    pub(super) kind: Option<String>,
    pub(super) duration_ms: Option<u64>,
    pub(super) width: Option<u32>,
    pub(super) height: Option<u32>,
    pub(super) format: Option<String>,
    pub(super) codecs: Option<String>,
}

pub(in super::super) fn stored_metadata(probe: Option<&Value>) -> StoredMetadata {
    let facts = probe.map(ProbeFacts::from_json).unwrap_or_default();
    let raw_size = probe.and_then(|value| value.pointer("/raw/format/size").and_then(raw_u64));
    StoredMetadata {
        probe_available: probe.is_some(),
        raw_size,
        facts,
    }
}

pub(in super::super) fn candidate_metadata(
    path: &Path,
    root: &Path,
) -> Result<CandidateMetadata, CutError> {
    let before = fs::symlink_metadata(path)?;
    if before.file_type().is_symlink()
        || !before.is_file()
        || !path.canonicalize()?.starts_with(root)
    {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "recovery candidate became an unsafe path during metadata review",
            "the bounded recovery scan does not follow symlinks or path escapes",
        ));
    }
    let bytes = before.len();
    // A recovery folder may include non-media files. They remain full-hash
    // candidates but cannot provide a metadata review hint without a probe.
    let facts = cut_media::probe(path).ok().map(|probe| {
        ProbeFacts::from_json(&serde_json::to_value(probe).expect("probe serializes"))
    });
    let after = fs::symlink_metadata(path)?;
    if after.file_type().is_symlink()
        || !after.is_file()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "recovery candidate changed during metadata review",
            "run the preview again after file copying finishes",
        ));
    }
    Ok(CandidateMetadata { bytes, facts })
}

impl ProbeFacts {
    pub(super) fn from_json(probe: &Value) -> Self {
        let raw = probe.get("raw");
        let format = raw
            .and_then(|raw| raw.pointer("/format/format_name"))
            .and_then(Value::as_str)
            .and_then(normalize_format)
            .or_else(|| {
                probe
                    .get("format")
                    .and_then(Value::as_str)
                    .and_then(|value| value.split_once('/').map(|(format, _)| format))
                    .and_then(normalize_format)
            });
        let codecs = raw
            .and_then(|raw| raw.get("streams"))
            .and_then(Value::as_array)
            .map(|streams| {
                streams
                    .iter()
                    .filter_map(|stream| stream.get("codec_name").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("+")
            })
            .and_then(|value| normalize_codecs(&value))
            .or_else(|| {
                probe
                    .get("format")
                    .and_then(Value::as_str)
                    .and_then(|value| value.split_once('/').map(|(_, codecs)| codecs))
                    .and_then(normalize_codecs)
            });
        Self {
            kind: probe.get("kind").and_then(Value::as_str).map(str::to_owned),
            duration_ms: probe.get("duration_ms").and_then(Value::as_u64),
            width: probe
                .get("width")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok()),
            height: probe
                .get("height")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok()),
            format,
            codecs,
        }
    }
}

fn raw_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn normalize_format(value: &str) -> Option<String> {
    normalize_list(value, ',')
}

fn normalize_codecs(value: &str) -> Option<String> {
    normalize_list(value, '+')
}

fn normalize_list(value: &str, separator: char) -> Option<String> {
    let mut parts = value
        .split(separator)
        .map(|part| part.trim().to_ascii_lowercase())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    parts.sort();
    parts.dedup();
    (!parts.is_empty()).then(|| parts.join("+"))
}

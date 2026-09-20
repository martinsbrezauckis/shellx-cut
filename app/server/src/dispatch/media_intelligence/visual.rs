use super::model::*;
use super::search::Candidate;
use super::status::index_kind_current;
use super::*;
use std::collections::BTreeSet;

fn embed_text(query: &str) -> Result<(Vec<f32>, crate::vissearch::Runtime), CutError> {
    let runtime = crate::vissearch::runtime()?.ok_or_else(|| {
        CutError::new(
            error_codes::UNIMPLEMENTED,
            "visual query encoder is unavailable",
            "non-visual evidence remains searchable; install the local captions/search runtime for visuals",
        )
    })?;
    let mut command = std::process::Command::new(&runtime.python);
    runtime.configure_command(&mut command);
    command.arg("--embed-text").arg(query);
    let output = run_bounded_foreground_command(&mut command, "media intelligence text encoder")?;
    if !output.status.success() {
        return Err(CutError::new(
            error_codes::SIDECAR,
            "visual query encoder failed",
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .last()
                .unwrap_or("")
                .to_string(),
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with('{'))
        .unwrap_or("");
    let value: Value = serde_json::from_str(line).map_err(|error| {
        CutError::new(
            error_codes::SIDECAR,
            "visual query encoder returned invalid JSON",
            error.to_string(),
        )
    })?;
    let vector = value
        .get("v")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_f64().map(|value| value as f32))
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty())
        .ok_or_else(|| {
            CutError::new(
                error_codes::SIDECAR,
                "visual query encoder returned no vector",
                "expected a non-empty v array",
            )
        })?;
    Ok((vector, runtime))
}

pub(super) fn visual_candidates(
    snapshot: &ProjectSnapshot,
    index: &MediaEvidenceIndex,
    current: &[SourceBinding],
    selected: Option<&BTreeSet<String>>,
    query: &str,
    limit: usize,
) -> Result<Vec<Candidate>, CutError> {
    let eligible = current
        .iter()
        .filter(|binding| {
            binding.visual_sha256.is_some()
                && index_kind_current(index, binding, "visual")
                && selected.is_none_or(|ids| ids.contains(&binding.asset_id))
        })
        .cloned()
        .collect::<Vec<_>>();
    if eligible.is_empty() {
        return Ok(Vec::new());
    }
    let (vector, runtime) = embed_text(query)?;
    let mut candidates = Vec::new();
    for binding in eligible {
        let Some(visual) = crate::vissearch::load_index_for_runtime(
            &snapshot.dir,
            &binding.asset_id,
            Some(&runtime),
        ) else {
            continue;
        };
        for hit in crate::vissearch::search(&visual, &vector, limit, 2_000).map_err(|error| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "visual evidence search failed",
                error,
            )
        })? {
            let provenance = binding.visual_sha256.as_deref().unwrap_or("");
            let excerpt = format!("Visual match for “{}”", query.trim());
            let evidence_id = opaque_id(&[
                "visual",
                &binding.asset_id,
                &hit.start_ms.to_string(),
                &hit.end_ms.to_string(),
                provenance,
                query,
            ]);
            candidates.push(Candidate {
                entry: EvidenceEntry {
                    evidence_id,
                    asset_id: binding.asset_id.clone(),
                    kind: "visual".into(),
                    source_start_ms: hit.start_ms,
                    source_end_ms: hit.end_ms.max(hit.start_ms.saturating_add(1)),
                    anchor_ms: hit.peak_ms,
                    excerpt,
                    speaker: None,
                    provenance_sha256: provenance.into(),
                    sequence_id: None,
                    timeline_anchor_ms: None,
                    marker_id: None,
                },
                match_kind: "semantic",
                relevance: Some(hit.score),
            });
        }
    }
    Ok(candidates)
}

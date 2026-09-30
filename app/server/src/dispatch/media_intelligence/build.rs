use super::model::*;
use super::*;
pub(super) fn clean_text(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(max_chars).collect()
}

fn receipt_path(snapshot: &ProjectSnapshot, relative: Option<&str>) -> Option<PathBuf> {
    resolve_existing_project_file(
        &snapshot.dir,
        relative?,
        "media evidence receipt",
        "re-run the matching analysis so Cut can publish a project-local receipt",
    )
    .ok()
}

fn duration_ms(asset: &cut_core::Asset) -> u64 {
    asset
        .probe
        .as_ref()
        .and_then(|value| value.get("duration_ms"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

pub(super) fn add_entry(
    entries: &mut Vec<EvidenceEntry>,
    asset_id: &str,
    kind: &str,
    range: [u64; 2],
    anchor_ms: u64,
    excerpt: String,
    speaker: Option<String>,
    provenance_sha256: &str,
    marker: Option<(&str, u64, &str)>,
) {
    let start = range[0];
    let end = range[1].max(start.saturating_add(1));
    let evidence_id = opaque_id(&[
        kind,
        asset_id,
        &start.to_string(),
        &end.to_string(),
        provenance_sha256,
        &excerpt,
    ]);
    entries.push(EvidenceEntry {
        evidence_id,
        asset_id: asset_id.to_string(),
        kind: kind.to_string(),
        source_start_ms: start,
        source_end_ms: end,
        anchor_ms,
        excerpt,
        speaker,
        provenance_sha256: provenance_sha256.to_string(),
        sequence_id: marker.map(|value| value.0.to_string()),
        timeline_anchor_ms: marker.map(|value| value.1),
        marker_id: marker.map(|value| value.2.to_string()),
    });
}

fn transcript_entries(
    entries: &mut Vec<EvidenceEntry>,
    asset_id: &str,
    transcript: &cut_perception::Transcript,
    provenance: &str,
) {
    let mut start = 0;
    while start < transcript.words.len() {
        let mut end = start;
        while end + 1 < transcript.words.len() && end + 1 - start < 14 {
            let word = &transcript.words[end];
            let next = &transcript.words[end + 1];
            if word.word.ends_with(['.', '!', '?'])
                || next.start_ms.saturating_sub(word.end_ms) > 800
            {
                break;
            }
            end += 1;
        }
        let words = &transcript.words[start..=end];
        let excerpt = clean_text(
            &words
                .iter()
                .map(|word| word.word.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            320,
        );
        let speaker = words
            .first()
            .and_then(|word| word.speaker.clone())
            .filter(|speaker| {
                words
                    .iter()
                    .all(|word| word.speaker.as_deref() == Some(speaker.as_str()))
            });
        add_entry(
            entries,
            asset_id,
            "transcript",
            [words[0].start_ms, words.last().unwrap().end_ms],
            words[0].start_ms,
            excerpt,
            speaker,
            provenance,
            None,
        );
        start = end + 1;
    }
}

fn perception_entries(
    entries: &mut Vec<EvidenceEntry>,
    asset_id: &str,
    report: &cut_perception::PerceptionReport,
    asset_duration: u64,
    kinds: &BTreeSet<String>,
    provenance: &str,
) {
    if kinds.contains("scene") {
        let mut starts = vec![0];
        starts.extend(report.scenes.iter().map(|scene| scene.at_ms));
        starts.sort_unstable();
        starts.dedup();
        for (index, start) in starts.iter().copied().enumerate() {
            let end = starts
                .get(index + 1)
                .copied()
                .unwrap_or(asset_duration.max(start.saturating_add(1)));
            add_entry(
                entries,
                asset_id,
                "scene",
                [start, end],
                start,
                format!("Scene {}", index + 1),
                None,
                provenance,
                None,
            );
        }
    }
    if kinds.contains("beat") {
        if let Some(grid) = &report.beats {
            for (index, beat) in grid.beats_ms.iter().copied().enumerate() {
                add_entry(
                    entries,
                    asset_id,
                    "beat",
                    [beat, beat.saturating_add(1)],
                    beat,
                    format!("Beat {} · {:.1} BPM", index + 1, grid.bpm),
                    None,
                    provenance,
                    None,
                );
            }
        }
    }
}

pub(super) fn build_index(
    snapshot: &ProjectSnapshot,
    bindings: Vec<SourceBinding>,
    kinds: BTreeSet<String>,
    cancellation: crate::jobs::JobCancellation,
) -> Result<MediaEvidenceIndex, CutError> {
    let mut entries = Vec::new();
    for binding in &bindings {
        if cancellation.is_cancelled() {
            return Err(CutError::new(
                error_codes::JOB_FAILED,
                "media intelligence rebuild was cancelled",
                "the current asset boundary was preserved",
            ));
        }
        let asset = &snapshot.project.assets[&binding.asset_id];
        if kinds.contains("metadata") {
            let label = Path::new(&asset.path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&binding.asset_id);
            let provenance = digest_bytes(
                format!("{}\n{}\n{}", binding.asset_id, binding.asset_hash, label).as_bytes(),
            );
            add_entry(
                &mut entries,
                &binding.asset_id,
                "metadata",
                [0, duration_ms(asset).max(1)],
                0,
                clean_text(label, 240),
                None,
                &provenance,
                None,
            );
        }
        if kinds.contains("transcript") {
            if let (Some(path), Some(provenance)) = (
                receipt_path(snapshot, asset.transcript.as_deref()),
                binding.transcript_sha256.as_deref(),
            ) {
                if let Ok(bytes) =
                    crate::vissearch::read_bounded_file(&path, MAX_EVIDENCE_RECEIPT_BYTES)
                {
                    if let Ok(transcript) = serde_json::from_slice(&bytes) {
                        transcript_entries(
                            &mut entries,
                            &binding.asset_id,
                            &transcript,
                            provenance,
                        );
                    }
                }
            }
        }
        if kinds.contains("scene") || kinds.contains("beat") {
            if let (Some(path), Some(provenance)) = (
                receipt_path(snapshot, asset.perception.as_deref()),
                binding.perception_sha256.as_deref(),
            ) {
                if let Ok(bytes) =
                    crate::vissearch::read_bounded_file(&path, MAX_EVIDENCE_RECEIPT_BYTES)
                {
                    if let Ok(report) = serde_json::from_slice(&bytes) {
                        perception_entries(
                            &mut entries,
                            &binding.asset_id,
                            &report,
                            duration_ms(asset),
                            &kinds,
                            provenance,
                        );
                    }
                }
            }
        }
    }
    if kinds.contains("marker") {
        let selected = bindings
            .iter()
            .map(|binding| binding.asset_id.clone())
            .collect();
        super::markers::marker_entries(&mut entries, snapshot, &selected);
    }
    entries.sort_by(|left, right| {
        (
            &left.kind,
            &left.asset_id,
            left.source_start_ms,
            &left.evidence_id,
        )
            .cmp(&(
                &right.kind,
                &right.asset_id,
                right.source_start_ms,
                &right.evidence_id,
            ))
    });
    let mut coverage = BTreeMap::new();
    for binding in &bindings {
        let mut ready = Vec::new();
        for kind in &kinds {
            let source_ready = match kind.as_str() {
                "transcript" => binding.transcript_sha256.is_some(),
                "visual" => binding.visual_sha256.is_some(),
                "scene" | "beat" => binding.perception_sha256.is_some(),
                "marker" | "metadata" => true,
                _ => false,
            };
            if source_ready {
                ready.push(kind.clone());
            }
        }
        coverage.insert(binding.asset_id.clone(), ready);
    }
    finalize_index(MediaEvidenceIndex {
        schema: INDEX_SCHEMA.into(),
        generator: generator_identity(),
        index_id: String::new(),
        project_id: opaque_id(&[&snapshot.dir.to_string_lossy()]),
        project_revision: snapshot.revision.clone(),
        selected_kinds: kinds.into_iter().collect(),
        bindings,
        coverage,
        entries,
    })
}

pub(super) fn publish_index(
    snapshot: &ProjectSnapshot,
    index: &MediaEvidenceIndex,
) -> Result<(), CutError> {
    publish_index_with_limit(snapshot, index, MAX_EVIDENCE_INDEX_BYTES)
}

pub(super) fn publish_index_with_limit(
    snapshot: &ProjectSnapshot,
    index: &MediaEvidenceIndex,
    max_bytes: u64,
) -> Result<(), CutError> {
    let payload = serde_json::to_vec_pretty(index)?;
    if payload.len() as u64 > max_bytes {
        return Err(CutError::new(
            error_codes::JOB_FAILED,
            "media intelligence index exceeds its byte limit",
            format!(
                "{} bytes exceeds {max_bytes} bytes; no index was published",
                payload.len()
            ),
        ));
    }
    let path = index_path(&snapshot.dir);
    crate::output_paths::ensure_plain_project_relative_dir(&snapshot.dir, Path::new("indexes"))?;
    write_output_atomic(&path, payload)
}

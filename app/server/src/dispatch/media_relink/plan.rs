//! Deterministic exact-hash plan construction and accepted selection.

use super::scan::{full_sha256_under_root, hash_json, scan_folder};
use super::*;
use cut_core::store::{is_exact_sha256, RelinkGroupChange};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn build_plan(snapshot: RelinkSnapshot) -> Result<PreparedPlan, CutError> {
    let (root, files, directories) = scan_folder(&snapshot.root)?;
    let mut candidates = Vec::with_capacity(files.len());
    for path in files {
        let hash = full_sha256_under_root(&path, &root)?;
        candidates.push(Candidate {
            display_name: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("media")
                .to_owned(),
            path: path.to_string_lossy().into_owned(),
            hash,
        });
    }
    candidates.sort_by(|left, right| left.path.cmp(&right.path));
    let mut exact = BTreeMap::<String, Vec<&Candidate>>::new();
    for candidate in &candidates {
        exact
            .entry(candidate.hash.clone())
            .or_default()
            .push(candidate);
    }
    let mut rows = Vec::new();
    for asset in snapshot
        .assets
        .into_iter()
        .filter(|asset| !asset.source_path.is_file())
    {
        let mut row = PlanAsset {
            asset_id: asset.asset_id.clone(),
            expected_hash: asset.expected_hash.clone(),
            old_path: asset.old_path.clone(),
            display_name: asset.display_name.clone(),
            disposition: "no_match".into(),
            chosen_path: None,
            chosen_hash: None,
            diagnostics: Vec::new(),
        };
        if !is_exact_sha256(&asset.expected_hash) {
            row.disposition = "hash_unavailable".into();
            row.diagnostics
                .push("stored identity is sampled or unavailable; exact relink is refused".into());
        } else if let Some(matches) = exact.get(&asset.expected_hash) {
            match matches.as_slice() {
                [candidate] => {
                    row.disposition = "eligible_exact_hash".into();
                    row.chosen_path = Some(candidate.path.clone());
                    row.chosen_hash = Some(candidate.hash.clone());
                }
                _ => {
                    row.disposition = "ambiguous_exact_hash".into();
                    row.diagnostics
                        .push(format!("{} exact candidates found", matches.len()));
                }
            }
        } else {
            classify_metadata_only(&mut row, &asset, &candidates);
        }
        rows.push(row);
    }
    let plan = RelinkPlan {
        project_identity: snapshot.project_identity,
        project_revision: snapshot.project_revision,
        root: root.to_string_lossy().into_owned(),
        scan_files: candidates.len(),
        scan_directories: directories,
        assets: rows,
    };
    let plan_hash = hash_json(&plan)?;
    Ok(PreparedPlan { plan, plan_hash })
}

fn classify_metadata_only(
    row: &mut PlanAsset,
    asset: &OfflineAssetSnapshot,
    candidates: &[Candidate],
) {
    let Some(kind) = asset.kind.as_deref() else {
        row.diagnostics
            .push("no recorded media kind for constrained metadata check".into());
        return;
    };
    let Some(duration_ms) = asset.duration_ms else {
        row.diagnostics
            .push("no recorded duration for constrained metadata check".into());
        return;
    };
    let matches = candidates
        .iter()
        .filter(|candidate| candidate.display_name == asset.display_name)
        .filter(|candidate| {
            cut_media::probe(Path::new(&candidate.path))
                .map(|probe| probe.kind == kind && probe.duration_ms == Some(duration_ms))
                .unwrap_or(false)
        })
        .count();
    if matches == 1 {
        row.disposition = "metadata_only".into();
        row.diagnostics.push(
            "filename, kind, and duration match but complete SHA-256 differs; refused".into(),
        );
    } else if matches > 1 {
        row.disposition = "ambiguous_metadata".into();
        row.diagnostics.push(format!(
            "{matches} filename/kind/duration candidates; refused"
        ));
    }
}

pub(super) fn preview_result(prepared: &PreparedPlan) -> Value {
    let assets = prepared
        .plan
        .assets
        .iter()
        .map(|row| {
            json!({
                "asset": row.asset_id,
                "expected_hash": row.expected_hash,
                "display_name": row.display_name,
                "disposition": row.disposition,
                "diagnostics": row.diagnostics,
                "candidate": row.chosen_hash.as_ref().map(|hash| json!({"sha256": hash})),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schema": PREVIEW_SCHEMA,
        "project_identity": prepared.plan.project_identity,
        "project_revision": prepared.plan.project_revision,
        "plan_hash": prepared.plan_hash,
        "scan": {"status": "complete", "files": prepared.plan.scan_files, "directories": prepared.plan.scan_directories},
        "assets": assets,
        "contract": {"selection": "eligible_exact_hash_only", "symlinks": "refused", "max_files": MAX_SCAN_FILES},
    })
}

pub(super) fn accepted_changes(
    plan: &RelinkPlan,
    accepted: &[String],
) -> Result<Vec<RelinkGroupChange>, CutError> {
    let requested = accepted.iter().collect::<BTreeSet<_>>();
    if requested.len() != accepted.len() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "bulk relink accept list repeats an asset",
            "select each eligible asset once",
        ));
    }
    let mut result = Vec::new();
    for asset_id in accepted {
        let row = plan
            .assets
            .iter()
            .find(|row| &row.asset_id == asset_id)
            .ok_or_else(|| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("asset '{asset_id}' is not in this offline-media preview"),
                    "refresh the preview",
                )
            })?;
        if row.disposition != "eligible_exact_hash" {
            return Err(CutError::new(
                error_codes::CONFLICT,
                format!(
                    "asset '{asset_id}' is {} and cannot be applied",
                    row.disposition
                ),
                "only a unique complete-SHA-256 candidate may be selected",
            ));
        }
        result.push(RelinkGroupChange {
            asset_id: row.asset_id.clone(),
            expected_hash: row.expected_hash.clone(),
            old_path: row.old_path.clone(),
            chosen_path: row
                .chosen_path
                .clone()
                .expect("eligible relink has chosen path"),
        });
    }
    Ok(result)
}

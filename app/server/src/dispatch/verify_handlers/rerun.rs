//! `verify.rerun` — repeat output-only checks on one receipt-bound artifact.
//!
//! A rerun never re-encodes or rewrites the source RenderReceipt. Its separate
//! receipt documents a fresh instrument pass over the exact output bytes.

use super::*;
use crate::dispatch::{owned_job_process_control, run_blocking, run_blocking_cancellable};
use crate::output_paths::write_output_atomic;
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};

#[path = "rerun_output.rs"]
mod rerun_output;
use rerun_output::fenced_output_for_receipt;

#[derive(serde::Deserialize)]
struct Args {
    render_id: String,
}

/// Re-run only checks that inspect the named render's already-produced bytes.
/// Timeline/source checks remain the original RenderReceipt's responsibility.
pub(crate) async fn verify_rerun(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    let args: Args = parse_args(args)?;
    let (project_dir, receipts, receipt) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let receipts = store.receipts_dir();
        let receipt = selected_render_receipt(&receipts, &args.render_id)?;
        (store.dir.clone(), receipts, receipt)
    };
    let output = fenced_output_for_receipt(&project_dir, &receipt.output_path, None)?;
    let expected_hash = receipt.output_hash.clone();
    let profile = profile_from_receipt(&receipt)?;

    // Refuse an already-stale artifact before allocating a durable job.
    let preflight_output = output.clone();
    let preflight_hash = expected_hash.clone();
    run_blocking("verify.rerun artifact identity", move || {
        assert_receipt_hash(&preflight_output, &preflight_hash)
    })
    .await?;

    let job = state.jobs.create("verify-rerun");
    let job_id = job.job_id.clone();
    let handle_render_id = receipt.render_id.clone();
    let handle_hash = expected_hash.clone();
    let verification_receipt = verification_receipt_name(&job_id);
    let st = state.clone();
    let jobs = state.jobs.clone();
    let job_hash = expected_hash.clone();
    jobs.spawn_limited(&job_id, "analysis", ANALYSIS_MAX_RUNNING, async move {
        let jid = job.job_id.clone();
        st.jobs.progress(
            &jid,
            0.15,
            Some("running output checks against the receipt-bound render".into()),
        );

        let artifact_output = output.clone();
        let artifact_hash = job_hash;
        let artifact_project = project_dir.clone();
        let artifact_receipts = receipts.clone();
        let artifact_render_id = receipt.render_id.clone();
        let artifact_receipt_path = receipt.output_path.clone();
        let receipt_duration = receipt.duration_ms;
        let receipt_profile = profile;
        let result_receipt = verification_receipt.clone();
        let instrument_job_id = jid.clone();
        let result = run_blocking_cancellable("verify.rerun output checks", move |cancellation| {
            // This single owner bounds every child in the whole recheck: the
            // sidecar and the ffprobe that follows it share cancellation,
            // deadline, process-tree cleanup, and diagnostic limits.
            let control = owned_job_process_control(cancellation);
            cut_media::ffmpeg::with_render_process_control(&control, || {
                let output = fenced_output_for_receipt(
                    &artifact_project,
                    &artifact_receipt_path,
                    Some(&artifact_output),
                )?;
                assert_receipt_hash(&output, &artifact_hash)?;

                let instrument_id = format!("{}.rerun.{}", artifact_render_id, instrument_job_id);
                let report = cut_perception::run_instruments_owned_ephemeral(
                    &output,
                    &artifact_receipts,
                    &instrument_id,
                    &artifact_hash,
                    cut_perception::InstrumentSet::RenderChecks,
                    None,
                    &control,
                );
                let report = report?;

                // The sidecar just consumed the file; fence and hash again
                // before the next subprocess opens it.
                let output = fenced_output_for_receipt(
                    &artifact_project,
                    &artifact_receipt_path,
                    Some(&artifact_output),
                )?;
                assert_receipt_hash(&output, &artifact_hash)?;
                let duration = cut_media::probe(&output)?.duration_ms.ok_or_else(|| {
                    CutError::new(
                        error_codes::FFMPEG,
                        "rendered output has no measurable duration",
                        "ffprobe did not report a duration for the artifact",
                    )
                })?;

                // The final fence/hash is immediately before persisting a
                // terminal claim, covering every prior file access.
                let output = fenced_output_for_receipt(
                    &artifact_project,
                    &artifact_receipt_path,
                    Some(&artifact_output),
                )?;
                assert_receipt_hash(&output, &artifact_hash)?;

                let facts = cut_perception::RenderFacts {
                    duration_ms: duration,
                    loudness: report.loudness.clone(),
                    output_report: Some(report),
                };
                let mut checks =
                    cut_perception::output_checks_with_profile(&facts, receipt_profile).into_vec();
                checks.push(duration_matches_receipt(duration, receipt_duration));
                let pass = checks.iter().all(|check| check.pass);
                let result = json!({
                    "render_id": artifact_render_id.clone(),
                    "source_receipt_id": artifact_render_id,
                    "verification_receipt": format!("receipts/{result_receipt}"),
                    "output_hash": artifact_hash,
                    "checked_at": OpRecord::now_ts(),
                    "scope": "rendered_output",
                    "profile": receipt_profile.as_str(),
                    "checks": checks,
                    "pass": pass,
                });
                write_verification_receipt(&artifact_receipts, &result_receipt, &result)?;
                Ok(result)
            })
        })
        .await;

        match result {
            Ok(result) => st.jobs.finish(&jid, result),
            Err(error) => st.jobs.fail(&jid, error),
        }
    });
    Ok(VerbResult::ok(json!({
        "job_id": job_id,
        "render_id": handle_render_id,
        "output_hash": handle_hash,
    })))
}

/// Load the receipt selected by the caller without following a leaf link or
/// accepting a file outside the project's own receipt directory. The embedded
/// id is a second identity fence: a named path cannot stand in for another
/// render's evidence.
fn selected_render_receipt(
    receipts: &Path,
    requested_id: &str,
) -> Result<cut_core::RenderReceipt, CutError> {
    plain_receipt_dir(receipts)?;
    let path = resolve_receipt_path(receipts, Some(requested_id))?;
    let receipts_dir = receipts.canonicalize().map_err(|error| {
        CutError::new(
            error_codes::IO,
            "could not inspect the project receipt directory",
            error.to_string(),
        )
    })?;
    let parent = path.parent().and_then(|parent| parent.canonicalize().ok());
    let plain = record_recovery::is_plain_regular_file(&path).map_err(|error| {
        CutError::new(
            error_codes::IO,
            "could not inspect the selected render receipt",
            error.to_string(),
        )
    })?;
    if parent.as_deref() != Some(receipts_dir.as_path()) || !plain {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "selected render receipt is not a local regular file in the receipt directory",
            format!("refusing unsafe receipt path {}", path.display()),
        )
        .with_suggested_action("render again to create a fresh local render receipt"));
    }
    let receipt: cut_core::RenderReceipt = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
    if receipt.render_id != requested_id {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "selected render receipt id does not match its file identity",
            format!(
                "requested {requested_id}; receipt embeds {}",
                receipt.render_id
            ),
        )
        .with_suggested_action("select the matching render receipt or render again"));
    }
    Ok(receipt)
}

fn plain_receipt_dir(receipts: &Path) -> Result<(), CutError> {
    let plain = record_recovery::is_plain_dir(receipts).map_err(|error| {
        CutError::new(
            error_codes::IO,
            "could not inspect the project receipt directory",
            error.to_string(),
        )
    })?;
    if !plain {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "project receipt directory is not a local directory",
            format!("refusing unsafe receipt directory {}", receipts.display()),
        )
        .with_suggested_action("render again to create receipts in the local project directory"));
    }
    Ok(())
}

fn verification_receipt_name(job_id: &str) -> String {
    format!("verify_rerun_{job_id}.json")
}

fn write_verification_receipt(receipts: &Path, name: &str, result: &Value) -> Result<(), CutError> {
    if name.starts_with("render_") {
        return Err(CutError::new(
            error_codes::IO,
            "could not allocate a separate verification receipt",
            "the render receipt is immutable",
        ));
    }
    plain_receipt_dir(receipts)?;
    let path = receipts.join(name);
    write_output_atomic(
        &path,
        serde_json::to_vec_pretty(result).map_err(CutError::from)?,
    )
}

fn profile_from_receipt(
    receipt: &cut_core::RenderReceipt,
) -> Result<cut_perception::FootageProfile, CutError> {
    let Some(entry) = receipt
        .checks
        .iter()
        .find(|check| check.name == cut_core::check_names::FOOTAGE_PROFILE)
    else {
        return Ok(cut_perception::FootageProfile::TalkingHead);
    };
    let Some(active) = entry.details.get("active_profile").and_then(Value::as_str) else {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "render receipt has invalid footage-profile evidence",
            "the footage_profile entry is missing details.active_profile",
        )
        .with_suggested_action("render again to create a complete receipt"));
    };
    active.parse().map_err(|reason: String| {
        CutError::new(
            error_codes::CONFLICT,
            "render receipt has an unknown footage profile",
            reason,
        )
        .with_suggested_action("render again with a supported footage profile")
    })
}

fn assert_receipt_hash(path: &Path, expected: &str) -> Result<(), CutError> {
    let actual = full_sha256(path)?;
    if actual == expected {
        return Ok(());
    }
    Err(CutError::new(
        error_codes::CONFLICT,
        "rendered output no longer matches its receipt",
        format!("receipt hash {expected}; current artifact hash {actual}"),
    )
    .with_suggested_action("render again before re-running output checks"))
}

fn full_sha256(path: &Path) -> Result<String, CutError> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn duration_matches_receipt(actual_ms: u64, receipt_ms: u64) -> cut_core::CheckResult {
    cut_core::CheckResult {
        name: "duration_matches_receipt".into(),
        pass: actual_ms == receipt_ms,
        details: json!({ "receipt_duration_ms": receipt_ms, "output_duration_ms": actual_ms }),
        evidence: json!({ "measured_by": "ffprobe", "output_duration_ms": actual_ms }),
    }
}

#[cfg(test)]
#[path = "rerun_tests.rs"]
mod tests;

//! Cancellable output-only instrument execution for `verify.rerun`.

use super::*;
use crate::dispatch::{owned_job_process_control, run_blocking_cancellable};

pub(super) fn spawn_verify_rerun_job(
    state: &AppState,
    job_id: &str,
    prepared: PreparedVerifyRerun,
) {
    let job_id = job_id.to_string();
    let verification_receipt = verification_receipt_name(&job_id);
    let st = state.clone();
    let jobs = state.jobs.clone();
    let spawned_job_id = job_id.clone();
    jobs.spawn_limited(&job_id, "analysis", ANALYSIS_MAX_RUNNING, async move {
        let jid = spawned_job_id;
        st.jobs.progress(
            &jid,
            0.15,
            Some("running output checks against the receipt-bound render".into()),
        );

        let artifact_output = prepared.output.clone();
        let artifact_hash = prepared.receipt.output_hash.clone();
        let artifact_project = prepared.project_dir.clone();
        let artifact_receipts = prepared.receipts.clone();
        let artifact_render_id = prepared.receipt.render_id.clone();
        let artifact_receipt_path = prepared.receipt.output_path.clone();
        let receipt_duration = prepared.receipt.duration_ms;
        let receipt_profile = prepared.profile;
        let result_receipt = verification_receipt.clone();
        let instrument_job_id = jid.clone();
        let result = run_blocking_cancellable("verify.rerun output checks", move |cancellation| {
            // This single owner bounds every child in the whole recheck: the
            // sidecar and the ffprobe that follows it share cancellation,
            // deadline, process-tree cleanup, and diagnostic limits.
            let control = owned_job_process_control(cancellation);
            cut_media::ffmpeg::with_render_process_control(&control, || {
                let output = rerun_output::fenced_output_for_receipt(
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
                let output = rerun_output::fenced_output_for_receipt(
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
                let output = rerun_output::fenced_output_for_receipt(
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
}

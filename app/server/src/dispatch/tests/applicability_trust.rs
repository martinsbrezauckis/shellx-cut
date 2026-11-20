//! Which quality checks apply to an export is decided on facts this engine
//! measured, never on probe facts cached in the project: a received project
//! that marks an audible clip "silent" does not waive a loudness target the
//! person set; an export that really has no sound still reads "not
//! applicable", with its reason.

use super::render_check_outcome::{project_with, StandInAnalysis};
use super::{lock_agent_cli_env, test_actor, wait_job};
use crate::dispatch::dispatch;
use crate::state::AppState;
use serde_json::{json, Value};

/// The stand-in analysis measures every output at -30 LUFS.
const MEASURED_LUFS: f64 = -30.0;

async fn render(state: &AppState, args: Value) -> Value {
    let rendered = dispatch(state, "render.final", args, test_actor()).await;
    assert!(rendered.ok, "{:?}", rendered.error);
    let job = wait_job(
        state,
        rendered.result.unwrap()["job_id"].as_str().unwrap(),
        180,
    )
    .await;
    assert_eq!(
        serde_json::to_value(&job).unwrap()["state"],
        "done",
        "{job:?}"
    );
    let checked = dispatch(state, "verify.checks", json!({}), test_actor()).await;
    checked.result.unwrap()
}

fn row<'a>(receipt: &'a Value, name: &str) -> &'a Value {
    receipt["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap()
}

#[tokio::test]
async fn a_forged_silent_probe_does_not_waive_an_asked_loudness_target() {
    let _env = lock_agent_cli_env();
    let _analysis = StandInAnalysis::install(MEASURED_LUFS);
    let (_root, state) = project_with("audible.mp4", 100).await;
    // The project now claims the audible clip has no sound, as a received
    // project.json can.
    crate::dispatch::media::update_asset(&state, "a1", |asset| {
        asset.probe.get_or_insert_with(|| json!({}))["has_audio"] = json!(false);
    })
    .await
    .unwrap();
    let receipt = render(
        &state,
        json!({"hardware": "off", "normalize_loudness": -16}),
    )
    .await;
    let lufs = row(&receipt, "lufs");
    assert!(
        lufs["details"].get("status").is_none(),
        "not waived: {lufs}"
    );
    assert_eq!(lufs["details"]["target_lufs"], -16.0, "{lufs}");
    assert_eq!(lufs["pass"], false, "-30 LUFS measured against -16: {lufs}");
    assert_eq!(receipt["pass"], false, "{receipt}");
}

#[tokio::test]
async fn an_export_that_really_has_no_sound_still_reads_not_applicable() {
    let _env = lock_agent_cli_env();
    let _analysis = StandInAnalysis::install(MEASURED_LUFS);
    let (root, state) = project_with("unused.mp4", 100).await;
    // A clip without any sound stream, placed on its own.
    let silent = root.path().join("no-sound.mp4");
    let made = std::process::Command::new(cut_media::ffmpeg::ffmpeg_bin())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc2=size=320x180:rate=25:duration=4")
        .args([
            "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p",
        ])
        .arg(&silent)
        .status()
        .unwrap();
    assert!(made.success());
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "silent", "dir": root.path().join("silent.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let imported = dispatch(
        &state,
        "media.import",
        json!({"path": silent}),
        test_actor(),
    )
    .await;
    assert!(imported.ok, "{:?}", imported.error);
    let job = wait_job(
        &state,
        imported.result.unwrap()["job_id"].as_str().unwrap(),
        60,
    )
    .await;
    if let Some(proxy) = job
        .result
        .and_then(|r| r["proxy_job"].as_str().map(String::from))
    {
        wait_job(&state, &proxy, 120).await;
    }
    // Even a project that (falsely) claims sound does not change this: the
    // engine measured the placed source, and it has none.
    crate::dispatch::media::update_asset(&state, "a1", |asset| {
        asset.probe.get_or_insert_with(|| json!({}))["has_audio"] = json!(true);
    })
    .await
    .unwrap();
    let receipt = render(&state, json!({"hardware": "off"})).await;
    for name in ["lufs", "silence_at_edges"] {
        let check = row(&receipt, name);
        assert_eq!(check["details"]["status"], "not_applicable", "{check}");
        assert_eq!(
            check["details"]["reason"], "The timeline has no sound.",
            "{check}"
        );
    }
}

/// A re-check relies on what a receipt decided (a row "not applicable") only
/// when this engine wrote that receipt; a receipt changed after it was written
/// (as a received project can carry) waives nothing.
#[tokio::test]
async fn a_rewritten_receipt_does_not_waive_a_recheck() {
    let _env = lock_agent_cli_env();
    let _analysis = StandInAnalysis::install(MEASURED_LUFS);
    let (_root, state) = project_with("recheck.mp4", 100).await;
    let receipt = render(
        &state,
        json!({"hardware": "off", "normalize_loudness": -16}),
    )
    .await;
    let render_id = receipt["render_id"].as_str().unwrap().to_string();
    let path = {
        let guard = state.project.read().await;
        guard
            .as_ref()
            .unwrap()
            .dir
            .join(format!("receipts/{render_id}.json"))
    };
    let mut planted: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for check in planted["checks"].as_array_mut().unwrap() {
        if check["name"] == "lufs" {
            *check = json!({"name": "lufs", "pass": true, "evidence": {},
                "details": {"status": "not_applicable", "applies_when": "x", "reason": "x"}});
        }
    }
    std::fs::write(&path, serde_json::to_vec_pretty(&planted).unwrap()).unwrap();
    let rerun = dispatch(
        &state,
        "verify.rerun",
        json!({"render_id": render_id}),
        test_actor(),
    )
    .await;
    assert!(rerun.ok, "{:?}", rerun.error);
    let job = wait_job(
        &state,
        rerun.result.unwrap()["job_id"].as_str().unwrap(),
        120,
    )
    .await;
    let result = job.result.unwrap();
    let lufs = row(&result, "lufs");
    assert!(
        lufs["details"].get("status").is_none(),
        "not waived: {lufs}"
    );
    assert_eq!(lufs["pass"], false, "{lufs}");
}

/// Attaching a review verdict rewrites a receipt; that rewrite never makes a
/// receipt this engine did not write into one it did.
#[tokio::test]
async fn attaching_a_verdict_does_not_launder_a_planted_receipt() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "planted", "dir": root.path().join("planted.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let receipts = root.path().join("planted.cutproj").join("receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    let receipts = receipts.canonicalize().unwrap();
    let path = receipts.join("render_001.json");
    let planted = json!({
        "render_id": "render_001", "ts": "t", "output_path": "exports/render_001.mp4",
        "output_hash": "sha256:00", "duration_ms": 1000, "preset": "draft", "at_op": "op_000001",
        "pass": true,
        "checks": [{"name": "lufs", "pass": true, "evidence": {},
            "details": {"status": "not_applicable", "applies_when": "x", "reason": "x"}}],
    });
    std::fs::write(&path, serde_json::to_vec_pretty(&planted).unwrap()).unwrap();
    crate::dispatch::verify_handlers::attach_judge_to_receipt(
        &state,
        &receipts,
        &path,
        json!({"status": "not_run"}),
    )
    .unwrap();
    assert!(
        !cut_media::cache_trust::trusted(&path),
        "a planted receipt stays not this engine's after the verdict is attached"
    );
    // A receipt this engine wrote stays its own after the same rewrite.
    cut_media::cache_trust::remember(&path);
    crate::dispatch::verify_handlers::attach_judge_to_receipt(
        &state,
        &receipts,
        &path,
        json!({"status": "not_run"}),
    )
    .unwrap();
    assert!(cut_media::cache_trust::trusted(&path));
}

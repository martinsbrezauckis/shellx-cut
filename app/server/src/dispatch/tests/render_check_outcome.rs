//! A render whose own checks fail is never plain "done" (fix-list item 166).
//!
//! The HQ Mac run rendered a cut-off source: the receipt failed
//! `duration_matches_edl` and `sources_decodable`, but the job read "done".
//! Now the job ends "done, but it did not pass its checks" (outcome_reason
//! checks_failed) with the verdict, the failing checks in plain words and the
//! way forward; the file is kept; the receipt and verify.checks carry the same.

use super::{test_actor, wait_job};
use crate::dispatch::dispatch;
use crate::jobs::JobState;
use crate::state::AppState;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub(super) fn make_clip(dir: &Path, name: &str, keep_percent: usize) -> PathBuf {
    let full = dir.join(format!("full-{name}"));
    let made = std::process::Command::new(cut_media::ffmpeg::ffmpeg_bin())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc2=size=320x180:rate=25:duration=8")
        .args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=8",
        ])
        .args([
            "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p",
        ])
        .args(["-c:a", "aac", "-b:a", "96k", "-movflags", "+faststart"])
        .arg(&full)
        .status()
        .unwrap();
    assert!(made.success());
    let bytes = std::fs::read(&full).unwrap();
    let out = dir.join(name);
    std::fs::write(&out, &bytes[..bytes.len() * keep_percent / 100]).unwrap();
    out
}

pub(super) async fn project_with(name: &str, keep_percent: usize) -> (tempfile::TempDir, AppState) {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "outcome", "dir": root.path().join("outcome.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let media = make_clip(root.path(), name, keep_percent);
    let imported = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(imported.ok, "{:?}", imported.error);
    let job = wait_job(
        &state,
        imported.result.unwrap()["job_id"].as_str().unwrap(),
        60,
    )
    .await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let proxy = job.result.unwrap()["proxy_job"]
        .as_str()
        .unwrap()
        .to_string();
    wait_job(&state, &proxy, 120).await;
    (root, state)
}

async fn render_of(
    name: &str,
    keep_percent: usize,
) -> (tempfile::TempDir, AppState, crate::jobs::JobRecord) {
    render_with(name, keep_percent, json!({"hardware": "off"})).await
}

/// Render with explicit render.final args (a profile, a loudness target).
async fn render_with(
    name: &str,
    keep_percent: usize,
    args: Value,
) -> (tempfile::TempDir, AppState, crate::jobs::JobRecord) {
    let (root, state) = project_with(name, keep_percent).await;
    let rendered = dispatch(&state, "render.final", args, test_actor()).await;
    assert!(rendered.ok, "{:?}", rendered.error);
    let job = wait_job(
        &state,
        rendered.result.unwrap()["job_id"].as_str().unwrap(),
        180,
    )
    .await;
    (root, state, job)
}

#[tokio::test]
async fn a_render_of_a_cut_off_source_ends_done_but_not_passing_and_says_why() {
    let _env = super::lock_agent_cli_env();
    let _analysis = StandInAnalysis::install(ON_TARGET_LUFS);
    let (_root, state, job) = render_of("cut-off.mp4", 25).await;
    assert_eq!(
        job.state,
        JobState::Done,
        "the file is written, not a failure: {job:?}"
    );
    // Read as an agent reads jobs.status (JSON), not through Rust types.
    let record = serde_json::to_value(&job).unwrap();
    assert_eq!(record["completion"], "done_with_warnings", "{record}");
    assert_eq!(record["outcome_reason"], "checks_failed", "{record}");
    assert_eq!(record["outcome"], "partial", "not a success: {record}");
    assert_eq!(record["message"], "done, but it did not pass its checks");
    let result = job.result.unwrap();
    assert!(
        Path::new(result["path"].as_str().unwrap()).is_file(),
        "the file is kept"
    );
    let checks = &result["checks"];
    assert_eq!(checks["verdict"], "problems", "{checks}");
    let names: Vec<&str> = checks["failed"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["level"] == "problem")
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["duration_matches_edl", "sources_decodable"]);
    assert_eq!(
        checks["failed"][0]["meaning"],
        "The file is not as long as the timeline: part of the picture or sound is missing."
    );
    let warnings = result["warnings"].as_array().unwrap();
    assert!(warnings[0]
        .as_str()
        .unwrap()
        .starts_with("The export was written but did not pass its checks (duration_matches_edl, sources_decodable)"));
    assert!(warnings.iter().any(|w| w
        .as_str()
        .unwrap()
        .contains("If a source is reported as cut off or damaged, import a complete copy")));

    // The receipt and verify.checks carry the same verdict.
    let receipt: Value =
        serde_json::from_slice(&std::fs::read(result["receipt"].as_str().unwrap()).unwrap())
            .unwrap();
    assert_eq!(receipt["outcome"], *checks);
    let verified = dispatch(&state, "verify.checks", json!({}), test_actor()).await;
    assert!(verified.ok, "{:?}", verified.error);
    assert_eq!(verified.result.unwrap()["outcome"]["verdict"], "problems");
}

#[tokio::test]
async fn a_render_of_a_whole_source_with_nothing_wrong_ends_plain_done() {
    // Every check that applies is measured and passes: plain "done", and a
    // queue of it reads "all delivered" (item 166 applicability rule).
    let _env = super::lock_agent_cli_env();
    let _analysis = StandInAnalysis::install(ON_TARGET_LUFS);
    let (_root, state, job) = render_of("whole.mp4", 100).await;
    let record = serde_json::to_value(&job).unwrap();
    assert_eq!(record["state"], "done", "{record}");
    assert_eq!(record["completion"], "success", "{record}");
    assert_eq!(record["outcome"], "succeeded", "{record}");
    assert_eq!(record["outcome_reason"], "completed", "{record}");
    assert_eq!(record["message"], "done", "{record}");
    assert_eq!(record["result"]["checks"]["verdict"], "passed", "{record}");
    assert!(record["result"].get("warnings").is_none(), "{record}");

    let queue = queue_one(&state).await;
    assert_eq!(queue["completion"], "success", "{queue}");
    assert_eq!(
        queue["result"]["summary_line"],
        "render queue: 1/1 delivered"
    );
}

#[tokio::test]
async fn a_queue_delivery_with_a_quality_warning_makes_the_queue_warn() {
    let _env = super::lock_agent_cli_env();
    let _analysis = StandInAnalysis::install(QUIET_LUFS);
    // The editorial battery is asked for (an explicit footage profile).
    let (_root, state, job) = render_with(
        "quiet.mp4",
        100,
        json!({"hardware": "off", "profile": "talking_head"}),
    )
    .await;
    let record = serde_json::to_value(&job).unwrap();
    assert_eq!(record["completion"], "done_with_warnings", "{record}");
    assert_eq!(
        record["outcome"], "succeeded",
        "warned, still delivered: {record}"
    );
    assert_eq!(
        record["outcome_reason"], "completed_with_warnings",
        "{record}"
    );
    assert_eq!(
        record["result"]["checks"]["verdict"], "warnings",
        "{record}"
    );

    let queue = queue_with(
        &state,
        json!({"preset": "draft", "hardware": "off", "profile": "talking_head"}),
    )
    .await;
    assert_eq!(queue["completion"], "done_with_warnings", "{queue}");
    assert_eq!(
        queue["result"]["summary_line"],
        "render queue: 1/1 delivered, 1 with warnings"
    );
    assert!(queue["result"]["warnings"][0]
        .as_str()
        .unwrap()
        .starts_with("delivery 1: The export was written; 1 quality check did not pass (lufs)."));
}

#[tokio::test]
async fn a_render_queue_with_a_cut_off_delivery_does_not_read_all_delivered() {
    let _env = super::lock_agent_cli_env();
    let _analysis = StandInAnalysis::install(ON_TARGET_LUFS);
    let (_root, state) = project_with("queued-cut-off.mp4", 25).await;
    let queued = dispatch(
        &state,
        "render.queue",
        json!({"jobs": [{"preset": "draft", "hardware": "off"}]}),
        test_actor(),
    )
    .await;
    assert!(queued.ok, "{:?}", queued.error);
    let job = wait_job(
        &state,
        queued.result.unwrap()["queue_id"].as_str().unwrap(),
        240,
    )
    .await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let record = serde_json::to_value(&job).unwrap();
    assert_eq!(record["outcome_reason"], "checks_failed", "{record}");
    let result = job.result.unwrap();
    assert_eq!(
        result["summary_line"],
        "render queue: 1/1 delivered, 1 written but did not pass their checks"
    );
    assert!(result["warnings"][0]
        .as_str()
        .unwrap()
        .starts_with("delivery 1: The export was written but did not pass its checks"));
}

/// A stand-in analysis tool, so every test here states what the analysis
/// measured instead of depending on the machine (whether the captions payload,
/// a Python or the real instruments are reachable from an isolated test
/// process). It answers every request with a clean report (full-frame picture,
/// no silence, black or frozen stretch) at the loudness `{LUFS}`.
const STAND_IN_ANALYSIS: &str = r#"import json, os, sys
request = json.load(sys.stdin)
if not os.path.exists(str(request["media_path"])):
    sys.exit("media not found")
print(json.dumps({"schema": "shellx-cut/perception/1", "asset_hash": request["asset_hash"],
                  "source_path": str(request["media_path"]),
                  "instruments_run": request["instruments"],
                  "silences": [], "black_spans": [], "frozen_spans": [],
                  "content_bbox": {"frame_width": 320, "frame_height": 180, "x": 0, "y": 0,
                                   "width": 320, "height": 180, "uniform_border": False},
                  "loudness": {"integrated_lufs": {LUFS}, "true_peak_dbtp": -12.0}}))
"#;

/// On target (-16 LUFS against a -16 target): every output check passes.
pub(super) const ON_TARGET_LUFS: f64 = -16.0;
/// Far below the target: `lufs` fails, a quality warning.
const QUIET_LUFS: f64 = -30.0;

/// Install the stand-in for the life of the guard. Hold the env lock: other
/// tests repoint the analysis tool while they hold it.
pub(super) struct StandInAnalysis {
    _dir: tempfile::TempDir,
    previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl StandInAnalysis {
    pub(super) fn install(integrated_lufs: f64) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let script = STAND_IN_ANALYSIS.replace("{LUFS}", &format!("{integrated_lufs:.1}"));
        std::fs::write(dir.path().join("instruments.py"), script).unwrap();
        let python = super::super::adapter_python::find_python_on_path().expect("python on PATH");
        let keys = [
            cut_perception::sidecar::ENV_SIDECAR_DIR,
            cut_perception::sidecar::ENV_PYTHON,
            cut_native_runtime_context::CONTEXT_ENV,
        ];
        let previous = keys.iter().map(|k| (*k, std::env::var_os(k))).collect();
        std::env::set_var(cut_perception::sidecar::ENV_SIDECAR_DIR, dir.path());
        std::env::set_var(cut_perception::sidecar::ENV_PYTHON, python);
        std::env::remove_var(cut_native_runtime_context::CONTEXT_ENV);
        Self {
            _dir: dir,
            previous,
        }
    }
}

impl Drop for StandInAnalysis {
    fn drop(&mut self) {
        for (key, value) in self.previous.drain(..) {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

async fn queue_one(state: &AppState) -> Value {
    queue_with(state, json!({"preset": "draft", "hardware": "off"})).await
}

async fn queue_with(state: &AppState, entry: Value) -> Value {
    let queued = dispatch(
        state,
        "render.queue",
        json!({"jobs": [entry]}),
        test_actor(),
    )
    .await;
    assert!(queued.ok, "{:?}", queued.error);
    let queue = wait_job(
        state,
        queued.result.unwrap()["queue_id"].as_str().unwrap(),
        240,
    )
    .await;
    serde_json::to_value(&queue).unwrap()
}

#[tokio::test]
async fn a_recheck_and_an_autopilot_with_failing_checks_do_not_read_plain_done() {
    // Other tests repoint the analysis tool while they hold this lock.
    let _env = super::lock_agent_cli_env();
    let _analysis = StandInAnalysis::install(QUIET_LUFS);
    // The editorial battery is asked for, so the re-check measures loudness.
    let (_root, state, job) = render_with(
        "recheck-cut-off.mp4",
        25,
        json!({"hardware": "off", "profile": "talking_head"}),
    )
    .await;
    let render_id = job.result.unwrap()["render_id"]
        .as_str()
        .unwrap()
        .to_string();

    let rerun = dispatch(
        &state,
        "verify.rerun",
        json!({"render_id": render_id}),
        test_actor(),
    )
    .await;
    assert!(rerun.ok, "{:?}", rerun.error);
    let rerun = wait_job(
        &state,
        rerun.result.unwrap()["job_id"].as_str().unwrap(),
        120,
    )
    .await;
    let rerun = serde_json::to_value(&rerun).unwrap();
    assert_eq!(rerun["state"], "done", "{rerun}");
    let verdict = &rerun["result"]["outcome"];
    assert!(
        verdict["verdict"].is_string(),
        "the re-check carries a verdict: {rerun}"
    );
    // The stand-in measures -30 LUFS against a -16 target: the re-check fails.
    assert_eq!(rerun["result"]["pass"], false, "{rerun}");
    assert_eq!(rerun["completion"], "done_with_warnings", "{rerun}");
    assert_eq!(rerun["result"]["warnings"][0], verdict["summary"]);

    let auto = dispatch(
        &state,
        "autopilot.run",
        json!({"max_fix_iters": 1}),
        test_actor(),
    )
    .await;
    assert!(auto.ok, "{:?}", auto.error);
    let auto = wait_job(
        &state,
        auto.result.unwrap()["job_id"].as_str().unwrap(),
        240,
    )
    .await;
    let auto = serde_json::to_value(&auto).unwrap();
    assert_eq!(auto["state"], "done", "{auto}");
    assert_eq!(auto["result"]["checks_pass"], false, "{auto}");
    assert_eq!(auto["completion"], "done_with_warnings", "{auto}");
    assert_eq!(auto["outcome"], "partial", "{auto}");
    assert_eq!(auto["outcome_reason"], "checks_failed", "{auto}");
    assert_eq!(
        auto["result"]["warnings"][0],
        auto["result"]["summary_line"]
    );
}

/// F11 (installed Windows run): a plain export with sound is not judged
/// against a loudness target or an edge-silence budget nobody asked for (as in
/// Premiere Pro / Media Encoder, where loudness is checked only with a target);
/// a target the person sets is measured against THAT target.
#[tokio::test]
async fn a_plain_export_is_not_judged_against_a_loudness_target_nobody_set() {
    let _env = super::lock_agent_cli_env();
    let _analysis = StandInAnalysis::install(QUIET_LUFS);
    let (_root, state, job) = render_of("plain-sound.mp4", 100).await;
    let record = serde_json::to_value(&job).unwrap();
    assert_eq!(record["completion"], "success", "{record}");
    assert_eq!(record["message"], "done", "{record}");
    let na = record["result"]["checks"]["not_applicable"]
        .as_array()
        .unwrap();
    assert!(
        na.iter().any(|n| n == "lufs") && na.iter().any(|n| n == "silence_at_edges"),
        "{record}"
    );

    let targeted = dispatch(
        &state,
        "render.final",
        json!({"hardware": "off", "normalize_loudness": -23}),
        test_actor(),
    )
    .await;
    assert!(targeted.ok, "{:?}", targeted.error);
    let job = wait_job(
        &state,
        targeted.result.unwrap()["job_id"].as_str().unwrap(),
        180,
    )
    .await;
    let record = serde_json::to_value(&job).unwrap();
    assert_eq!(record["completion"], "done_with_warnings", "{record}");
    let receipt = dispatch(&state, "verify.checks", json!({}), test_actor()).await;
    let receipt = receipt.result.unwrap();
    let lufs = receipt["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "lufs")
        .unwrap()
        .clone();
    assert_eq!(lufs["details"]["target_lufs"], -23.0, "{lufs}");
    assert_eq!(
        lufs["pass"], false,
        "the stand-in measured -30 LUFS: {lufs}"
    );
}

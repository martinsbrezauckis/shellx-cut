//! A quality check with nothing to check on this export is "not applicable",
//! never a failure or a warning, and an ordinary export ends plain "done"
//! (fix-list item 166 follow-up). Run as on a machine WITHOUT the captions
//! tools: the output checks are then "unmeasured" (they apply, with reason),
//! which is a different state and still visible.

use super::{lock_agent_cli_env, test_actor, wait_job};
use crate::dispatch::dispatch;
use crate::state::AppState;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Point the analysis runtime at nothing (captions tools not installed) for
/// the life of the guard, restoring the previous values after.
struct NoCaptionsTools {
    sidecar: Option<std::ffi::OsString>,
    python: Option<std::ffi::OsString>,
}

impl NoCaptionsTools {
    fn set(dir: &Path) -> Self {
        let guard = Self {
            sidecar: std::env::var_os("SHELLX_CUT_SIDECAR_DIR"),
            python: std::env::var_os("SHELLX_CUT_PYTHON"),
        };
        std::env::set_var("SHELLX_CUT_SIDECAR_DIR", dir.join("missing-sidecar"));
        std::env::set_var("SHELLX_CUT_PYTHON", dir.join("missing-python"));
        guard
    }
}

impl Drop for NoCaptionsTools {
    fn drop(&mut self) {
        for (key, value) in [
            ("SHELLX_CUT_SIDECAR_DIR", self.sidecar.take()),
            ("SHELLX_CUT_PYTHON", self.python.take()),
        ] {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn make_clip(dir: &Path, name: &str, keep_percent: usize) -> PathBuf {
    let full = dir.join(format!("full-{name}"));
    let made = std::process::Command::new(cut_media::ffmpeg::ffmpeg_bin())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc2=size=320x180:rate=25:duration=4")
        .args(["-f", "lavfi", "-i"])
        .arg("sine=frequency=440:sample_rate=48000:duration=4")
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

async fn project(dir: &Path, clip: &Path) -> AppState {
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "applicable", "dir": dir.join("applicable.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let imported = dispatch(&state, "media.import", json!({"path": clip}), test_actor()).await;
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
    state
}

async fn render(state: &AppState) -> Value {
    let rendered = dispatch(
        state,
        "render.final",
        json!({"hardware": "off"}),
        test_actor(),
    )
    .await;
    assert!(rendered.ok, "{:?}", rendered.error);
    let job = wait_job(
        state,
        rendered.result.unwrap()["job_id"].as_str().unwrap(),
        180,
    )
    .await;
    serde_json::to_value(&job).unwrap()
}

fn row<'a>(receipt: &'a Value, name: &str) -> &'a Value {
    receipt["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("{name} missing: {receipt}"))
}

async fn receipt(state: &AppState) -> Value {
    let checked = dispatch(state, "verify.checks", json!({}), test_actor()).await;
    assert!(checked.ok, "{:?}", checked.error);
    checked.result.unwrap()
}

#[tokio::test]
async fn an_ordinary_export_without_captions_tools_ends_plain_done() {
    let _env = lock_agent_cli_env();
    let root = tempfile::tempdir().unwrap();
    let _tools = NoCaptionsTools::set(root.path());
    let clip = make_clip(root.path(), "ordinary.mp4", 100);
    let state = project(root.path(), &clip).await;
    let job = render(&state).await;
    assert_eq!(job["state"], "done", "{job}");
    assert_eq!(job["completion"], "success", "{job}");
    assert_eq!(job["outcome_reason"], "completed", "{job}");
    assert_eq!(job["message"], "done", "{job}");
    assert!(
        job["result"].get("warnings").is_none(),
        "no warnings: {job}"
    );
    let checks = &job["result"]["checks"];
    assert_eq!(checks["verdict"], "unmeasured", "{checks}");
    assert_eq!(
        checks["not_applicable"],
        json!([
            "cut_on_word",
            "lufs",
            "caption_presence",
            "silence_at_edges"
        ])
    );
    assert!(checks.get("failed").is_none(), "{checks}");

    let receipt = receipt(&state).await;
    let captions = row(&receipt, "caption_presence");
    assert_eq!(captions["details"]["status"], "not_applicable");
    assert_eq!(
        captions["details"]["reason"],
        "This export has no captions."
    );
    assert_eq!(
        row(&receipt, "cut_on_word")["details"]["status"],
        "not_applicable"
    );
    // No loudness target and no profile: loudness is not judged (F11).
    assert_eq!(
        row(&receipt, "lufs")["details"]["reason"],
        "No loudness target was set for this export."
    );
    // The picture checks apply but could not be measured: that stays
    // visible, with its reason.
    let frames = row(&receipt, "black_or_frozen_frames");
    assert_eq!(frames["details"]["status"], "unmeasured", "{frames}");
    assert!(frames["details"]["reason"]
        .as_str()
        .unwrap()
        .starts_with("not measured, because"));
    assert_eq!(
        receipt["outcome"]["not_applicable"],
        checks["not_applicable"]
    );
}

#[tokio::test]
async fn a_hidden_caption_track_is_not_applicable_and_a_visible_one_is_checked() {
    let _env = lock_agent_cli_env();
    let root = tempfile::tempdir().unwrap();
    let _tools = NoCaptionsTools::set(root.path());
    let clip = make_clip(root.path(), "captioned.mp4", 100);
    let state = project(root.path(), &clip).await;
    let added = dispatch(
        &state,
        "captions.add_text",
        json!({"text": "hello there", "range_ms": [0, 1500]}),
        test_actor(),
    )
    .await;
    assert!(added.ok, "{:?}", added.error);

    // Visible: burned in, so the check applies and is measured (it passes).
    let shown = render(&state).await;
    assert_eq!(shown["message"], "done", "{shown}");
    let captions = row(&receipt(&state).await, "caption_presence").clone();
    assert!(captions["details"].get("status").is_none(), "{captions}");
    assert_eq!(captions["pass"], true, "{captions}");

    // Hidden: nothing is burned, so it does not apply, and says why.
    let track = {
        let st = dispatch(&state, "project.state", json!({}), test_actor()).await;
        st.result.unwrap()["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["kind"] == "caption")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let hidden = dispatch(
        &state,
        "edit.track_visible",
        json!({"track": track, "on": false}),
        test_actor(),
    )
    .await;
    assert!(hidden.ok, "{:?}", hidden.error);
    let job = render(&state).await;
    assert_eq!(job["message"], "done", "{job}");
    assert!(job["result"].get("warnings").is_none(), "{job}");
    let captions = row(&receipt(&state).await, "caption_presence").clone();
    assert_eq!(captions["details"]["status"], "not_applicable");
    assert_eq!(
        captions["details"]["reason"],
        "The caption track is hidden, so this export carries no captions."
    );
}

#[tokio::test]
async fn a_cut_off_source_without_captions_tools_still_does_not_pass() {
    // The installed Windows run: a PC without the captions tools.
    let _env = lock_agent_cli_env();
    let root = tempfile::tempdir().unwrap();
    let _tools = NoCaptionsTools::set(root.path());
    let clip = make_clip(root.path(), "cut-off.mp4", 25);
    let state = project(root.path(), &clip).await;
    let job = render(&state).await;
    assert_eq!(job["outcome_reason"], "checks_failed", "{job}");
    assert_eq!(job["message"], "done, but it did not pass its checks");
    let result = &job["result"];
    assert_eq!(
        result["setup_needed"]["missing"], "captions_runtime",
        "{result}"
    );
    assert!(result["warnings"][0]
        .as_str()
        .unwrap()
        .starts_with("The export was written but did not pass its checks"));
    let failed: Vec<&str> = result["checks"]["failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        failed,
        ["duration_matches_edl", "sources_decodable"],
        "{result}"
    );
}

async fn export_with_list(root: &Path, name: &str) -> (Value, Value) {
    let clip = make_clip(root, name, 100);
    let state = project(root, &clip).await;
    let job = render(&state).await;
    // The list is information: the export still ends plain done.
    assert_eq!(job["state"], "done", "{job}");
    assert_eq!(job["result"]["export_checks"]["phase"], "finished_file");
    let receipt = receipt(&state).await;
    dump(&format!("receipt-{name}.json"), &receipt);
    let list = receipt["export_checks"].clone();
    assert_eq!(list["phase"], "finished_file", "{receipt}");
    assert_eq!(list["render_id"], receipt["render_id"]);
    assert_eq!(list["entries"].as_array().unwrap().len(), 27);
    (job, list)
}

/// Real replies for the pictures of the interface: written when
/// `EXPORT_CHECKS_DUMP` names a folder, otherwise nothing happens.
fn dump(name: &str, value: &Value) {
    if let Some(dir) = std::env::var_os("EXPORT_CHECKS_DUMP") {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            Path::new(&dir).join(name),
            serde_json::to_vec_pretty(value).unwrap(),
        )
        .unwrap();
    }
}

fn entry<'a>(list: &'a Value, id: &str) -> &'a Value {
    list["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .unwrap()
}

#[tokio::test]
async fn a_configured_tool_that_cannot_start_shows_as_an_error_with_a_copyable_reason() {
    let _env = lock_agent_cli_env();
    let root = tempfile::tempdir().unwrap();
    // The configured interpreter is gone: an installation that is broken, not
    // one that was never made.
    let _tools = NoCaptionsTools::set(root.path());
    let (job, list) = export_with_list(root.path(), "damaged.mp4").await;
    assert_eq!(entry(&list, "duration_matches_edl")["state"], "checked");
    for id in ["black_or_frozen_frames", "black_or_frozen"] {
        let row = entry(&list, id);
        assert_eq!(row["state"], "error", "{id}: {row}");
        assert_eq!(row["reason_code"], "check_failed");
        assert_eq!(row["action"]["kind"], "copy_details");
        let text = row["action"]["text"].as_str().unwrap();
        assert!(text.contains(id) && text.contains("Cut: "), "{text}");
        assert!(row["report_details"]["check_id"] == id);
    }
    // The verdict of the export itself did not change.
    assert_eq!(job["completion"], "success", "{job}");
}

#[tokio::test]
async fn before_the_export_a_tool_that_cannot_start_is_an_error_and_the_verdict_is_unchanged() {
    let _env = lock_agent_cli_env();
    let root = tempfile::tempdir().unwrap();
    let _tools = NoCaptionsTools::set(root.path());
    let clip = make_clip(root.path(), "before.mp4", 100);
    let state = project(root.path(), &clip).await;
    let reply = dispatch(&state, "verify.pregate", json!({}), test_actor()).await;
    assert!(reply.ok, "{:?}", reply.error);
    let res = reply.result.unwrap();
    dump("pregate-damaged.json", &res);
    let list = &res["export_checks"];
    assert_eq!(list["phase"], "before_export");
    for id in ["black_or_frozen", "silent_output"] {
        let row = entry(list, id);
        assert_eq!(row["state"], "error", "{id}: {row}");
        assert_eq!(row["action"]["kind"], "copy_details");
        assert!(row["action"]["text"].as_str().unwrap().contains(id));
    }
    // Calculated from the timeline: a real answer even with the tools broken.
    assert_eq!(entry(list, "empty_tail")["state"], "checked");
    // The old verdict fields mean what they always meant.
    assert_eq!(res["pass"], true);
}

//! An older project with an odd frame size: the person is told BEFORE
//! anything is exported or changed, and the fix is one `project.format` call.
//!
//! Before 0.6.116 a first clip could give a project an odd size; every video
//! render of it failed at the pad/encoder stage. Item 104 rounded it down
//! during `render.final` only; the owner's decision (2026-11-20) is to say so
//! first instead: every video export at the project size refuses up front
//! with `odd_project_size` and the exact call, stills still work.

use super::{test_actor, wait_job};
use crate::dispatch::dispatch;
use crate::jobs::JobState;
use crate::state::AppState;
use serde_json::{json, Value};

const MESSAGE: &str = "This project is 641x361, but video exports need an even width and height. \
                       Change the project to 640x360";
const ACTION: &str = r#"project.format {"width":640,"height":360}"#;

/// A project holding one 1 s clip whose size the test then makes odd, the
/// way an older release left it.
async fn odd_project(root: &std::path::Path) -> AppState {
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "odd-old", "dir": root.join("odd-old.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let media = root.join("clip.mp4");
    let made = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc2=size=320x180:rate=30:duration=1")
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&media)
        .status()
        .unwrap();
    assert!(made.success());
    let imported = dispatch(
        &state,
        "media.import",
        json!({"path": media, "proxy": false}),
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
    assert_eq!(job.state, JobState::Done, "{job:?}");
    {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        store.project.settings.width = 641;
        store.project.settings.height = 361;
        store.save().unwrap();
    }
    state
}

#[tokio::test]
async fn every_video_export_of_an_odd_project_is_refused_first_with_the_exact_call() {
    let root = tempfile::tempdir().unwrap();
    let state = odd_project(root.path()).await;
    for (verb, args) in [
        ("render.final", json!({"dry_run": true})),
        ("render.final", json!({"hardware": "off"})),
        ("render.queue", json!({"jobs": [{"preset": "draft"}]})),
        ("render.direct", json!({})),
        ("render.reframe", json!({"aspect": "9:16"})),
        ("render.preview", json!({"at_ms": 0, "duration_ms": 500})),
        (
            "export.range",
            json!({"range_ms": [0, 500], "to_asset": false}),
        ),
        ("export.gif", json!({"range_ms": [0, 500]})),
        ("autopilot.run", json!({})),
    ] {
        let r = dispatch(&state, verb, args.clone(), test_actor()).await;
        let error = r
            .error
            .unwrap_or_else(|| panic!("{verb} {args} must refuse before anything runs"));
        assert_eq!(error.code, "odd_project_size", "{verb}: {error:?}");
        assert_eq!(error.message, MESSAGE, "{verb}");
        assert_eq!(error.suggested_action.as_deref(), Some(ACTION), "{verb}");
    }
    // Nothing was rendered or queued.
    let jobs = dispatch(&state, "jobs.list", json!({}), test_actor()).await;
    let kinds: Vec<String> = jobs.result.unwrap()["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["kind"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        !kinds.iter().any(|k| k == "render" || k == "render_queue"),
        "{kinds:?}"
    );

    // An export with its own even size is not refused.
    let own = dispatch(
        &state,
        "render.final",
        json!({"dry_run": true, "aspect": "16:9"}),
        test_actor(),
    )
    .await;
    assert!(own.ok, "{:?}", own.error);
    // A still works and says it is 1 px smaller.
    let still = dispatch(
        &state,
        "export.frame",
        json!({"at_ms": 500, "to_asset": false}),
        test_actor(),
    )
    .await;
    assert!(still.ok, "{:?}", still.error);
    assert_eq!(
        still.result.unwrap()["note"],
        "The project is 641x361; this still is 640x360 because pictures are composed with even sides."
    );
    // The frame rate can still be changed without touching the size.
    let fps = dispatch(&state, "project.format", json!({"fps": 25}), test_actor()).await;
    assert!(fps.ok, "{:?}", fps.error);
}

#[tokio::test]
async fn the_one_call_in_the_refusal_makes_the_project_exportable() {
    let root = tempfile::tempdir().unwrap();
    let state = odd_project(root.path()).await;
    let refused = dispatch(
        &state,
        "render.final",
        json!({"dry_run": true}),
        test_actor(),
    )
    .await;
    let action = refused.error.unwrap().suggested_action.unwrap();
    // The action is the call itself: verb, then its JSON args.
    let (verb, args) = action.split_once(' ').unwrap();
    let args: Value = serde_json::from_str(args).unwrap();
    let changed = dispatch(&state, verb, args, test_actor()).await;
    assert!(changed.ok, "{:?}", changed.error);

    let rendered = dispatch(
        &state,
        "render.final",
        json!({"hardware": "off"}),
        test_actor(),
    )
    .await;
    assert!(rendered.ok, "{:?}", rendered.error);
    let job = wait_job(
        &state,
        rendered.result.unwrap()["job_id"].as_str().unwrap(),
        120,
    )
    .await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let result = job.result.unwrap();
    assert!(
        result["encode"].get("adjustments").is_none(),
        "no quiet note"
    );
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0"])
        .args(["-show_entries", "stream=width,height", "-of", "csv=p=0"])
        .arg(result["path"].as_str().unwrap())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "640,360");
}

/// Reopening a project whose journal holds an odd size within the limits (as
/// an older release left it, and as the coverage fixture writes it) keeps the
/// size: `project.open` reports no reduction, so the odd-size notice and the
/// up-front refusal stay reachable.
#[tokio::test]
async fn reopening_an_odd_project_keeps_its_size_and_reports_no_reduction() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let dir = root.path().join("odd-reopen.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "odd-reopen", "dir": dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let closed = dispatch(&state, "project.close", json!({}), test_actor()).await;
    assert!(closed.ok, "{:?}", closed.error);
    let journal = dir.join("ops.jsonl");
    let text = std::fs::read_to_string(&journal).unwrap();
    let rewritten: String = text
        .lines()
        .map(|line| {
            let mut record: Value = serde_json::from_str(line).unwrap();
            if record["verb"] == "project.create" {
                record["args"]["settings"]["width"] = json!(641);
                record["args"]["settings"]["height"] = json!(361);
            }
            format!("{}\n", serde_json::to_string(&record).unwrap())
        })
        .collect();
    std::fs::write(&journal, rewritten).unwrap();
    let _ = std::fs::remove_file(dir.join("project.json"));

    let opened = dispatch(&state, "project.open", json!({"path": dir}), test_actor()).await;
    assert!(opened.ok, "{:?}", opened.error);
    let warnings = opened.warnings.clone().unwrap_or_default();
    assert!(
        !warnings.iter().any(|w| w.code == "project_format_reduced"),
        "{warnings:?}"
    );
    let result = opened.result.unwrap();
    assert_eq!(result["project"]["settings"]["width"], 641);
    assert_eq!(result["project"]["settings"]["height"], 361);
    let refused = dispatch(
        &state,
        "render.final",
        json!({"dry_run": true}),
        test_actor(),
    )
    .await;
    let error = refused
        .error
        .expect("the odd project's export is refused first");
    assert_eq!(error.code, "odd_project_size", "{error:?}");
    assert_eq!(error.message, MESSAGE);
}

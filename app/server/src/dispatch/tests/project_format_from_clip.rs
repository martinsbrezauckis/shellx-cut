//! The first clip sets the project format truthfully, and every size limit is
//! the same one everywhere.
//!
//! The HQ runs found an 8192x4096 clip leaving the project at 1920x1080@30
//! with no word (the project cap was 7680 and the refusal was dropped), while
//! `resolution:"match_source"` rendered 8192x4096 past a schema that said 7680,
//! and a variable-frame-rate phone clip gave a 58.591 fps timeline.

use super::{test_actor, wait_job};
use crate::dispatch::dispatch;
use crate::jobs::JobState;
use crate::state::AppState;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn make(dir: &Path, name: &str, lavfi: &str, extra: &[&str]) -> PathBuf {
    let out = dir.join(name);
    let made = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", lavfi])
        .args(extra)
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&out)
        .status()
        .unwrap();
    assert!(made.success(), "ffmpeg could not make {name}");
    out
}

async fn new_project(root: &Path, name: &str) -> AppState {
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": name, "dir": root.join(format!("{name}.cutproj"))}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    state
}

/// Import into the empty project; return the import job and the settings.
async fn import_first(state: &AppState, media: &Path) -> (crate::jobs::JobRecord, Value) {
    let imported = dispatch(
        state,
        "media.import",
        json!({"path": media, "proxy": false}),
        test_actor(),
    )
    .await;
    assert!(imported.ok, "{:?}", imported.error);
    let job = wait_job(
        state,
        imported.result.unwrap()["job_id"].as_str().unwrap(),
        60,
    )
    .await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let s = dispatch(state, "project.state", json!({}), test_actor()).await;
    (job, s.result.unwrap()["settings"].clone())
}

#[tokio::test]
async fn an_8192x4096_clip_sets_the_project_and_match_source_agrees() {
    let root = tempfile::tempdir().unwrap();
    let state = new_project(root.path(), "eight-k").await;
    let media = make(
        root.path(),
        "nasa-like.mp4",
        "testsrc2=size=8192x4096:rate=24000/1001:duration=0.25",
        &[],
    );
    let (job, settings) = import_first(&state, &media).await;
    let format = &job.result.as_ref().unwrap()["format"];
    assert_eq!(format["adopted"], true, "{format}");
    assert_eq!(
        (settings["width"].as_u64(), settings["height"].as_u64()),
        (Some(8192), Some(4096))
    );
    assert!(
        (settings["fps"].as_f64().unwrap() - 24_000.0 / 1001.0).abs() < 1e-6,
        "{settings}"
    );
    let plan = dispatch(
        &state,
        "render.final",
        json!({"dry_run": true}),
        test_actor(),
    )
    .await;
    assert!(plan.ok, "{:?}", plan.error);
    assert_eq!(plan.result.unwrap()["output"]["width"], 8192);
    let plan = dispatch(
        &state,
        "render.final",
        json!({"dry_run": true, "resolution": "match_source"}),
        test_actor(),
    )
    .await;
    assert!(plan.ok, "{:?}", plan.error);
}

#[tokio::test]
async fn a_clip_beyond_the_frame_limit_keeps_the_project_and_says_why() {
    let root = tempfile::tempdir().unwrap();
    let state = new_project(root.path(), "too-big").await;
    let media = make(
        root.path(),
        "huge.mp4",
        "testsrc2=size=10240x4320:rate=24:duration=0.25",
        &[],
    );
    let (job, settings) = import_first(&state, &media).await;
    let result = job.result.as_ref().unwrap();
    let format = &result["format"];
    assert_eq!(format["adopted"], false, "{format}");
    let reason = format["reason"].as_str().unwrap();
    assert!(reason.contains("10240x4320"), "{reason}");
    assert!(reason.contains("8192x4320"), "{reason}");
    assert_eq!(
        result["warnings"][0], reason,
        "the job carries it as a warning"
    );
    assert_eq!(
        job.completion,
        Some(crate::jobs::JobCompletion::DoneWithWarnings),
        "the job ends with warnings, not plain done"
    );
    assert_eq!(settings["width"], 1920);
    let refused = dispatch(
        &state,
        "render.final",
        json!({"dry_run": true, "resolution": "match_source"}),
        test_actor(),
    )
    .await;
    assert!(
        !refused.ok,
        "a frame beyond the limit must not reach the encoder"
    );
    let e = refused.error.unwrap();
    assert_eq!(e.code, "invalid_args");
    assert!(
        e.message.contains("match_source") && e.message.contains("10240x4320"),
        "{}",
        e.message
    );
}

#[tokio::test]
async fn a_variable_frame_rate_clip_gives_its_nominal_rate_and_says_so() {
    let root = tempfile::tempdir().unwrap();
    let state = new_project(root.path(), "vfr").await;
    // Two of every 30 frames dropped with real timestamps: r 60/1, avg 56/1.
    let media = make(
        root.path(),
        "phone.mp4",
        "testsrc2=size=320x180:rate=60:duration=3",
        &[
            "-vf",
            "select=not(eq(mod(n\\,30)\\,7))*not(eq(mod(n\\,30)\\,22))",
            "-fps_mode",
            "vfr",
        ],
    );
    let (job, settings) = import_first(&state, &media).await;
    assert_eq!(settings["fps"], 60.0, "{settings}");
    let result = job.result.as_ref().unwrap();
    assert_eq!(result["variable_frame_rate"], true);
    let note = result["format"]["note"].as_str().unwrap();
    assert_eq!(
        note,
        "This clip has a variable frame rate (56 fps on average, 60 fps nominal), so the timeline uses 60 fps."
    );
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let probe = s.result.unwrap()["assets"]["a1"]["probe"].clone();
    assert_eq!(probe["variable_frame_rate"], true);
    assert_eq!(probe["nominal_fps"], 60.0);
}

#[tokio::test]
async fn an_odd_sized_clip_gives_an_even_project_that_can_render() {
    let root = tempfile::tempdir().unwrap();
    let state = new_project(root.path(), "odd").await;
    // libx264 needs even sides for 4:2:0 (and testsrc2 rounds to even), so
    // scale to an odd size and encode 4:4:4.
    let out = root.path().join("odd.mp4");
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30:duration=0.5",
        ])
        .args(["-vf", "scale=641:361"])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv444p",
        ])
        .arg(&out)
        .status()
        .unwrap();
    assert!(made.success());
    let (job, settings) = import_first(&state, &out).await;
    assert_eq!(
        (settings["width"].as_u64(), settings["height"].as_u64()),
        (Some(640), Some(360))
    );
    let result = job.result.clone().unwrap();
    let note = result["format"]["note"]
        .as_str()
        .unwrap_or_else(|| panic!("{result}"))
        .to_string();
    assert!(
        note.contains("641x361") && note.contains("640x360"),
        "{note}"
    );
    let rendered = dispatch(
        &state,
        "render.final",
        json!({"hardware": "off"}),
        test_actor(),
    )
    .await;
    assert!(rendered.ok, "{:?}", rendered.error);
    let render = wait_job(
        &state,
        rendered.result.unwrap()["job_id"].as_str().unwrap(),
        120,
    )
    .await;
    assert_eq!(render.state, JobState::Done, "{render:?}");
}

#[tokio::test]
async fn one_frame_limit_for_project_format_and_render_size() {
    let root = tempfile::tempdir().unwrap();
    let state = new_project(root.path(), "limits").await;
    let ok = dispatch(
        &state,
        "project.format",
        json!({"width": 8192, "height": 4096}),
        test_actor(),
    )
    .await;
    assert!(ok.ok, "8192x4096 is within the limit: {:?}", ok.error);
    for (w, h, needle) in [(8192, 8192, "too large"), (641, 360, "odd side")] {
        let r = dispatch(
            &state,
            "project.format",
            json!({"width": w, "height": h}),
            test_actor(),
        )
        .await;
        assert!(!r.ok, "{w}x{h}");
        let e = r.error.unwrap();
        assert!(e.message.contains(needle), "{w}x{h}: {}", e.message);
    }
    let media = make(
        root.path(),
        "clip.mp4",
        "testsrc2=size=320x180:rate=30:duration=0.5",
        &[],
    );
    let imported = dispatch(
        &state,
        "media.import",
        json!({"path": media, "proxy": false}),
        test_actor(),
    )
    .await;
    let job = wait_job(
        &state,
        imported.result.unwrap()["job_id"].as_str().unwrap(),
        60,
    )
    .await;
    assert_eq!(job.state, JobState::Done);
    let inserted = dispatch(
        &state,
        "edit.insert",
        json!({"asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [0, 500]}),
        test_actor(),
    )
    .await;
    assert!(inserted.ok, "{:?}", inserted.error);
    let plan = |args: Value| {
        let state = state.clone();
        async move { dispatch(&state, "render.final", args, test_actor()).await }
    };
    assert!(
        plan(json!({"dry_run": true, "width": 8192, "height": 4320}))
            .await
            .ok
    );
    let r = plan(json!({"dry_run": true, "width": 8192, "height": 8192})).await;
    assert!(r.error.unwrap().message.contains("8192x8192 is too large"));
    let r = plan(
        json!({"dry_run": true, "width": 32, "height": 32, "format": "av1", "hardware": "off"}),
    )
    .await;
    assert!(r.error.unwrap().message.contains("libsvtav1"));
}

#[tokio::test]
async fn explicitly_created_default_format_survives_a_four_k_import_and_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("chosen.cutproj");
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({
            "name": "chosen", "dir": path,
            "settings": cut_core::ProjectSettings::default(),
        }),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let opened = dispatch(&state, "project.open", json!({"path": path}), test_actor()).await;
    assert!(opened.ok, "{:?}", opened.error);
    let media = make(
        root.path(),
        "four-k.mp4",
        "color=size=3840x2160:rate=24:duration=0.1",
        &[],
    );
    let (job, settings) = import_first(&state, &media).await;
    assert_eq!(settings["width"], 1920, "{settings}");
    assert_eq!(settings["height"], 1080);
    assert_eq!(settings["fps"], 30.0);
    assert_eq!(job.result.unwrap()["format"]["adopted"], false);
}

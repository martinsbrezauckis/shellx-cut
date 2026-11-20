//! A file that is not readable media never stays in the project.
//!
//! Text renamed `.mp4`/`.png`, an empty file, a folder: the verb or its import
//! job says plainly which file and why, with the stable `unreadable_media`
//! code, and no asset is left behind. Relink refuses the same files the same way.

use super::{test_actor, wait_job};
use crate::dispatch::dispatch;
use crate::jobs::JobState;
use crate::state::AppState;
use serde_json::{json, Value};
use std::path::Path;

async fn open_project(state: &AppState, root: &Path) {
    let created = dispatch(
        state,
        "project.create",
        json!({"name": "unreadable", "dir": root.join("unreadable.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
}

async fn assets(state: &AppState) -> Value {
    let s = dispatch(state, "project.state", json!({}), test_actor()).await;
    assert!(s.ok, "{:?}", s.error);
    s.result.unwrap()["assets"].clone()
}

async fn op_verbs(state: &AppState) -> Vec<String> {
    let ops = dispatch(state, "project.ops", json!({}), test_actor()).await;
    ops.result.unwrap()["ops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| op["verb"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn text_bytes() -> Vec<u8> {
    b"hello this is not a video\n".repeat(200)
}

#[tokio::test]
async fn a_text_file_named_like_media_is_undone_with_a_plain_reason() {
    for name in ["textfile.mp4", "textfile.png"] {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::new();
        open_project(&state, root.path()).await;
        let media = root.path().join(name);
        std::fs::write(&media, text_bytes()).unwrap();
        let imported = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
        assert!(
            imported.ok,
            "the probe runs in the import job: {:?}",
            imported.error
        );
        let result = imported.result.unwrap();
        let job_id = result["job_id"].as_str().unwrap().to_string();
        let asset_id = result["asset_id"].as_str().unwrap().to_string();
        let job = wait_job(&state, &job_id, 30).await;
        assert_eq!(job.state, JobState::Failed, "{name}: {job:?}");
        let error = job.error.expect("a failed import names its error");
        assert_eq!(error.code, "unreadable_media", "{name}: {error:?}");
        assert!(
            error.message.starts_with(&format!("{name} cannot be used")),
            "{name}: the person reads which file: {}",
            error.message
        );
        assert!(
            error.message.contains("not a video, audio or image file"),
            "{}",
            error.message
        );
        assert!(
            error.cause.contains("the import was undone"),
            "{name}: the agent learns nothing stayed behind: {}",
            error.cause
        );
        assert!(
            error
                .suggested_action
                .as_deref()
                .is_some_and(|a| a.contains("media player")),
            "{name}: a next step a person can follow: {error:?}"
        );
        assert!(
            assets(&state).await.get(&asset_id).is_none(),
            "{name}: no dead asset stays in Assets"
        );
        assert_eq!(
            op_verbs(&state).await,
            ["project.create", "media.import", "media.remove"],
            "{name}: the undo is an ordinary, replay-safe op"
        );
    }
}

#[tokio::test]
async fn an_empty_file_and_a_folder_are_refused_before_anything_is_registered() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    open_project(&state, root.path()).await;
    let empty = root.path().join("empty.mp4");
    std::fs::write(&empty, b"").unwrap();
    let refused = dispatch(&state, "media.import", json!({"path": empty}), test_actor()).await;
    assert!(!refused.ok);
    let error = refused.error.unwrap();
    assert_eq!(error.code, "unreadable_media");
    assert!(error.message.contains("empty.mp4"), "{}", error.message);
    assert!(
        error.message.contains("empty (0 bytes)"),
        "{}",
        error.message
    );
    assert!(error.suggested_action.is_some());

    let folder = root.path().join("clips.mp4");
    std::fs::create_dir(&folder).unwrap();
    let refused = dispatch(
        &state,
        "media.import",
        json!({"path": folder}),
        test_actor(),
    )
    .await;
    assert!(!refused.ok);
    let error = refused.error.unwrap();
    assert_eq!(error.code, "invalid_args");
    assert!(error.message.contains("is a folder"), "{}", error.message);

    assert_eq!(assets(&state).await, json!({}));
    assert_eq!(op_verbs(&state).await, ["project.create"]);
}

#[tokio::test]
async fn relink_to_a_file_that_is_not_media_is_refused_and_keeps_the_source() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    open_project(&state, root.path()).await;
    let clip = root.path().join("clip.mp4");
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=160x90:r=30",
            "-t",
            "0.5",
        ])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-y"])
        .arg(&clip)
        .status()
        .unwrap();
    assert!(made.success());
    let imported = dispatch(&state, "media.import", json!({"path": clip}), test_actor()).await;
    assert!(imported.ok, "{:?}", imported.error);
    let result = imported.result.unwrap();
    let job = wait_job(&state, result["job_id"].as_str().unwrap(), 30).await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let before = assets(&state).await["a1"]["path"].clone();

    let text = root.path().join("replacement.mp4");
    std::fs::write(&text, text_bytes()).unwrap();
    let relinked = dispatch(
        &state,
        "media.relink",
        json!({"asset": "a1", "path": text}),
        test_actor(),
    )
    .await;
    assert!(!relinked.ok, "a text file must not become the source");
    let error = relinked.error.unwrap();
    assert_eq!(error.code, "unreadable_media", "{error:?}");
    assert!(error.message.starts_with("replacement.mp4 cannot be used"));
    assert_eq!(assets(&state).await["a1"]["path"], before);
}

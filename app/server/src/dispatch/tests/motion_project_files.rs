//! Motion's default output folder lives inside the project. A received
//! project with a link at `motion/` must not redirect the script Cut writes,
//! or the folder handed to the Motion tool, outside the project.

use super::test_actor;
use crate::dispatch::dispatch;
use crate::state::AppState;
use serde_json::json;
use std::path::{Path, PathBuf};

async fn project_with_linked_motion_folder(root: &Path) -> (AppState, PathBuf) {
    let state = AppState::new();
    let project = root.join("t.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "t", "dir": &project}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let outside = root.join("outside-folder");
    std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, project.join("motion")).unwrap();
    (state, outside)
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect()
}

#[cfg(unix)]
#[tokio::test]
async fn linked_motion_folder_receives_no_script() {
    let dir = tempfile::tempdir().unwrap();
    let (state, outside) = project_with_linked_motion_folder(dir.path()).await;

    let response = dispatch(
        &state,
        "motion.script_to_cut",
        json!({"script": {"id": "linked"}, "policy": "preview"}),
        test_actor(),
    )
    .await;

    assert!(entries(&outside).is_empty(), "{:?}", entries(&outside));
    let error = response.error.expect("a linked motion folder is refused");
    assert!(error.message.contains("not a plain directory"), "{error:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn linked_motion_folder_is_refused_before_the_motion_tool_runs() {
    let dir = tempfile::tempdir().unwrap();
    let (state, outside) = project_with_linked_motion_folder(dir.path()).await;

    let response = dispatch(
        &state,
        "motion.template_to_cut",
        json!({"template": "lower-third", "policy": "preview"}),
        test_actor(),
    )
    .await;

    assert!(entries(&outside).is_empty(), "{:?}", entries(&outside));
    let error = response.error.expect("a linked motion folder is refused");
    assert!(error.message.contains("not a plain directory"), "{error:?}");
}

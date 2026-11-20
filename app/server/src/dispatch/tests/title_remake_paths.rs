//! Security review of the automatic title remake: a project received from
//! someone else records attacker-chosen asset paths. Drawing a missing title
//! again (at open, before an exact frame, render or export) must never create
//! or overwrite a file outside the project's own `titles` folder.

use super::test_actor;
use crate::dispatch::dispatch;
use crate::state::AppState;
use serde_json::json;
use std::path::{Path, PathBuf};

fn has_ffmpeg() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok()
}

/// A project with one placed title; returns (state, project dir, title asset id, movie path).
async fn titled(root: &Path) -> (AppState, PathBuf, String, PathBuf) {
    let state = AppState::new();
    let dir = root.join("received.cutproj");
    let made = dispatch(
        &state,
        "project.create",
        json!({"name": "received", "dir": dir, "cancel_running_jobs": true}),
        test_actor(),
    )
    .await;
    assert!(made.ok, "{:?}", made.error);
    let added = dispatch(
        &state,
        "title.add",
        json!({"text": "TITLE 147", "range_ms": [0, 1000], "preset": "title_card"}),
        test_actor(),
    )
    .await;
    assert!(added.ok, "{:?}", added.error);
    let id = added.result.unwrap()["asset_id"]
        .as_str()
        .unwrap()
        .to_string();
    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    let movie = PathBuf::from(&store.project.assets[&id].path);
    let dir = store.dir.clone();
    drop(guard);
    (state, dir, id, movie)
}

fn files_under(dir: &Path) -> usize {
    std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
}

#[tokio::test]
async fn a_recorded_path_outside_the_titles_folder_is_never_written() {
    if !has_ffmpeg() {
        eprintln!("skipped: ffmpeg not found");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let name = "title_0123456789abcdef.mov";
    // Each form, if followed naively, lands in <case>/escape (outside the project).
    let mut forms: Vec<Box<dyn Fn(&Path, &Path) -> String>> = vec![
        Box::new(move |_, _| format!("titles/../../escape/{name}")),
        Box::new(move |case, _| case.join("escape").join(name).display().to_string()),
        Box::new(move |_, dir| format!("{}/titles/../../escape/{name}", dir.display())),
        Box::new(move |_, _| format!(r"titles\..\..\escape\{name}")),
    ];
    if cfg!(unix) {
        // Spellings a text comparison would take for the titles folder: on Unix
        // a backslash is part of a file name, and `//?/` is a folder (on
        // Windows both are the titles folder itself, correctly).
        forms.push(Box::new(move |_, dir| {
            format!(r"{}/titles\{name}", dir.display())
        }));
        forms.push(Box::new(move |_, dir| {
            format!("//?/{}/titles/{name}", dir.display())
        }));
    }
    for (n, form) in forms.iter().enumerate() {
        let case = root.path().join(format!("case{n}"));
        std::fs::create_dir_all(&case).unwrap();
        let (state, dir, id, movie) = titled(&case).await;
        std::fs::remove_file(&movie).unwrap();
        let beside = files_under(&dir);
        let recorded = form(&case, &dir);
        state
            .project
            .write()
            .await
            .as_mut()
            .unwrap()
            .project
            .assets
            .get_mut(&id)
            .unwrap()
            .path = recorded.clone();
        let result = crate::dispatch::edit_tools::ensure_overlay_movies(&state).await;
        assert!(
            !case.join("escape").exists(),
            "{recorded}: nothing is created outside the project"
        );
        assert_eq!(
            files_under(&dir.join("titles")),
            0,
            "{recorded}: nothing written"
        );
        assert_eq!(
            files_under(&dir),
            beside,
            "{recorded}: nothing written beside the titles folder"
        );
        if let Err(error) = result {
            assert!(
                error.message.contains("could not be drawn"),
                "{recorded}: {}",
                error.message
            );
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_linked_titles_folder_or_a_link_at_the_movie_is_never_written_through() {
    if !has_ffmpeg() {
        eprintln!("skipped: ffmpeg not found");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    // The titles folder replaced by a link to a folder outside the project.
    let (state, dir, _id, movie) = titled(root.path()).await;
    let outside = root.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::rename(dir.join("titles"), root.path().join("titles-moved")).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("titles")).unwrap();
    let error = crate::dispatch::edit_tools::ensure_overlay_movies(&state)
        .await
        .unwrap_err();
    assert!(
        error.message.contains("could not be drawn"),
        "{}",
        error.message
    );
    assert_eq!(
        files_under(&outside),
        0,
        "nothing written through the linked folder"
    );

    // A link planted at the movie's own name, pointing outside: present, not followed.
    let root = tempfile::tempdir().unwrap();
    let (state, _dir, _id, movie2) = titled(root.path()).await;
    let victim = root.path().join("victim.mov");
    std::fs::remove_file(&movie2).unwrap();
    std::os::unix::fs::symlink(&victim, &movie2).unwrap();
    let _ = crate::dispatch::edit_tools::ensure_overlay_movies(&state).await;
    assert!(!victim.exists(), "the link target is never created");
    let _ = movie;
}

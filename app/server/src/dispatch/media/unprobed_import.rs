//! Undo an import whose probe failed.
//!
//! `media.import` registers the asset before its job runs ffprobe. When the
//! probe fails, the asset has no facts: either the tool disappeared between the
//! verb and its job (a removed or broken install), or ffprobe read the file and
//! it is not media Cut can use (a renamed text file, a damaged download). In
//! both cases the registration is undone with a normal, replay-safe
//! `media.remove` op, so the project never keeps an asset that cannot be used.
//! Only an asset that is still unprobed, unused and in the same project is
//! removed; anything else is left alone and the job failure still says why.

use super::*;
use std::path::Path;

/// Remove `asset_id` from the project at `project_dir` when it is still the
/// unprobed, unplaced asset this import registered. Returns true when removed.
pub(super) async fn undo_unprobed_import(
    state: &AppState,
    project_dir: &Path,
    asset_id: &str,
    reason: &CutError,
) -> bool {
    let removed = {
        let mut guard = state.project.write().await;
        let Some(store) = guard.as_mut() else {
            return false;
        };
        let untouched = store.dir == project_dir
            && store
                .project
                .assets
                .get(asset_id)
                .is_some_and(|asset| asset.probe.is_none())
            && !store
                .project
                .all_sequence_tracks()
                .flat_map(|track| &track.clips)
                .any(|clip| matches!(clip, cut_core::Clip::Media(m) if m.asset == asset_id));
        if !untouched {
            return false;
        }
        store.record_remove_asset(
            asset_id,
            Actor::system(),
            Some(format!("import undone: {}", reason.message)),
        )
    };
    match removed {
        Ok((_, op)) => {
            state.events.publish(Event::OpApplied { op });
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn agent() -> Actor {
        Actor {
            kind: cut_core::ActorKind::Agent,
            name: "test".into(),
            via: "test".into(),
            request: None,
        }
    }

    async fn project_with_unprobed_asset(root: &Path) -> (AppState, PathBuf, String) {
        let state = AppState::new();
        let created = crate::dispatch::dispatch(
            &state,
            "project.create",
            json!({"name": "undo", "dir": root.join("undo.cutproj")}),
            agent(),
        )
        .await;
        assert!(created.ok, "{:?}", created.error);
        let mut guard = state.project.write().await;
        let store = guard.as_mut().expect("open project");
        let dir = store.dir.clone();
        let asset = cut_core::Asset {
            path: root.join("clip.mp4").display().to_string(),
            hash: "sha256:00".into(),
            probe: None,
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        };
        let (asset_id, _) = store
            .record_import(None, asset, agent(), None)
            .expect("import");
        drop(guard);
        (state, dir, asset_id)
    }

    fn damaged_picture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../media/tests/fixtures/original-still.png")
    }

    #[tokio::test]
    async fn damaged_picture_is_refused_before_import_is_recorded() {
        let root = tempfile::tempdir().unwrap();
        let (state, _, _) = project_with_unprobed_asset(root.path()).await;
        let before = state
            .project
            .read()
            .await
            .as_ref()
            .unwrap()
            .project
            .assets
            .len();
        let result = media_import(
            &state,
            json!({"path": damaged_picture(), "proxy": false}),
            agent(),
        )
        .await;
        assert!(result.is_err(), "damaged still was admitted");
        assert_eq!(result.unwrap_err().code, error_codes::UNREADABLE_MEDIA);
        assert_eq!(
            state
                .project
                .read()
                .await
                .as_ref()
                .unwrap()
                .project
                .assets
                .len(),
            before
        );
    }

    #[tokio::test]
    async fn existing_damaged_picture_is_reported_unreadable_not_missing() {
        let root = tempfile::tempdir().unwrap();
        let (state, dir, _) = project_with_unprobed_asset(root.path()).await;
        let id = {
            let mut guard = state.project.write().await;
            let asset = cut_core::Asset {
                path: damaged_picture().display().to_string(),
                hash: cut_core::hash_file(&damaged_picture()).unwrap(),
                probe: Some(json!({"kind":"image", "width":320, "height":180})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            };
            guard
                .as_mut()
                .unwrap()
                .record_import(None, asset, agent(), None)
                .unwrap()
                .0
        };
        let opened =
            crate::dispatch::dispatch(&state, "project.open", json!({"path":dir}), agent()).await;
        assert!(opened.ok, "{:?}", opened.error);
        let result = media_check(&state, json!({"asset":id})).await.unwrap();
        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["result"]["assets"][0]["exists"], true);
        assert_eq!(value["result"]["assets"][0]["readable"], false);
    }

    #[tokio::test]
    async fn an_import_whose_probe_cannot_start_leaves_no_asset() {
        let root = tempfile::tempdir().unwrap();
        let (state, dir, asset_id) = project_with_unprobed_asset(root.path()).await;
        let reason = cut_media::tool_setup::missing_tool_error("ffprobe");
        assert!(undo_unprobed_import(&state, &dir, &asset_id, &reason).await);
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        assert!(!store.project.assets.contains_key(&asset_id));
    }

    #[tokio::test]
    async fn another_project_or_a_probed_asset_is_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let (state, dir, asset_id) = project_with_unprobed_asset(root.path()).await;
        let reason = cut_media::tool_setup::missing_tool_error("ffprobe");
        let elsewhere = root.path().join("other.cutproj");
        assert!(!undo_unprobed_import(&state, &elsewhere, &asset_id, &reason).await);
        {
            let mut guard = state.project.write().await;
            let store = guard.as_mut().unwrap();
            store.project.assets.get_mut(&asset_id).unwrap().probe = Some(json!({}));
        }
        assert!(!undo_unprobed_import(&state, &dir, &asset_id, &reason).await);
        let guard = state.project.read().await;
        assert!(guard
            .as_ref()
            .unwrap()
            .project
            .assets
            .contains_key(&asset_id));
    }
}

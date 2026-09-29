//! Grouped clip deletion and automatic empty-track cleanup.

use super::*;

/// A Timeline delete of the last overlay clip auto-cleans that track. Both
/// schema-validated operations must share one history step, including redo.
#[tokio::test]
async fn overlay_clip_delete_and_track_cleanup_undo_redo_as_one() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir":dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"x").unwrap();
    dispatch(&state, "media.import", json!({"path":media}), test_actor()).await;
    update_asset(&state, "a1", |a| {
        a.probe = Some(
            json!({"kind":"video","width":1920,"height":1080,"duration_ms":6000,"has_audio":false}),
        );
    })
    .await
    .unwrap();
    let add = dispatch(
        &state,
        "edit.add_track",
        json!({"kind":"video"}),
        test_actor(),
    )
    .await;
    assert!(add.ok, "add overlay: {:?}", add.error);
    let insert = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"v2","at_ms":0,"src_range_ms":[0,6000]}),
        test_actor(),
    )
    .await;
    assert!(insert.ok, "insert overlay clip: {:?}", insert.error);

    let deleted = dispatch(
        &state,
        "edit.ripple_delete",
        json!({"track":"v2","range_ms":[0,6000],"ripple":true,"group_id":"overlay-delete-1"}),
        test_actor(),
    )
    .await;
    assert!(deleted.ok, "delete overlay clip: {:?}", deleted.error);
    let removed = dispatch(
        &state,
        "edit.remove_track",
        json!({"track":"v2","force":true,"group_id":"overlay-delete-1"}),
        test_actor(),
    )
    .await;
    assert!(
        removed.ok,
        "grouped auto-clean must pass schema: {:?}",
        removed.error
    );

    let state_project =
        |value: serde_json::Value| -> cut_core::Project { serde_json::from_value(value).unwrap() };
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    assert!(state_project(s.result.unwrap()).track("v2").is_none());
    let undo = dispatch(&state, "project.undo", json!({}), test_actor()).await;
    assert!(undo.ok, "one Undo: {:?}", undo.error);
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project = state_project(s.result.unwrap());
    assert_eq!(
        project.track("v2").unwrap().clips.len(),
        1,
        "one Undo restores both track and clip"
    );
    let redo = dispatch(&state, "project.redo", json!({}), test_actor()).await;
    assert!(redo.ok, "one Redo: {:?}", redo.error);
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    assert!(
        state_project(s.result.unwrap()).track("v2").is_none(),
        "one Redo removes both again"
    );
}

/// Linked media on two extra tracks may empty both lanes. Their two deletes
/// and two cleanup operations remain one adjacent durable user action.
#[tokio::test]
async fn linked_overlay_pair_cleanup_undo_redo_as_one() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir":dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"x").unwrap();
    dispatch(&state, "media.import", json!({"path":media}), test_actor()).await;
    update_asset(&state, "a1", |a| {
        a.probe = Some(
            json!({"kind":"video","width":1920,"height":1080,"duration_ms":6000,"has_audio":true}),
        );
    })
    .await
    .unwrap();
    for kind in ["video", "audio"] {
        let r = dispatch(&state, "edit.add_track", json!({"kind":kind}), test_actor()).await;
        assert!(r.ok, "add {kind} overlay: {:?}", r.error);
    }
    for track in ["v2", "a2t"] {
        let r = dispatch(
            &state,
            "edit.insert",
            json!({"asset":"a1","track":track,"at_ms":0,"src_range_ms":[0,6000]}),
            test_actor(),
        )
        .await;
        assert!(r.ok, "insert {track}: {:?}", r.error);
    }
    for track in ["v2", "a2t"] {
        let r = dispatch(&state, "edit.ripple_delete", json!({"track":track,"range_ms":[0,6000],"ripple":true,"group_id":"linked-overlay-delete"}), test_actor()).await;
        assert!(r.ok, "delete {track}: {:?}", r.error);
    }
    for track in ["v2", "a2t"] {
        let r = dispatch(
            &state,
            "edit.remove_track",
            json!({"track":track,"force":true,"group_id":"linked-overlay-delete"}),
            test_actor(),
        )
        .await;
        assert!(r.ok, "cleanup {track}: {:?}", r.error);
    }
    let read = async {
        let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
        serde_json::from_value::<cut_core::Project>(s.result.unwrap()).unwrap()
    };
    let project = read.await;
    assert!(project.track("v2").is_none() && project.track("a2t").is_none());
    let undo = dispatch(&state, "project.undo", json!({}), test_actor()).await;
    assert!(undo.ok, "one Undo: {:?}", undo.error);
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    for track in ["v2", "a2t"] {
        assert_eq!(
            project.track(track).unwrap().clips.len(),
            1,
            "one Undo restores {track} clip and lane"
        );
    }
    let redo = dispatch(&state, "project.redo", json!({}), test_actor()).await;
    assert!(redo.ok, "one Redo: {:?}", redo.error);
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    assert!(project.track("v2").is_none() && project.track("a2t").is_none());
}

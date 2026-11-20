//! No live edit leaves a media clip shorter than one frame (the routes beyond
//! the edit points of `edit_point_snap`): a trim, an inserted source range, a
//! speed change, a range edge, an overwrite end and a copy of an existing
//! sliver are refused with the reason and the nearest valid value. Before the
//! fix each of these made a clip of a few milliseconds.

use super::*;

fn asset() -> cut_core::Asset {
    cut_core::Asset {
        path: "/fixture/subframe.mp4".into(),
        hash: "sha256:subframe".into(),
        probe: Some(
            json!({"duration_ms": 60_000, "kind": "video", "has_audio": false, "fps": 30.0}),
        ),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    }
}

/// A 30 fps project (one frame = 33.3 ms) with v1 = c1 [0, 1000) + c2 [1000, 2000).
async fn two_clips(name: &str) -> AppState {
    let root = tempfile::tempdir().unwrap();
    let project_dir = root.keep().join(format!("{name}.cutproj"));
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": name, "dir": project_dir, "settings": {"width": 640, "height": 360, "fps": 30}}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    {
        let mut guard = state.project.write().await;
        guard
            .as_mut()
            .unwrap()
            .record_import(Some("a1".into()), asset(), test_actor(), None)
            .unwrap();
    }
    for (at, range) in [(0, [5_000, 6_000]), (1_000, [8_000, 9_000])] {
        let r = dispatch(
            &state,
            "edit.insert",
            json!({"asset": "a1", "track": "v1", "at_ms": at, "src_range_ms": range}),
            test_actor(),
        )
        .await;
        assert!(r.ok, "{:?}", r.error);
    }
    state
}

async fn refused(state: &AppState, verb: &str, args: Value, action: &str) {
    let r = dispatch(state, verb, args, test_actor()).await;
    assert!(!r.ok, "{verb} must be refused");
    let e = r.error.unwrap();
    assert!(
        e.message.contains("at least one frame: 33.3 ms at 30 fps"),
        "{verb}: {e:?}"
    );
    assert!(
        e.suggested_action.as_deref().unwrap_or("").contains(action),
        "{verb}: {e:?}"
    );
}

async fn clip_ids(state: &AppState) -> Vec<String> {
    let guard = state.project.read().await;
    guard.as_ref().unwrap().project.tracks[0]
        .clips
        .iter()
        .filter_map(|c| match c {
            cut_core::Clip::Media(m) => Some(m.id.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_trim_insert_speed_or_range_that_leaves_less_than_a_frame_is_refused() {
    let state = two_clips("subframe-routes").await;
    let ids = clip_ids(&state).await;
    let c1 = ids[0].clone();
    refused(
        &state,
        "edit.trim",
        json!({"clip": c1, "src_out_ms": 5_020}),
        "src_out_ms 5034",
    )
    .await;
    refused(
        &state,
        "edit.insert",
        json!({"asset": "a1", "track": "v1", "at_ms": 2_000, "src_range_ms": [100, 110]}),
        "source range of at least 34 ms",
    )
    .await;
    // c1 at 100 ms; at 4x it would be 25 ms.
    let short = dispatch(
        &state,
        "edit.trim",
        json!({"clip": c1, "src_out_ms": 5_100}),
        test_actor(),
    )
    .await;
    assert!(short.ok, "{:?}", short.error);
    refused(
        &state,
        "edit.speed",
        json!({"clip": c1, "factor": 4.0}),
        "at most 3.000",
    )
    .await;
    // A ripple delete ending 10 ms before c1's end leaves a 10 ms piece.
    refused(
        &state,
        "edit.ripple_delete",
        json!({"track": "v1", "range_ms": [0, 90]}),
        "one frame",
    )
    .await;
    assert_eq!(clip_ids(&state).await.len(), 2, "nothing changed");
    // A frame or more is fine.
    let ok = dispatch(
        &state,
        "edit.trim",
        json!({"clip": c1, "src_out_ms": 5_034}),
        test_actor(),
    )
    .await;
    assert!(ok.ok, "{:?}", ok.error);
}

#[tokio::test]
async fn a_copy_of_an_existing_sliver_is_refused_and_the_sliver_is_listed() {
    let state = two_clips("subframe-copy").await;
    // A sliver from an older project (made before this rule): edit the state directly.
    {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        if let cut_core::Clip::Media(m) = &mut store.project.tracks[0].clips[1] {
            m.src_out_ms = m.src_in_ms + 1;
        }
    }
    let ids = clip_ids(&state).await;
    let st = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let short = st.result.unwrap()["subframe_clips"].clone();
    assert_eq!(
        short,
        json!([{"clip": ids[1], "track": "v1", "at_ms": 1000, "duration_ms": 1}])
    );
    refused(
        &state,
        "edit.duplicate",
        json!({"clip": ids[1]}),
        "itself shorter than one frame",
    )
    .await;
    // An edit elsewhere is not blocked by the old sliver.
    let ok = dispatch(
        &state,
        "edit.trim",
        json!({"clip": ids[0], "src_out_ms": 5_500}),
        test_actor(),
    )
    .await;
    assert!(ok.ok, "{:?}", ok.error);
}

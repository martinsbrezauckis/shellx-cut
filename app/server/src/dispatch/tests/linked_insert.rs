//! Atomic linked A/V insertion: transaction, replay, undo, and ripple proof.

use super::*;

fn muxed_asset() -> cut_core::Asset {
    cut_core::Asset {
        path: "/fixture/linked.mp4".into(),
        hash: "sha256:linked".into(),
        probe: Some(json!({"duration_ms": 6_000, "kind": "video", "has_audio": true})),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    }
}

async fn linked_insert_state(name: &str) -> AppState {
    let root = tempfile::tempdir().unwrap();
    let project_dir = root.keep().join(format!("{name}.cutproj"));
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": name, "dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "project create failed: {:?}", created.error);
    let mut guard = state.project.write().await;
    guard
        .as_mut()
        .unwrap()
        .record_import(Some("a1".into()), muxed_asset(), test_actor(), None)
        .unwrap();
    drop(guard);
    state
}

fn linked_args(video_track: &str, audio_track: &str) -> Value {
    json!({
        "asset": "a1",
        "at_ms": 1_000,
        "video_track": video_track,
        "audio_track": audio_track,
        "src_range_ms": [100, 1_100],
        "ripple": true,
    })
}

#[tokio::test]
async fn linked_insert_is_one_event_one_op_replayable_and_one_undo() {
    let state = linked_insert_state("linked-existing").await;
    let mut events = state.events.subscribe();
    let inserted = dispatch(
        &state,
        "edit.insert_linked",
        linked_args("v1", "a1t"),
        test_actor(),
    )
    .await;
    assert!(inserted.ok, "linked insert failed: {:?}", inserted.error);
    assert_eq!(inserted.op_ids.as_ref().map(Vec::len), Some(1));
    let receipt = inserted.result.expect("linked insert receipt");
    assert_eq!(receipt["video_track"], "v1");
    assert_eq!(receipt["audio_track"], "a1t");
    assert_eq!(receipt["src_range_ms"], json!([100, 1_100]));
    assert_eq!(receipt["ripple"], true);

    let op_id = inserted.op_ids.unwrap().pop().unwrap();
    match events.try_recv() {
        Ok(crate::events::Event::OpApplied { op }) => {
            assert_eq!(op.op_id, op_id);
            assert_eq!(op.verb, "edit.insert_linked");
            assert_eq!(
                op.effects
                    .iter()
                    .filter(|effect| effect.detail.get("added_clip").is_some())
                    .count(),
                2,
                "the durable operation contains both linked clips",
            );
        }
        event => panic!("expected exactly one linked op event, got {event:?}"),
    }
    assert!(
        events.try_recv().is_err(),
        "a linked insertion cannot emit a second per-leg event"
    );

    let (live, log) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let video_clip_id = receipt["video_clip_id"].as_str().unwrap();
        let audio_clip_id = receipt["audio_clip_id"].as_str().unwrap();
        assert!(store.project.find_clip(video_clip_id).is_some());
        assert!(store.project.find_clip(audio_clip_id).is_some());
        (store.project.clone(), store.log.read_all().unwrap())
    };
    assert_eq!(
        cut_core::rebuild_from_log(&log).expect("linked insertion replay"),
        live,
        "reopen/replay keeps both legs as one durable edit",
    );

    let undone = dispatch(&state, "project.undo", json!({}), test_actor()).await;
    assert!(
        undone.ok,
        "one undo must remove both linked legs: {:?}",
        undone.error
    );
    let guard = state.project.read().await;
    let project = &guard.as_ref().unwrap().project;
    assert!(project.track("v1").unwrap().clips.is_empty());
    assert!(project.track("a1t").unwrap().clips.is_empty());
}

#[tokio::test]
async fn linked_insert_created_tracks_replay_with_distinct_pinned_ids() {
    let state = linked_insert_state("linked-new-tracks").await;
    let inserted = dispatch(
        &state,
        "edit.insert_linked",
        json!({
            "asset": "a1",
            "at_ms": 500,
            "create_video_track": true,
            "create_audio_track": true,
            "src_range_ms": [0, 500],
            "ripple": false,
        }),
        test_actor(),
    )
    .await;
    assert!(
        inserted.ok,
        "linked insert with destinations failed: {:?}",
        inserted.error
    );
    let receipt = inserted.result.unwrap();
    assert_eq!(receipt["created_video_track"], true);
    assert_eq!(receipt["created_audio_track"], true);
    assert_eq!(receipt["video_track"], "v2");
    assert_eq!(receipt["audio_track"], "a2t");

    let (project_dir, live, log) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        assert!(store.project.track("v2").is_some());
        assert!(store.project.track("a2t").is_some());
        (
            store.dir.clone(),
            store.project.clone(),
            store.log.read_all().unwrap(),
        )
    };
    assert_eq!(
        cut_core::rebuild_from_log(&log).expect("created linked track replay"),
        live,
        "replay consumes video and audio added_track pins in lowered-step order",
    );

    let reopened_state = AppState::new();
    let reopened = dispatch(
        &reopened_state,
        "project.open",
        json!({"path": project_dir}),
        test_actor(),
    )
    .await;
    assert!(
        reopened.ok,
        "linked created-track project reopen failed: {:?}",
        reopened.error
    );
    let guard = reopened_state.project.read().await;
    let reopened_project = &guard.as_ref().unwrap().project;
    assert!(
        reopened_project.track("v2").is_some(),
        "reopen preserves the pinned video destination id"
    );
    assert!(
        reopened_project.track("a2t").is_some(),
        "reopen preserves the pinned audio destination id"
    );
    assert!(reopened_project
        .find_clip(receipt["video_clip_id"].as_str().unwrap())
        .is_some());
    assert!(reopened_project
        .find_clip(receipt["audio_clip_id"].as_str().unwrap())
        .is_some());
}

#[tokio::test]
async fn linked_insert_rejects_locked_second_leg_without_project_log_or_event_change() {
    let state = linked_insert_state("linked-locked-audio").await;
    let locked = dispatch(
        &state,
        "edit.track_lock",
        json!({"track":"a1t", "on":true}),
        test_actor(),
    )
    .await;
    assert!(locked.ok, "audio lock setup failed: {:?}", locked.error);
    let (before_project, before_log) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        (store.project.clone(), store.log.read_all().unwrap())
    };
    let mut events = state.events.subscribe();

    let rejected = dispatch(
        &state,
        "edit.insert_linked",
        linked_args("v1", "a1t"),
        test_actor(),
    )
    .await;
    assert!(
        !rejected.ok,
        "a locked audio leg must reject the linked request"
    );
    assert_eq!(
        rejected.error.as_ref().map(|error| error.code.as_str()),
        Some(error_codes::CONFLICT),
    );

    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    assert_eq!(
        store.project, before_project,
        "a rejected second leg cannot leak a video clip or track"
    );
    assert_eq!(
        store.log.read_all().unwrap(),
        before_log,
        "a rejected linked request cannot mint an operation id"
    );
    assert!(
        events.try_recv().is_err(),
        "a rejected linked request cannot publish op_applied"
    );
}

async fn assert_exhausted_linked_destination_rejects_without_mutation(
    name: &str,
    setup: Value,
    args: Value,
) {
    let state = linked_insert_state(name).await;
    let added = dispatch(&state, "edit.add_track", setup, test_actor()).await;
    assert!(
        added.ok,
        "exhausted-id setup failed for {name}: {:?}",
        added.error
    );
    let (before_project, before_log) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        (store.project.clone(), store.log.read_all().unwrap())
    };
    let mut events = state.events.subscribe();

    let rejected = dispatch(&state, "edit.insert_linked", args, test_actor()).await;
    assert!(
        !rejected.ok,
        "an exhausted automatic linked destination must reject instead of overflowing"
    );
    assert_eq!(
        rejected.error.as_ref().map(|error| error.code.as_str()),
        Some(error_codes::CONFLICT),
    );

    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    assert_eq!(
        store.project, before_project,
        "an exhausted automatic id cannot leak a linked clip or track"
    );
    assert_eq!(
        store.log.read_all().unwrap(),
        before_log,
        "an exhausted automatic id cannot mint a linked operation"
    );
    assert!(
        events.try_recv().is_err(),
        "an exhausted automatic id cannot publish op_applied"
    );
}

#[tokio::test]
async fn linked_insert_rejects_exhausted_automatic_track_ids_without_panic() {
    assert_exhausted_linked_destination_rejects_without_mutation(
        "linked-exhausted-video-track-id",
        json!({"kind":"video", "id":"v18446744073709551615"}),
        json!({
            "asset":"a1",
            "at_ms":1_000,
            "create_video_track":true,
            "audio_track":"a1t",
            "src_range_ms":[100, 1_100],
            "ripple":true,
        }),
    )
    .await;
    assert_exhausted_linked_destination_rejects_without_mutation(
        "linked-exhausted-audio-track-id",
        json!({"kind":"audio", "id":"a18446744073709551615t"}),
        json!({
            "asset":"a1",
            "at_ms":1_000,
            "video_track":"v1",
            "create_audio_track":true,
            "src_range_ms":[100, 1_100],
            "ripple":true,
        }),
    )
    .await;
}

#[tokio::test]
async fn linked_insert_ripples_other_timeline_material_once() {
    let state = linked_insert_state("linked-ripple").await;
    let added_track = dispatch(
        &state,
        "edit.add_track",
        json!({"kind":"video"}),
        test_actor(),
    )
    .await;
    assert!(added_track.ok, "v2 setup failed: {:?}", added_track.error);
    for track in ["v1", "a1t", "v2"] {
        let seeded = dispatch(
            &state,
            "edit.insert",
            json!({"asset":"a1", "track":track, "at_ms":0, "src_range_ms":[0,4_000], "ripple":false}),
            test_actor(),
        )
        .await;
        assert!(seeded.ok, "seed {track} failed: {:?}", seeded.error);
    }
    let marker = dispatch(
        &state,
        "edit.add_marker",
        json!({"at_ms":2_000, "label":"after linked insert"}),
        test_actor(),
    )
    .await;
    assert!(marker.ok, "marker setup failed: {:?}", marker.error);

    let inserted = dispatch(
        &state,
        "edit.insert_linked",
        linked_args("v1", "a1t"),
        test_actor(),
    )
    .await;
    assert!(inserted.ok, "linked ripple failed: {:?}", inserted.error);
    let guard = state.project.read().await;
    let project = &guard.as_ref().unwrap().project;
    assert_eq!(project.track("v1").unwrap().duration_ms(), 5_000);
    assert_eq!(project.track("a1t").unwrap().duration_ms(), 5_000);
    assert_eq!(
        project.track("v2").unwrap().duration_ms(),
        5_000,
        "the non-linked video lane receives one 1000ms ripple gap, not one gap per A/V leg",
    );
    assert_eq!(
        project
            .markers
            .iter()
            .find(|marker| marker.label == "after linked insert")
            .unwrap()
            .at_ms,
        3_000,
        "absolute-time markers shift once with the linked insertion",
    );
}

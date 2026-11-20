//! Item 176: `edit.move` placement on the issue #2 project shape.
//!
//! A 90 s clip cut at 30 s and 60 s on v1 (c1, c3, c4) and a1t (c2, c5, c6),
//! plus an empty v2. A move without ripple overwrites its new slot (the
//! Premiere/Resolve drag): the linked audio lands at its picture's time, and
//! no later clip moves on any track. Every row also proves that one undo
//! restores the exact previous project, one redo the exact moved one, and that
//! replaying the log rebuilds the live timeline.

use super::*;

/// One line per track: `id start-end(src in-out)` in seconds, gaps as `gap`.
fn layout(project: &cut_core::Project) -> Vec<String> {
    project
        .tracks
        .iter()
        .map(|track| {
            let mut cursor = 0;
            let parts: Vec<String> = track
                .clips
                .iter()
                .map(|clip| {
                    let start = cursor;
                    cursor += clip.timeline_duration_ms();
                    let span = format!("{}-{}", start / 1000, cursor / 1000);
                    match clip {
                        cut_core::Clip::Media(m) => format!(
                            "{} {span}(src{}-{})",
                            m.id,
                            m.src_in_ms / 1000,
                            m.src_out_ms / 1000
                        ),
                        _ => format!("gap {span}"),
                    }
                })
                .collect();
            format!("{}: {}", track.id, parts.join(", "))
        })
        .collect()
}

async fn issue2_state() -> AppState {
    let dir = tempfile::tempdir().unwrap().keep();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "issue2", "dir": dir.join("issue2.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let media = dir.join("issue2.mp4");
    std::fs::write(&media, b"not-really-video").unwrap();
    let imported = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(imported.ok, "{:?}", imported.error);
    for track in ["v1", "a1t"] {
        let args =
            json!({"asset":"a1","track":track,"at_ms":0,"src_range_ms":[0,90_000],"ripple":false});
        let inserted = dispatch(&state, "edit.insert", args, test_actor()).await;
        assert!(inserted.ok, "{:?}", inserted.error);
    }
    for (track, at_ms) in [
        ("v1", 30_000),
        ("v1", 60_000),
        ("a1t", 30_000),
        ("a1t", 60_000),
    ] {
        let split = dispatch(
            &state,
            "edit.split",
            json!({"track":track,"at_ms":at_ms}),
            test_actor(),
        )
        .await;
        assert!(split.ok, "{:?}", split.error);
    }
    let added = dispatch(
        &state,
        "edit.add_track",
        json!({"kind":"video"}),
        test_actor(),
    )
    .await;
    assert!(added.ok, "{:?}", added.error);
    assert_eq!(layout(&project(&state).await), BASE);
    state
}

async fn project(state: &AppState) -> cut_core::Project {
    state.project.read().await.as_ref().unwrap().project.clone()
}

const BASE: [&str; 3] = [
    "v1: c1 0-30(src0-30), c3 30-60(src30-60), c4 60-90(src60-90)",
    "v2: ",
    "a1t: c2 0-30(src0-30), c5 30-60(src30-60), c6 60-90(src60-90)",
];

/// Move on a fresh issue #2 project and check the layout, the receipt, undo,
/// redo and replay. Returns the move's result for row-specific checks.
async fn check_move(args: Value, expected: [&str; 3]) -> Value {
    let state = issue2_state().await;
    let before = project(&state).await;
    let moved = dispatch(&state, "edit.move", args.clone(), test_actor()).await;
    assert!(moved.ok, "{args}: {:?}", moved.error);
    assert_eq!(
        moved.op_ids.as_ref().map(Vec::len),
        Some(1),
        "{args}: one op"
    );
    let after = project(&state).await;
    assert_eq!(layout(&after), expected, "{args}: timeline after the move");

    let log = state
        .project
        .read()
        .await
        .as_ref()
        .unwrap()
        .log
        .read_all()
        .unwrap();
    let rebuilt = cut_core::rebuild_from_log(&log).expect("replay the move");
    assert_eq!(rebuilt, after, "{args}: replay rebuilds the live timeline");

    let undo = dispatch(&state, "project.undo", json!({}), test_actor()).await;
    assert!(undo.ok, "{args}: {:?}", undo.error);
    assert_eq!(
        project(&state).await,
        before,
        "{args}: one undo restores everything"
    );
    let redo = dispatch(&state, "project.redo", json!({}), test_actor()).await;
    assert!(redo.ok, "{args}: {:?}", redo.error);
    assert_eq!(
        project(&state).await,
        after,
        "{args}: one redo reapplies the move"
    );
    moved.result.unwrap()
}

#[tokio::test]
async fn linked_move_to_another_video_track_at_the_same_time_moves_nothing_else() {
    // The finding: before the fix c6 ended up at 90-120 behind a 30 s gap.
    let result = check_move(
        json!({"clip":"c3","to_track":"v2","at_ms":30_000,"linked":true}),
        [
            "v1: c1 0-30(src0-30), gap 30-60, c4 60-90(src60-90)",
            "v2: gap 0-30, c3 30-60(src30-60)",
            "a1t: c2 0-30(src0-30), c5 30-60(src30-60), c6 60-90(src60-90)",
        ],
    )
    .await;
    assert_eq!(result["linked_clip"], "c5");
    assert_eq!(result["placement"], "overwrite");
    assert_eq!(result["removed_clips"], json!([]));
    let op = &result["op"];
    assert_eq!(op["args"]["placement"], "overwrite", "recorded for replay");
    let lowered = op["effects"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|effect| effect.get("lowered"))
        .expect("linked move is lowered");
    for step in lowered.as_array().unwrap() {
        assert_eq!(step["args"]["placement"], "overwrite", "{step}");
    }
}

#[tokio::test]
async fn unlinked_vertical_move_leaves_the_audio_alone() {
    let result = check_move(
        json!({"clip":"c3","to_track":"v2","at_ms":30_000,"linked":false}),
        [
            "v1: c1 0-30(src0-30), gap 30-60, c4 60-90(src60-90)",
            "v2: gap 0-30, c3 30-60(src30-60)",
            BASE[2],
        ],
    )
    .await;
    assert!(result["linked_clip"].is_null());
}

#[tokio::test]
async fn first_and_last_segment_moves_keep_every_other_clip_in_place() {
    // First segment: before the fix c5 and c6 were pushed 30 s right.
    check_move(
        json!({"clip":"c1","to_track":"v2","at_ms":0}),
        [
            "v1: gap 0-30, c3 30-60(src30-60), c4 60-90(src60-90)",
            "v2: c1 0-30(src0-30)",
            BASE[2],
        ],
    )
    .await;
    // Last segment: the vacated slot is the track tail, so v1 just ends at 60.
    check_move(
        json!({"clip":"c4","to_track":"v2","at_ms":60_000}),
        [
            "v1: c1 0-30(src0-30), c3 30-60(src30-60)",
            "v2: gap 0-60, c4 60-90(src60-90)",
            BASE[2],
        ],
    )
    .await;
}

#[tokio::test]
async fn linked_move_later_on_the_same_track_overwrites_and_keeps_sync() {
    // c4 loses its first 15 s under the moved pair; its rest stays at 75-90
    // with source 75-90 on both tracks (before the fix c4/c6 moved to 90-120).
    check_move(
        json!({"clip":"c3","to_track":"v1","at_ms":45_000}),
        [
            "v1: c1 0-30(src0-30), gap 30-45, c3 45-75(src30-60), c4 75-90(src75-90)",
            "v2: ",
            "a1t: c2 0-30(src0-30), gap 30-45, c5 45-75(src30-60), c6 75-90(src75-90)",
        ],
    )
    .await;
    // Unlinked: only the picture moves; c4's remaining picture still matches
    // the untouched c6 audio at 75 s (source 75 s on both).
    check_move(
        json!({"clip":"c3","to_track":"v1","at_ms":45_000,"linked":false}),
        [
            "v1: c1 0-30(src0-30), gap 30-45, c3 45-75(src30-60), c4 75-90(src75-90)",
            "v2: ",
            BASE[2],
        ],
    )
    .await;
}

#[tokio::test]
async fn linked_move_earlier_and_past_the_end_on_the_same_track() {
    // Earlier: overwrites the tail of c1 and the head of c3, no new clip ids.
    check_move(
        json!({"clip":"c4","to_track":"v1","at_ms":15_000}),
        [
            "v1: c1 0-15(src0-15), c4 15-45(src60-90), c3 45-60(src45-60)",
            "v2: ",
            "a1t: c2 0-15(src0-15), c6 15-45(src60-90), c5 45-60(src45-60)",
        ],
    )
    .await;
    // Past the end: a gap pads to the new position (unchanged behaviour).
    check_move(
        json!({"clip":"c3","to_track":"v1","at_ms":100_000}),
        [
            "v1: c1 0-30(src0-30), gap 30-60, c4 60-90(src60-90), gap 90-100, c3 100-130(src30-60)",
            "v2: ",
            "a1t: c2 0-30(src0-30), gap 30-60, c6 60-90(src60-90), gap 90-100, c5 100-130(src30-60)",
        ],
    )
    .await;
}

#[tokio::test]
async fn linked_move_to_another_track_and_time_overwrites_the_occupied_audio() {
    check_move(
        json!({"clip":"c3","to_track":"v2","at_ms":45_000}),
        [
            "v1: c1 0-30(src0-30), gap 30-60, c4 60-90(src60-90)",
            "v2: gap 0-45, c3 45-75(src30-60)",
            "a1t: c2 0-30(src0-30), gap 30-45, c5 45-75(src30-60), c6 75-90(src75-90)",
        ],
    )
    .await;
    // The audio slot 70-100 is partly occupied by c6 and partly past the end.
    check_move(
        json!({"clip":"c3","to_track":"v2","at_ms":70_000}),
        [
            "v1: c1 0-30(src0-30), gap 30-60, c4 60-90(src60-90)",
            "v2: gap 0-70, c3 70-100(src30-60)",
            "a1t: c2 0-30(src0-30), gap 30-60, c6 60-70(src60-70), c5 70-100(src30-60)",
        ],
    )
    .await;
    // A slot that covers a whole clip removes it, and the receipt says so.
    let result = check_move(
        json!({"clip":"c4","to_track":"v1","at_ms":30_000}),
        [
            "v1: c1 0-30(src0-30), c4 30-60(src60-90)",
            "v2: ",
            "a1t: c2 0-30(src0-30), c6 30-60(src60-90)",
        ],
    )
    .await;
    assert_eq!(result["removed_clips"], json!(["c3", "c5"]));
}

#[tokio::test]
async fn rippled_linked_move_keeps_the_insert_and_shifts_every_track_together() {
    // Alt-drag: Premiere's insert drag. Unchanged by item 176, and in sync:
    // c4 and c6 both continue at 90 s.
    let result = check_move(
        json!({"clip":"c3","to_track":"v2","at_ms":30_000,"ripple":true}),
        [
            "v1: c1 0-30(src0-30), gap 30-60, gap 60-90, c4 90-120(src60-90)",
            "v2: gap 0-30, c3 30-60(src30-60)",
            "a1t: c2 0-30(src0-30), c5 30-60(src30-60), gap 60-90, c6 90-120(src60-90)",
        ],
    )
    .await;
    assert_eq!(result["placement"], "insert");
    assert_eq!(result["rippled_tracks"], json!(["v1"]));
}

#[tokio::test]
async fn move_that_would_cut_a_frozen_clip_is_refused_and_changes_nothing() {
    let state = issue2_state().await;
    let frozen = dispatch(&state, "edit.freeze", json!({"clip":"c4"}), test_actor()).await;
    assert!(frozen.ok, "{:?}", frozen.error);
    let before = project(&state).await;
    let ops_before = state
        .project
        .read()
        .await
        .as_ref()
        .unwrap()
        .log
        .read_all()
        .unwrap()
        .len();
    let moved = dispatch(
        &state,
        "edit.move",
        json!({"clip":"c3","to_track":"v1","at_ms":45_000}),
        test_actor(),
    )
    .await;
    assert!(!moved.ok, "the overwrite would split the frozen clip");
    let error = moved.error.unwrap();
    assert_eq!(error.code, "conflict");
    assert!(
        error.message.contains("moved clip edge 75000ms"),
        "{}",
        error.message
    );
    assert_eq!(project(&state).await, before, "the refusal is atomic");
    let ops_after = state
        .project
        .read()
        .await
        .as_ref()
        .unwrap()
        .log
        .read_all()
        .unwrap()
        .len();
    assert_eq!(ops_after, ops_before, "a refused move logs nothing");
}

/// A project saved by 0.6.115 or this round's candidates holds linked moves
/// lowered WITHOUT `placement`. They must replay as the splice they were, or
/// reopening an older project would rewrite its timeline under the person.
#[tokio::test]
async fn moves_logged_without_placement_replay_with_the_historical_splice() {
    let state = issue2_state().await;
    let mut guard = state.project.write().await;
    let store = guard.as_mut().unwrap();
    let legacy_step = |clip: &str, track: &str| cut_core::InverseOp {
        verb: "edit.move".into(),
        args: json!({"clip": clip, "to_track": track, "at_ms": 30_000, "ripple": false}),
    };
    store
        .apply_lowered(
            "edit.move",
            json!({"clip":"c3","to_track":"v2","at_ms":30_000,"ripple":false,"linked":true}),
            test_actor(),
            None,
            vec![legacy_step("c3", "v2"), legacy_step("c5", "a1t")],
            Vec::new(),
        )
        .expect("legacy linked move");
    let live = store.project.clone();
    let log = store.log.read_all().unwrap();
    drop(guard);
    assert_eq!(
        layout(&live)[2],
        "a1t: c2 0-30(src0-30), c5 30-60(src30-60), gap 60-90, c6 90-120(src60-90)",
        "the historical splice result an older project was saved with"
    );
    assert_eq!(cut_core::rebuild_from_log(&log).unwrap(), live);
}

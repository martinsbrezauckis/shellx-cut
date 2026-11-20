//! Edit points less than one frame from a clip edge move to the edge (the 1 ms
//! clip after `edit.speed 2` on a 1513 ms clip and an insert 1 ms early).
//! Before the fix each of these left a 1 ms clip.

use super::*;

fn asset() -> cut_core::Asset {
    cut_core::Asset {
        path: "/fixture/snap.mp4".into(),
        hash: "sha256:snap".into(),
        probe: Some(
            json!({"duration_ms": 60_000, "kind": "video", "has_audio": true, "fps": 30.0}),
        ),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    }
}

/// A 60 fps project with v1 = c1 (src 8000..9513 at 2x: 757 ms on the timeline).
async fn sped_clip_state(name: &str) -> AppState {
    let root = tempfile::tempdir().unwrap();
    let project_dir = root.keep().join(format!("{name}.cutproj"));
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": name, "dir": project_dir, "settings": {"width": 640, "height": 360, "fps": 60}}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "project create failed: {:?}", created.error);
    {
        let mut guard = state.project.write().await;
        guard
            .as_mut()
            .unwrap()
            .record_import(Some("a1".into()), asset(), test_actor(), None)
            .unwrap();
    }
    let ins = dispatch(
        &state,
        "edit.insert",
        json!({"asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [8_000, 9_513]}),
        test_actor(),
    )
    .await;
    assert!(ins.ok, "{:?}", ins.error);
    let clip = ins.result.unwrap()["clip_id"].as_str().unwrap().to_string();
    let sp = dispatch(
        &state,
        "edit.speed",
        json!({"clip": clip, "factor": 2.0}),
        test_actor(),
    )
    .await;
    assert!(sp.ok, "{:?}", sp.error);
    state
}

/// (clip id, src_in, src_out) of every media clip on `track`, in order.
async fn media(state: &AppState, track: &str) -> Vec<(String, u64, u64)> {
    let guard = state.project.read().await;
    let p = &guard.as_ref().unwrap().project;
    p.track(track)
        .unwrap()
        .clips
        .iter()
        .filter_map(|c| match c {
            cut_core::Clip::Media(m) => Some((m.id.clone(), m.src_in_ms, m.src_out_ms)),
            _ => None,
        })
        .collect()
}

fn no_sliver(clips: &[(String, u64, u64)]) {
    assert!(
        clips.iter().all(|(_, a, b)| b - a > 16),
        "a piece shorter than a frame was left: {clips:?}"
    );
}

#[tokio::test]
async fn an_insert_one_ms_before_the_end_goes_in_at_the_end() {
    let state = sped_clip_state("snap-insert").await;
    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset": "a1", "track": "v1", "at_ms": 756, "src_range_ms": [12_000, 13_000]}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["snapped"]["from"], json!(756));
    assert_eq!(res["snapped"]["to"], json!(757));
    assert!(res["snapped"]["why"]
        .as_str()
        .unwrap()
        .contains("less than one frame"));
    let clips = media(&state, "v1").await;
    assert_eq!(clips.len(), 2, "{clips:?}");
    no_sliver(&clips);
    // The op records the moved point (replay is verbatim).
    let guard = state.project.read().await;
    let op = guard
        .as_ref()
        .unwrap()
        .log
        .read_all()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_eq!(op.args["at_ms"], json!(757));
}

#[tokio::test]
async fn a_split_one_ms_before_the_end_splits_nothing_and_says_why() {
    let state = sped_clip_state("snap-split").await;
    let r = dispatch(
        &state,
        "edit.split",
        json!({"track": "v1", "at_ms": 756}),
        test_actor(),
    )
    .await;
    assert!(!r.ok);
    let e = r.error.unwrap();
    assert!(e.message.contains("already cut at 757 ms"), "{e:?}");
    assert!(e.cause.contains("less than one frame"), "{e:?}");
    assert_eq!(media(&state, "v1").await.len(), 1);
    // A split a frame or more away still splits.
    let ok = dispatch(
        &state,
        "edit.split",
        json!({"track": "v1", "at_ms": 700}),
        test_actor(),
    )
    .await;
    assert!(ok.ok, "{:?}", ok.error);
    assert_eq!(media(&state, "v1").await.len(), 2);
}

#[tokio::test]
async fn a_paste_and_a_move_one_ms_from_an_edge_land_on_it() {
    let state = sped_clip_state("snap-paste-move").await;
    let c1 = media(&state, "v1").await[0].0.clone();
    let r = dispatch(
        &state,
        "edit.paste",
        json!({"clip": c1, "to_track": "v1", "at_ms": 756, "ripple": true}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.result.unwrap()["snapped"]["to"], json!(757));
    let clips = media(&state, "v1").await;
    no_sliver(&clips);
    // Move the second clip to 1 ms after the start of the track.
    let second = clips[1].0.clone();
    let m = dispatch(
        &state,
        "edit.move",
        json!({"clip": second, "to_track": "v1", "at_ms": 1, "ripple": true, "linked": false}),
        test_actor(),
    )
    .await;
    assert!(m.ok, "{:?}", m.error);
    assert_eq!(m.result.unwrap()["snapped"]["to"], json!(0));
    no_sliver(&media(&state, "v1").await);
}

#[tokio::test]
async fn a_linked_insert_one_ms_early_lands_on_the_edge() {
    let state = sped_clip_state("snap-linked").await;
    let r = dispatch(
        &state,
        "edit.insert_linked",
        json!({"asset": "a1", "at_ms": 756, "video_track": "v1", "audio_track": "a1t",
               "src_range_ms": [12_000, 13_000], "ripple": true}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.result.unwrap()["snapped"]["to"], json!(757));
    no_sliver(&media(&state, "v1").await);
}

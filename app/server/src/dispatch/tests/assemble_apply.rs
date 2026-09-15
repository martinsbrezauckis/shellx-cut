//! Reviewed Assemble plans become one real timeline operation only after apply.

use super::*;

async fn assembled_state(name: &str, width: u32, height: u32) -> (tempfile::TempDir, AppState) {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = root.path().join(format!("{name}.cutproj"));
    let created = dispatch(
        &state,
        "project.create",
        json!({
            "name": name,
            "dir": project_dir,
            "settings": {"width": width, "height": height},
        }),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    {
        let mut guard = state.project.write().await;
        guard
            .as_mut()
            .unwrap()
            .record_import(
                Some("a1".into()),
                cut_core::Asset {
                    path: "/fixture/assemble.mp4".into(),
                    hash: "sha256:assemble".into(),
                    probe: Some(json!({
                        "kind": "video",
                        "width": 1920,
                        "height": 1080,
                        "duration_ms": 8_000,
                        "has_audio": true,
                    })),
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                },
                test_actor(),
                None,
            )
            .unwrap();
    }
    let receipts = root.path().join(format!("{name}.cutproj/receipts"));
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(
        receipts.join("a1.words.json"),
        serde_json::to_string(&json!({
            "asset": "a1", "model": "test", "language": "en",
            "words": [
                {"idx": 0, "word": "welcome", "start_ms": 100, "end_ms": 350, "confidence": 0.9},
                {"idx": 1, "word": "to", "start_ms": 400, "end_ms": 520, "confidence": 0.9},
                {"idx": 2, "word": "the", "start_ms": 560, "end_ms": 680, "confidence": 0.9},
                {"idx": 3, "word": "product", "start_ms": 720, "end_ms": 980, "confidence": 0.9},
                {"idx": 4, "word": "demo", "start_ms": 1030, "end_ms": 1280, "confidence": 0.9},
                {"idx": 5, "word": "where", "start_ms": 1380, "end_ms": 1580, "confidence": 0.9},
                {"idx": 6, "word": "editing", "start_ms": 1620, "end_ms": 1890, "confidence": 0.9},
                {"idx": 7, "word": "stays", "start_ms": 1930, "end_ms": 2120, "confidence": 0.9},
                {"idx": 8, "word": "simple", "start_ms": 2170, "end_ms": 2450, "confidence": 0.9}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    update_asset(&state, "a1", |asset| {
        asset.transcript = Some("receipts/a1.words.json".into())
    })
    .await
    .unwrap();
    (root, state)
}

async fn plan_repurpose(state: &AppState) -> Value {
    let plan = dispatch(
        state,
        "assemble.repurpose",
        json!({"asset": "a1", "count": 1, "target_ms": 3_000}),
        test_actor(),
    )
    .await;
    assert!(plan.ok, "{:?}", plan.error);
    assert!(plan.op_ids.is_none(), "planning cannot mutate the timeline");
    plan.result.unwrap()
}

fn repurpose_apply(plan: &Value, request_id: &str) -> Value {
    json!({
        "asset": "a1",
        "count": 1,
        "target_ms": 3_000,
        "apply": plan["plan_binding"].clone(),
        "request_id": request_id,
        "expected_revision": plan["plan_binding"]["project_revision"].clone(),
    })
}

#[tokio::test]
async fn assemble_repurpose_apply_materializes_real_av_once_replays_and_undoes_once() {
    let (_root, state) = assembled_state("repurpose", 1920, 1080).await;
    let plan = plan_repurpose(&state).await;
    assert_eq!(plan["plan_binding"]["verb"], "assemble.repurpose");
    assert_eq!(
        plan["plan_binding"]["selected_ranges"],
        json!([plan["clips"].as_array().unwrap()[0]["word_range"]])
    );

    let applied = dispatch(
        &state,
        "assemble.repurpose",
        repurpose_apply(&plan, "assemble-repurpose-0001"),
        test_actor(),
    )
    .await;
    assert!(applied.ok, "{:?}", applied.error);
    assert_eq!(applied.op_ids.as_ref().map(Vec::len), Some(1));
    let receipt = applied.result.unwrap();
    assert_eq!(receipt["materialized"], true);
    assert_eq!(receipt["spans_placed"], 1);

    let (live, log) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let video_id = receipt["video_clip_ids"][0].as_str().unwrap();
        let audio_id = receipt["audio_clip_ids"][0].as_str().unwrap();
        assert!(
            store.project.find_clip(video_id).is_some(),
            "video clip is core state"
        );
        assert!(
            store.project.find_clip(audio_id).is_some(),
            "audio clip is core state"
        );
        assert_eq!(store.project.track("v1").unwrap().clips.len(), 1);
        assert_eq!(store.project.track("a1t").unwrap().clips.len(), 1);
        (store.project.clone(), store.log.read_all().unwrap())
    };
    assert_eq!(
        log.last().map(|op| op.verb.as_str()),
        Some("assemble.repurpose")
    );
    let rebuilt = cut_core::rebuild_from_log(&log).unwrap();
    assert_eq!(
        rebuilt.tracks, live.tracks,
        "the atomic lowered operation is durable and replayable"
    );

    let undone = dispatch(&state, "project.undo", json!({}), test_actor()).await;
    assert!(undone.ok, "{:?}", undone.error);
    let guard = state.project.read().await;
    let project = &guard.as_ref().unwrap().project;
    assert!(project.track("v1").unwrap().clips.is_empty());
    assert!(project.track("a1t").unwrap().clips.is_empty());
}

#[tokio::test]
async fn assemble_shorts_apply_requires_matching_project_aspect_without_mutating() {
    let (_root, state) = assembled_state("shorts-refused", 1920, 1080).await;
    let plan = dispatch(
        &state,
        "assemble.shorts",
        json!({"asset": "a1", "count": 1, "target_ms": 3_000, "aspect": "9:16"}),
        test_actor(),
    )
    .await;
    assert!(plan.ok, "{:?}", plan.error);
    let result = plan.result.unwrap();
    assert_eq!(result["materialization"]["eligible"], false);
    assert_eq!(result["materialization"]["required_aspect"], "9:16");
    assert_eq!(result["materialization"]["project_aspect"], "16:9");
    let before = {
        let guard = state.project.read().await;
        guard.as_ref().unwrap().log.read_all().unwrap().len()
    };
    let applied = dispatch(
        &state,
        "assemble.shorts",
        json!({
            "asset": "a1", "count": 1, "target_ms": 3_000, "aspect": "9:16",
            "apply": result["plan_binding"].clone(),
            "request_id": "assemble-shorts-refused-0001",
            "expected_revision": result["plan_binding"]["project_revision"].clone(),
        }),
        test_actor(),
    )
    .await;
    assert!(!applied.ok);
    assert_eq!(applied.error.unwrap().code, error_codes::CONFLICT);
    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    assert_eq!(store.log.read_all().unwrap().len(), before);
    assert!(store.project.track("v1").unwrap().clips.is_empty());
}

#[tokio::test]
async fn assemble_shorts_apply_crops_and_captions_real_timeline_then_one_undo() {
    let (_root, state) = assembled_state("shorts-applied", 1080, 1920).await;
    let plan = dispatch(
        &state,
        "assemble.shorts",
        json!({"asset": "a1", "count": 1, "target_ms": 3_000, "aspect": "9:16"}),
        test_actor(),
    )
    .await;
    assert!(plan.ok, "{:?}", plan.error);
    let result = plan.result.unwrap();
    assert_eq!(result["materialization"]["eligible"], true);
    let applied = dispatch(
        &state,
        "assemble.shorts",
        json!({
            "asset": "a1", "count": 1, "target_ms": 3_000, "aspect": "9:16",
            "apply": result["plan_binding"].clone(),
            "request_id": "assemble-shorts-applied-0001",
            "expected_revision": result["plan_binding"]["project_revision"].clone(),
        }),
        test_actor(),
    )
    .await;
    assert!(applied.ok, "{:?}", applied.error);
    assert_eq!(applied.op_ids.as_ref().map(Vec::len), Some(1));
    let receipt = applied.result.unwrap();
    let (live, log) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        (store.project.clone(), store.log.read_all().unwrap())
    };
    assert_eq!(
        log.last().map(|op| op.verb.as_str()),
        Some("assemble.shorts")
    );
    assert_eq!(
        cut_core::rebuild_from_log(&log).unwrap().tracks,
        live.tracks
    );
    let guard = state.project.read().await;
    let project = &guard.as_ref().unwrap().project;
    let (track_id, clip_index) = project
        .find_clip(receipt["video_clip_ids"][0].as_str().unwrap())
        .unwrap();
    let video = match &project.track(track_id).unwrap().clips[clip_index] {
        cut_core::Clip::Media(clip) => clip,
        _ => panic!("Assemble short must materialize a media clip"),
    };
    assert_eq!(
        video
            .crop
            .as_ref()
            .map(|crop| [crop.x, crop.y, crop.w, crop.h]),
        Some([656, 0, 607, 1080])
    );
    let captions = project.track("asmcap1").expect("Assemble caption track");
    let cue = match captions.clips.first().unwrap() {
        cut_core::Clip::Caption(cue) => cue,
        _ => panic!("caption track holds caption clips"),
    };
    assert_eq!(cue.text, "welcome to the product demo where editing");
    assert_eq!(
        cue.range_ms,
        [40, 1830],
        "first caption timing follows the inserted source span"
    );
    let cue = match captions.clips.get(1).unwrap() {
        cut_core::Clip::Caption(cue) => cue,
        _ => panic!("caption track holds caption clips"),
    };
    assert_eq!(cue.text, "stays simple");
    assert_eq!(
        cue.range_ms,
        [1870, 2390],
        "wrapped caption retains word timing"
    );
    drop(guard);

    let undone = dispatch(&state, "project.undo", json!({}), test_actor()).await;
    assert!(undone.ok, "{:?}", undone.error);
    let guard = state.project.read().await;
    let project = &guard.as_ref().unwrap().project;
    assert!(project.track("v1").unwrap().clips.is_empty());
    assert!(project.track("a1t").unwrap().clips.is_empty());
    assert!(
        project.track("asmcap1").is_none(),
        "one undo removes the added caption track too"
    );
}

#[tokio::test]
async fn assemble_apply_refuses_stale_foreign_and_locked_plans_without_partial_edits() {
    let (_root, stale_state) = assembled_state("stale", 1920, 1080).await;
    let stale_plan = plan_repurpose(&stale_state).await;
    let marker = dispatch(
        &stale_state,
        "edit.add_marker",
        json!({"at_ms": 100, "label": "intervening edit"}),
        test_actor(),
    )
    .await;
    assert!(marker.ok, "{:?}", marker.error);
    let stale = dispatch(
        &stale_state,
        "assemble.repurpose",
        repurpose_apply(&stale_plan, "assemble-stale-0001"),
        test_actor(),
    )
    .await;
    assert!(!stale.ok);
    assert_eq!(stale.error.unwrap().code, error_codes::CONFLICT);
    let guard = stale_state.project.read().await;
    assert!(guard
        .as_ref()
        .unwrap()
        .project
        .track("v1")
        .unwrap()
        .clips
        .is_empty());
    drop(guard);

    let (provenance_root, provenance_state) =
        assembled_state("transcript-provenance", 1920, 1080).await;
    let provenance_plan = plan_repurpose(&provenance_state).await;
    let provenance_receipt = provenance_root
        .path()
        .join("transcript-provenance.cutproj/receipts/a1.words.json");
    let mut provenance: Value =
        serde_json::from_slice(&std::fs::read(&provenance_receipt).unwrap()).unwrap();
    provenance["model"] = json!("corrected-receipt-provenance");
    std::fs::write(
        &provenance_receipt,
        serde_json::to_vec(&provenance).unwrap(),
    )
    .unwrap();
    let provenance_apply = dispatch(
        &provenance_state,
        "assemble.repurpose",
        repurpose_apply(&provenance_plan, "assemble-transcript-provenance-0001"),
        test_actor(),
    )
    .await;
    assert!(
        provenance_apply.ok,
        "receipt provenance alone must not invalidate reviewed word spans: {:?}",
        provenance_apply.error
    );

    let (transcript_root, transcript_state) =
        assembled_state("transcript-change", 1920, 1080).await;
    let transcript_plan = plan_repurpose(&transcript_state).await;
    let receipt = transcript_root
        .path()
        .join("transcript-change.cutproj/receipts/a1.words.json");
    let changed = std::fs::read_to_string(&receipt)
        .unwrap()
        .replace("\"welcome\"", "\"changed\"");
    std::fs::write(&receipt, changed).unwrap();
    let before = {
        let guard = transcript_state.project.read().await;
        guard.as_ref().unwrap().log.read_all().unwrap().len()
    };
    let changed_apply = dispatch(
        &transcript_state,
        "assemble.repurpose",
        repurpose_apply(&transcript_plan, "assemble-transcript-changed-0001"),
        test_actor(),
    )
    .await;
    assert!(!changed_apply.ok);
    assert_eq!(changed_apply.error.unwrap().code, error_codes::CONFLICT);
    let guard = transcript_state.project.read().await;
    let store = guard.as_ref().unwrap();
    assert_eq!(store.log.read_all().unwrap().len(), before);
    assert!(store.project.track("v1").unwrap().clips.is_empty());
    drop(guard);

    let (_foreign_root, foreign_state) = assembled_state("foreign", 1920, 1080).await;
    // Both fixtures have the same durable op number, so the optimistic guard
    // passes and the handler must prove the path-free project identity differs.
    let foreign = dispatch(
        &foreign_state,
        "assemble.repurpose",
        repurpose_apply(&stale_plan, "assemble-foreign-0001"),
        test_actor(),
    )
    .await;
    assert!(!foreign.ok);
    assert_eq!(foreign.error.unwrap().code, error_codes::CONFLICT);
    let guard = foreign_state.project.read().await;
    assert!(guard
        .as_ref()
        .unwrap()
        .project
        .track("v1")
        .unwrap()
        .clips
        .is_empty());
    drop(guard);

    let (_locked_root, locked_state) = assembled_state("locked", 1920, 1080).await;
    let locked = dispatch(
        &locked_state,
        "edit.track_lock",
        json!({"track": "v1", "on": true}),
        test_actor(),
    )
    .await;
    assert!(locked.ok, "{:?}", locked.error);
    let locked_plan = plan_repurpose(&locked_state).await;
    let locked_apply = dispatch(
        &locked_state,
        "assemble.repurpose",
        repurpose_apply(&locked_plan, "assemble-locked-0001"),
        test_actor(),
    )
    .await;
    assert!(!locked_apply.ok);
    assert_eq!(locked_apply.error.unwrap().code, error_codes::CONFLICT);
    let guard = locked_state.project.read().await;
    assert!(guard
        .as_ref()
        .unwrap()
        .project
        .track("v1")
        .unwrap()
        .clips
        .is_empty());
}

#[tokio::test]
async fn assemble_from_script_keeps_unmatched_lines_visible_and_refuses_all_unmatched_apply() {
    let (_root, state) = assembled_state("script", 1920, 1080).await;
    let plan = dispatch(
        &state,
        "assemble.from_script",
        json!({"asset": "a1", "script": "nonexistent zebras\nstill nowhere", "min_score": 0.9}),
        test_actor(),
    )
    .await;
    assert!(plan.ok, "{:?}", plan.error);
    let result = plan.result.unwrap();
    assert_eq!(result["matched"], 0);
    assert_eq!(result["plan_binding"]["selected_ranges"], json!([]));
    assert!(result["plan_binding"]["transcript_sha256"]
        .as_str()
        .is_some_and(|hash| hash.starts_with("sha256:")));
    assert_eq!(result["segments"].as_array().unwrap().len(), 2);
    assert!(result["segments"]
        .as_array()
        .unwrap()
        .iter()
        .all(|segment| segment["matched"] == false));
    let before = {
        let guard = state.project.read().await;
        guard.as_ref().unwrap().log.read_all().unwrap().len()
    };
    let applied = dispatch(
        &state,
        "assemble.from_script",
        json!({
            "asset": "a1", "script": "nonexistent zebras\nstill nowhere", "min_score": 0.9,
            "apply": result["plan_binding"].clone(),
            "request_id": "assemble-script-unmatched-0001",
            "expected_revision": result["plan_binding"]["project_revision"].clone(),
        }),
        test_actor(),
    )
    .await;
    assert!(!applied.ok);
    let error = applied.error.as_ref().unwrap();
    assert_eq!(error.code, error_codes::INVALID_ARGS);
    assert!(error.message.contains("no matched transcript ranges"));
    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    assert_eq!(store.log.read_all().unwrap().len(), before);
    assert!(store.project.track("v1").unwrap().clips.is_empty());
}

#[tokio::test]
async fn assemble_from_script_applies_only_the_reviewed_matched_ranges() {
    let (_root, state) = assembled_state("script-matched", 1920, 1080).await;
    let plan = dispatch(
        &state,
        "assemble.from_script",
        json!({
            "asset": "a1",
            "script": "welcome to the product demo\nnot present zebras\nediting stays simple",
            "min_score": 0.7,
        }),
        test_actor(),
    )
    .await;
    assert!(plan.ok, "{:?}", plan.error);
    let result = plan.result.unwrap();
    assert_eq!(result["matched"], 2);
    assert_eq!(
        result["segments"][1]["matched"], false,
        "unmatched line remains visible"
    );
    let applied = dispatch(
        &state,
        "assemble.from_script",
        json!({
            "asset": "a1",
            "script": "welcome to the product demo\nnot present zebras\nediting stays simple",
            "min_score": 0.7,
            "apply": result["plan_binding"].clone(),
            "request_id": "assemble-script-matched-0001",
            "expected_revision": result["plan_binding"]["project_revision"].clone(),
        }),
        test_actor(),
    )
    .await;
    assert!(applied.ok, "{:?}", applied.error);
    let receipt = applied.result.unwrap();
    assert_eq!(receipt["spans_placed"], 2);
    let (live, log) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        assert_eq!(store.project.track("v1").unwrap().clips.len(), 2);
        assert_eq!(store.project.track("a1t").unwrap().clips.len(), 2);
        (store.project.clone(), store.log.read_all().unwrap())
    };
    assert_eq!(
        log.last().map(|op| op.verb.as_str()),
        Some("assemble.from_script")
    );
    assert_eq!(
        cut_core::rebuild_from_log(&log).unwrap().tracks,
        live.tracks
    );

    let undone = dispatch(&state, "project.undo", json!({}), test_actor()).await;
    assert!(undone.ok, "{:?}", undone.error);
    let guard = state.project.read().await;
    let project = &guard.as_ref().unwrap().project;
    assert!(project.track("v1").unwrap().clips.is_empty());
    assert!(project.track("a1t").unwrap().clips.is_empty());
}

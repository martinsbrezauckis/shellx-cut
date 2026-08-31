use std::fs;

use record_core::{PresetRevision, SceneEventKind, SceneId, TimerConfig};
use record_recovery::CaptureRoot;

use crate::recording_scenes::{
    RecordingSceneConfig, RecordingSceneEngine, RecordingSceneTimerAction,
    RECORDING_SCENE_PROJECTION_SCHEMA,
};

fn setup(capture_id: &str) -> (tempfile::TempDir, CaptureRoot) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project.cutproj");
    fs::create_dir(&project).unwrap();
    let root = CaptureRoot::for_project(&project).unwrap();
    root.create_capture_dir(capture_id).unwrap();
    (temp, root)
}

#[test]
fn public_named_layouts_freeze_a_replayable_screen_to_presenter_timeline() {
    let config: RecordingSceneConfig = serde_json::from_value(serde_json::json!({
        "catalog_revision": 7, "initial_scene_id": "screen",
        "presets": [
            {"id":"screen","name":"Screen","preset_revision":3,"layout":{"kind":"screen"}},
            {"id":"presenter","name":"Presenter","preset_revision":4,"layout":{"kind":"presenter_pip","corner":"top_right","size_percent":25,"shape":"rounded_rect"}}
        ], "timer":{"kind":"elapsed"}
    }))
    .unwrap();
    assert!(!config.initial_requires_camera().unwrap());
    let (_temp, root) = setup("recording-scenes-public");
    let mut engine = RecordingSceneEngine::create(
        &root,
        "recording-scenes-public",
        config.accepted_snapshot().unwrap(),
    )
    .unwrap();
    assert_eq!(
        engine.start_at(0).unwrap().active_scene_id.as_str(),
        "screen"
    );
    assert!(engine
        .activation_requires_camera(
            &SceneId::parse("presenter").unwrap(),
            PresetRevision::new(4).unwrap()
        )
        .unwrap());
    assert_eq!(
        engine
            .activate_at(
                250,
                SceneId::parse("presenter").unwrap(),
                PresetRevision::new(4).unwrap(),
            )
            .unwrap()
            .active_scene_id
            .as_str(),
        "presenter"
    );
    engine.finish_at(400).unwrap();
    let projection = engine.completed_projection_at(400).unwrap();
    let encoded = serde_json::to_value(&projection).unwrap();
    assert_eq!(encoded["schema"], RECORDING_SCENE_PROJECTION_SCHEMA);
    assert_eq!(encoded["events"].as_array().map(Vec::len), Some(3));
    assert_eq!(
        encoded["events"][1]["kind"]["ActivateScene"]["scene_id"],
        "presenter"
    );
    assert_eq!(projection.journal_sha256().len(), 64);
    let decoded: crate::recording_scenes::RecordingSceneProjection =
        serde_json::from_value(encoded.clone()).unwrap();
    decoded.validate().unwrap();
    let mut forged = encoded;
    forged["scene"]["active_scene_id"] = serde_json::json!("screen");
    let forged: crate::recording_scenes::RecordingSceneProjection =
        serde_json::from_value(forged).unwrap();
    assert!(forged.validate().is_err());
    drop(engine);
    assert_eq!(
        RecordingSceneEngine::reopen(&root, "recording-scenes-public")
            .unwrap()
            .completed_projection_at(400)
            .unwrap()
            .events()
            .len(),
        3
    );
}

#[test]
fn unsupported_layout_vocabulary_is_rejected_before_a_journal_exists() {
    let error = serde_json::from_value::<RecordingSceneConfig>(serde_json::json!({
        "catalog_revision":1,"initial_scene_id":"wide",
        "presets":[{"id":"wide","name":"Side by side","preset_revision":1,"layout":{"kind":"side_by_side"}}],
        "timer":{"kind":"elapsed"}
    }))
    .unwrap_err();
    assert!(error.to_string().contains("side_by_side"));
}

#[test]
fn torn_tail_replay_then_append_preserves_header_and_prior_events() {
    let (_temp, root) = setup("recording-scenes-reopen");
    let mut engine = RecordingSceneEngine::create(
        &root,
        "recording-scenes-reopen",
        RecordingSceneConfig::default_screen()
            .accepted_snapshot()
            .unwrap(),
    )
    .unwrap();
    engine.start_at(0).unwrap();
    drop(engine);
    let journal = root
        .capture_file("recording-scenes-reopen", "recorder-scenes.journal.jsonl")
        .unwrap();
    let mut bytes = fs::read(&journal).unwrap();
    bytes.extend_from_slice(br#"{"entry":"event""#);
    fs::write(&journal, bytes).unwrap();
    let mut reopened = RecordingSceneEngine::reopen(&root, "recording-scenes-reopen").unwrap();
    reopened.finish_at(10).unwrap();
    let projection = reopened.completed_projection_at(10).unwrap();
    assert_eq!(projection.events().len(), 2);
    assert_eq!(projection.events()[0].sequence, 1);
    assert_eq!(projection.events()[1].sequence, 2);
    drop(reopened);
    let repaired = fs::read(&journal).unwrap();
    assert!(repaired.starts_with(br#"{"entry":"header"#));
    assert!(repaired.ends_with(b"\n"));
}

#[test]
fn public_wire_config_round_trips_one_capture_wide_countdown_timer() {
    let value = serde_json::json!({
        "catalog_revision":9,"initial_scene_id":"screen",
        "presets":[
            {"id":"screen","name":"Screen","preset_revision":2,"layout":{"kind":"screen"}},
            {"id":"presenter","name":"Presenter","preset_revision":3,"layout":{"kind":"presenter_pip","corner":"top_right","size_percent":25,"shape":"circle"}}
        ],"timer":{"kind":"countdown","duration_ms":90_000}
    });
    let config: RecordingSceneConfig = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(&config).unwrap(), value);
    let snapshot = config.accepted_snapshot().unwrap();
    assert_eq!(
        snapshot.timer_config().unwrap(),
        TimerConfig::countdown(90_000).unwrap()
    );
    assert!(snapshot
        .presets()
        .iter()
        .all(|preset| preset.timer() == TimerConfig::countdown(90_000).unwrap()));
}

#[test]
fn elapsed_and_countdown_timer_actions_are_durable_capture_clock_events() {
    let (_temp, root) = setup("recording-scenes-timers");
    let config: RecordingSceneConfig = serde_json::from_value(serde_json::json!({
        "catalog_revision":1,"initial_scene_id":"screen",
        "presets":[{"id":"screen","name":"Screen","preset_revision":1,"layout":{"kind":"screen"}}],
        "timer":{"kind":"countdown","duration_ms":1_000}
    }))
    .unwrap();
    let mut engine = RecordingSceneEngine::create(
        &root,
        "recording-scenes-timers",
        config.accepted_snapshot().unwrap(),
    )
    .unwrap();
    engine.start_at(0).unwrap();
    engine
        .timer_action_at(100, RecordingSceneTimerAction::Pause)
        .unwrap();
    engine
        .timer_action_at(200, RecordingSceneTimerAction::Resume)
        .unwrap();
    engine
        .timer_action_at(300, RecordingSceneTimerAction::Reset)
        .unwrap();
    engine
        .timer_action_at(400, RecordingSceneTimerAction::Restart)
        .unwrap();
    engine
        .timer_action_at(500, RecordingSceneTimerAction::End)
        .unwrap();
    let projection = engine.completed_projection_at(500).unwrap();
    assert_eq!(projection.events().len(), 8);
    assert!(matches!(
        projection.events()[5].kind,
        SceneEventKind::TimerReset
    ));
    assert!(matches!(
        projection.events()[6].kind,
        SceneEventKind::TimerStart
    ));
    assert_eq!(projection.events()[5].logical_media_time_ms, 400);
    assert_eq!(projection.events()[6].logical_media_time_ms, 400);

    root.create_capture_dir("recording-scenes-elapsed").unwrap();
    let mut elapsed = RecordingSceneEngine::create(
        &root,
        "recording-scenes-elapsed",
        RecordingSceneConfig::default_screen()
            .accepted_snapshot()
            .unwrap(),
    )
    .unwrap();
    elapsed.start_at(0).unwrap();
    elapsed
        .timer_action_at(20, RecordingSceneTimerAction::Pause)
        .unwrap();
    elapsed
        .timer_action_at(40, RecordingSceneTimerAction::Resume)
        .unwrap();
    elapsed.finish_at(60).unwrap();
    assert_eq!(
        elapsed.completed_projection_at(60).unwrap().events().len(),
        4
    );
}

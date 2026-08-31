use super::{
    scene::{
        AcceptedStartSnapshot, CatalogRevision, PipCorner, PipShape, PipSizePercent, PresenterPip,
        PresetRevision, SceneCatalog, SceneComposition, SceneEvent, SceneEventKind, SceneId,
        ScenePreset, TimerConfig,
    },
    scene_projection::{EditableSceneTimeline, SceneFixtureRenderer, SceneTimerFixture},
    scene_reducer::{reduce_scene_event, SceneState, TimerPhase},
    EditPlan,
};

fn id(value: &str) -> SceneId {
    SceneId::parse(value).unwrap()
}

fn snapshot(timer: TimerConfig) -> AcceptedStartSnapshot {
    let screen = ScenePreset::new(
        id("screen"),
        "Screen",
        PresetRevision::new(1).unwrap(),
        SceneComposition::ScreenOnly,
        timer,
    )
    .unwrap();
    let pip = ScenePreset::new(
        id("presenter"),
        "Presenter",
        PresetRevision::new(2).unwrap(),
        SceneComposition::PresenterPip(PresenterPip::new(
            PipCorner::BottomRight,
            PipSizePercent::new(25).unwrap(),
            PipShape::RoundedRect,
        )),
        TimerConfig::Off,
    )
    .unwrap();
    let catalog = SceneCatalog::new(CatalogRevision::new(1).unwrap(), vec![screen, pip]).unwrap();
    AcceptedStartSnapshot::accept(&catalog, &id("screen")).unwrap()
}

fn event(
    sequence: u64,
    logical_media_time_ms: u64,
    snapshot: &AcceptedStartSnapshot,
    kind: SceneEventKind,
) -> SceneEvent {
    SceneEvent {
        sequence,
        logical_media_time_ms,
        snapshot_revision: snapshot.revision(),
        kind,
    }
}

#[test]
fn fixture_projection_is_equal_for_the_same_logical_state() {
    let snapshot = snapshot(TimerConfig::countdown(1_000).unwrap());
    let state = SceneState::initial(&snapshot).unwrap();
    let state = reduce_scene_event(
        &snapshot,
        &state,
        &event(1, 0, &snapshot, SceneEventKind::TimerStart),
    )
    .unwrap();
    let state = reduce_scene_event(
        &snapshot,
        &state,
        &event(
            2,
            250,
            &snapshot,
            SceneEventKind::ActivateScene {
                scene_id: id("presenter"),
                preset_revision: PresetRevision::new(2).unwrap(),
            },
        ),
    )
    .unwrap();

    let first = SceneFixtureRenderer::render(&snapshot, &state, 500).unwrap();
    let second = SceneFixtureRenderer::render(&snapshot, &state, 500).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.active_scene_id, id("presenter"));
    assert!(matches!(
        first.composition,
        SceneComposition::PresenterPip(_)
    ));
    assert_eq!(
        first.timer,
        SceneTimerFixture::Countdown {
            phase: TimerPhase::Running,
            remaining_ms: 500,
        }
    );
}

#[test]
fn fixture_projection_has_no_wall_clock_drift_while_paused() {
    let snapshot = snapshot(TimerConfig::Elapsed);
    let state = SceneState::initial(&snapshot).unwrap();
    let state = reduce_scene_event(
        &snapshot,
        &state,
        &event(1, 10, &snapshot, SceneEventKind::TimerStart),
    )
    .unwrap();
    let state = reduce_scene_event(
        &snapshot,
        &state,
        &event(2, 410, &snapshot, SceneEventKind::TimerPause),
    )
    .unwrap();

    let fixture = SceneFixtureRenderer::render(&snapshot, &state, 9_999).unwrap();

    assert_eq!(
        fixture.timer,
        SceneTimerFixture::Elapsed {
            phase: TimerPhase::Paused,
            elapsed_ms: 400,
        }
    );
}

#[test]
fn fixture_projection_describes_screen_only_off_without_a_camera_fixture() {
    let snapshot = snapshot(TimerConfig::Off);
    let state = SceneState::initial(&snapshot).unwrap();

    let fixture = SceneFixtureRenderer::render(&snapshot, &state, 0).unwrap();

    assert_eq!(fixture.active_scene_id, id("screen"));
    assert_eq!(fixture.composition, SceneComposition::ScreenOnly);
    assert_eq!(fixture.timer, SceneTimerFixture::Off);
}

#[test]
fn editable_timeline_replays_only_durable_scene_boundaries() {
    let snapshot = snapshot(TimerConfig::Elapsed);
    let events = vec![
        event(1, 0, &snapshot, SceneEventKind::TimerStart),
        event(
            2,
            250,
            &snapshot,
            SceneEventKind::ActivateScene {
                scene_id: id("presenter"),
                preset_revision: PresetRevision::new(2).unwrap(),
            },
        ),
        event(
            3,
            400,
            &snapshot,
            SceneEventKind::ActivateScene {
                scene_id: id("screen"),
                preset_revision: PresetRevision::new(1).unwrap(),
            },
        ),
        event(4, 700, &snapshot, SceneEventKind::TimerEnd),
    ];

    let timeline =
        EditableSceneTimeline::from_replay(&snapshot, &events, 700, "a".repeat(64)).unwrap();

    assert_eq!(timeline.logical_duration_ms, 700);
    assert_eq!(
        timeline
            .screen
            .iter()
            .map(|segment| (segment.start_ms, segment.end_ms, segment.scene_id.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (0, 250, "screen"),
            (250, 400, "presenter"),
            (400, 700, "screen")
        ]
    );
    assert!(timeline.camera[0].presenter.is_none());
    assert!(timeline.camera[1].presenter.is_some());
    assert!(timeline.camera[2].presenter.is_none());
    assert_eq!(
        timeline.timer[1].at_start,
        SceneTimerFixture::Elapsed {
            phase: TimerPhase::Running,
            elapsed_ms: 250,
        }
    );
    assert_eq!(
        timeline.terminal.timer,
        SceneTimerFixture::Elapsed {
            phase: TimerPhase::Ended,
            elapsed_ms: 700,
        }
    );
    assert_eq!(
        serde_json::from_str::<EditableSceneTimeline>(&serde_json::to_string(&timeline).unwrap())
            .unwrap(),
        timeline
    );
}

#[test]
fn editable_timeline_applies_one_camera_keyframe_per_scene_boundary() {
    let snapshot = snapshot(TimerConfig::Elapsed);
    let events = vec![
        event(1, 0, &snapshot, SceneEventKind::TimerStart),
        event(
            2,
            200,
            &snapshot,
            SceneEventKind::ActivateScene {
                scene_id: id("presenter"),
                preset_revision: PresetRevision::new(2).unwrap(),
            },
        ),
        event(3, 500, &snapshot, SceneEventKind::TimerEnd),
    ];
    let timeline =
        EditableSceneTimeline::from_replay(&snapshot, &events, 500, "b".repeat(64)).unwrap();
    let mut plan = EditPlan::empty(1600, 900, 500, 30.0);

    timeline
        .apply_to_edit_plan(&mut plan, Some("camera.mp4".into()), None)
        .unwrap();

    let webcam = plan.webcam.as_ref().unwrap();
    assert_eq!(webcam.timeline.len(), 2);
    assert_eq!(webcam.timeline[0].t_ms, 0);
    assert_eq!(webcam.timeline[0].visible, Some(false));
    assert_eq!(webcam.timeline[1].t_ms, 200);
    assert_eq!(webcam.timeline[1].visible, Some(true));
    assert_eq!(webcam.timeline[1].size, Some(0.25));
    assert!(plan.scene_timeline.is_some());
    assert!(plan.validate().is_ok());
}

#[test]
fn editable_timeline_refuses_to_invent_a_camera_source() {
    let snapshot = snapshot(TimerConfig::Off);
    let events = vec![event(
        1,
        0,
        &snapshot,
        SceneEventKind::ActivateScene {
            scene_id: id("presenter"),
            preset_revision: PresetRevision::new(2).unwrap(),
        },
    )];
    let timeline =
        EditableSceneTimeline::from_replay(&snapshot, &events, 50, "c".repeat(64)).unwrap();
    let mut plan = EditPlan::empty(1600, 900, 50, 30.0);

    assert!(timeline.apply_to_edit_plan(&mut plan, None, None).is_err());
    assert!(plan.webcam.is_none());
}

#[test]
fn editable_timeline_applies_events_at_zero_before_its_first_span() {
    let snapshot = snapshot(TimerConfig::Elapsed);
    let events = vec![
        event(1, 0, &snapshot, SceneEventKind::TimerStart),
        event(
            2,
            0,
            &snapshot,
            SceneEventKind::ActivateScene {
                scene_id: id("presenter"),
                preset_revision: PresetRevision::new(2).unwrap(),
            },
        ),
        event(3, 61_500, &snapshot, SceneEventKind::TimerEnd),
    ];

    let timeline =
        EditableSceneTimeline::from_replay(&snapshot, &events, 61_500, "d".repeat(64)).unwrap();

    assert_eq!(timeline.screen.len(), 1);
    assert_eq!(timeline.screen[0].scene_id, id("presenter"));
    assert!(timeline.camera[0].presenter.is_some());
    assert_eq!(timeline.timer_label_at(60_500), Ok(Some("01:00".into())));
    assert_eq!(timeline.timer_label_at(61_500), Ok(Some("01:01".into())));
}

#[test]
fn editable_timeline_allows_an_empty_zero_duration_terminal_projection() {
    let snapshot = snapshot(TimerConfig::Off);
    let timeline = EditableSceneTimeline::from_replay(&snapshot, &[], 0, "e".repeat(64)).unwrap();
    let mut plan = EditPlan::empty(1600, 900, 0, 30.0);

    assert!(timeline.screen.is_empty());
    assert!(timeline.camera.is_empty());
    assert!(timeline.timer.is_empty());
    assert_eq!(timeline.timer_label_at(0), Ok(None));
    timeline.apply_to_edit_plan(&mut plan, None, None).unwrap();
    assert!(plan.validate().is_ok());
}

#[test]
fn editable_timeline_rejects_a_forged_timer_endpoint() {
    let snapshot = snapshot(TimerConfig::Elapsed);
    let events = vec![event(1, 0, &snapshot, SceneEventKind::TimerStart)];
    let mut timeline =
        EditableSceneTimeline::from_replay(&snapshot, &events, 500, "f".repeat(64)).unwrap();
    timeline.timer[0].at_end = SceneTimerFixture::Elapsed {
        phase: TimerPhase::Running,
        elapsed_ms: 499,
    };

    assert!(timeline.validate().is_err());
}

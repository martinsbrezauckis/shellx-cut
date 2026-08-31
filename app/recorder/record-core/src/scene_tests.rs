use super::{scene::*, scene_reducer::*};

fn id(value: &str) -> SceneId {
    SceneId::parse(value).unwrap()
}

fn preset(revision: u64, composition: SceneComposition, timer: TimerConfig) -> ScenePreset {
    ScenePreset::new(
        id("demo"),
        "Demo scene",
        PresetRevision::new(revision).unwrap(),
        composition,
        timer,
    )
    .unwrap()
}

fn snapshot(timer: TimerConfig) -> AcceptedStartSnapshot {
    let catalog = SceneCatalog::new(
        CatalogRevision::new(7).unwrap(),
        vec![preset(3, SceneComposition::ScreenOnly, timer)],
    )
    .unwrap();
    AcceptedStartSnapshot::accept(&catalog, &id("demo")).unwrap()
}

fn event(
    sequence: u64,
    logical_media_time_ms: u64,
    revision: SceneSnapshotRevision,
    kind: SceneEventKind,
) -> SceneEvent {
    SceneEvent {
        sequence,
        logical_media_time_ms,
        snapshot_revision: revision,
        kind,
    }
}

#[test]
fn ids_names_revisions_and_catalog_bounds_fail_closed() {
    assert_eq!(id("screen-and-presenter").as_str(), "screen-and-presenter");
    for invalid in [
        "",
        "Screen",
        "-screen",
        "screen-",
        "screen--pip",
        "screen_pip",
    ] {
        assert_eq!(SceneId::parse(invalid), Err(SceneError::InvalidSceneId));
    }
    assert_eq!(CatalogRevision::new(0), Err(SceneError::ZeroRevision));
    assert_eq!(PresetRevision::new(0), Err(SceneError::ZeroRevision));
    let valid = preset(1, SceneComposition::ScreenOnly, TimerConfig::Off);
    assert_eq!(
        ScenePreset::new(
            id("other"),
            " spaced",
            PresetRevision::new(1).unwrap(),
            SceneComposition::ScreenOnly,
            TimerConfig::Off,
        ),
        Err(SceneError::InvalidSceneName)
    );
    assert_eq!(
        ScenePreset::new(
            id("control"),
            "line\nbreak",
            PresetRevision::new(1).unwrap(),
            SceneComposition::ScreenOnly,
            TimerConfig::Off,
        ),
        Err(SceneError::InvalidSceneName)
    );
    assert_eq!(
        SceneCatalog::new(CatalogRevision::new(1).unwrap(), vec![valid.clone(), valid]),
        Err(SceneError::InvalidCatalog)
    );
    let too_many = (0..=MAX_SCENES)
        .map(|number| {
            ScenePreset::new(
                id(&format!("scene-{number}")),
                format!("Scene {number}"),
                PresetRevision::new(1).unwrap(),
                SceneComposition::ScreenOnly,
                TimerConfig::Off,
            )
            .unwrap()
        })
        .collect();
    assert_eq!(
        SceneCatalog::new(CatalogRevision::new(1).unwrap(), too_many),
        Err(SceneError::InvalidCatalog)
    );
}

#[test]
fn constrained_composition_and_accepted_snapshot_are_immutable() {
    assert_eq!(PipSizePercent::new(14), Err(SceneError::InvalidPipSize));
    assert_eq!(PipSizePercent::new(36), Err(SceneError::InvalidPipSize));
    let pip = PresenterPip::new(
        PipCorner::TopRight,
        PipSizePercent::new(24).unwrap(),
        PipShape::RoundedRect,
    );
    let original = preset(
        1,
        SceneComposition::PresenterPip(pip),
        TimerConfig::countdown(1_000).unwrap(),
    );
    let catalog = SceneCatalog::new(CatalogRevision::new(1).unwrap(), vec![original]).unwrap();
    let accepted = AcceptedStartSnapshot::accept(&catalog, &id("demo")).unwrap();
    let edited = SceneCatalog::new(
        CatalogRevision::new(2).unwrap(),
        vec![preset(
            2,
            SceneComposition::ScreenOnly,
            TimerConfig::Elapsed,
        )],
    )
    .unwrap();

    assert_eq!(accepted.revision().catalog.get(), 1);
    assert_eq!(accepted.revision().initial_preset.get(), 1);
    assert_eq!(accepted.initial_scene().unwrap().name(), "Demo scene");
    assert_eq!(
        accepted.initial_scene().unwrap().composition(),
        SceneComposition::PresenterPip(pip)
    );
    assert_eq!(
        accepted.initial_scene().unwrap().timer(),
        TimerConfig::countdown(1_000).unwrap()
    );
    assert_eq!(edited.preset(&id("demo")).unwrap().revision().get(), 2);
}

#[test]
fn countdown_reaches_time_up_at_the_exact_logical_media_time() {
    let snapshot = snapshot(TimerConfig::countdown(1_000).unwrap());
    let active = SceneState::initial(&snapshot).unwrap();
    let running = reduce_scene_event(
        &snapshot,
        &active,
        &event(1, 0, snapshot.revision(), SceneEventKind::TimerStart),
    )
    .unwrap();

    assert_eq!(
        running.timer_at(999).unwrap(),
        TimerReading {
            phase: TimerPhase::Running,
            display_ms: Some(1)
        }
    );
    assert_eq!(
        running.timer_at(1_000).unwrap(),
        TimerReading {
            phase: TimerPhase::TimeUp,
            display_ms: Some(0)
        }
    );
    let ended = reduce_scene_event(
        &snapshot,
        &running,
        &event(2, 1_000, snapshot.revision(), SceneEventKind::TimerEnd),
    )
    .unwrap();
    assert_eq!(
        ended.timer_at(1_000).unwrap(),
        TimerReading {
            phase: TimerPhase::Ended,
            display_ms: Some(0)
        }
    );
    assert_eq!(
        reduce_scene_event(
            &snapshot,
            &ended,
            &event(3, 1_000, snapshot.revision(), SceneEventKind::TimerEnd),
        ),
        Err(SceneError::InvalidTimerTransition)
    );
}

#[test]
fn pause_resume_and_reset_only_advance_on_logical_running_time() {
    let snapshot = snapshot(TimerConfig::countdown(1_000).unwrap());
    let active = SceneState::initial(&snapshot).unwrap();
    let running = reduce_scene_event(
        &snapshot,
        &active,
        &event(1, 0, snapshot.revision(), SceneEventKind::TimerStart),
    )
    .unwrap();
    let paused = reduce_scene_event(
        &snapshot,
        &running,
        &event(2, 400, snapshot.revision(), SceneEventKind::TimerPause),
    )
    .unwrap();
    assert_eq!(paused.timer_at(900).unwrap().display_ms, Some(600));
    let resumed = reduce_scene_event(
        &snapshot,
        &paused,
        &event(3, 900, snapshot.revision(), SceneEventKind::TimerResume),
    )
    .unwrap();
    assert_eq!(resumed.timer_at(1_499).unwrap().display_ms, Some(1));
    let reset = reduce_scene_event(
        &snapshot,
        &resumed,
        &event(4, 1_300, snapshot.revision(), SceneEventKind::TimerReset),
    )
    .unwrap();
    assert_eq!(
        reset.timer_at(1_300).unwrap(),
        TimerReading {
            phase: TimerPhase::Ready,
            display_ms: Some(1_000)
        }
    );
}

#[test]
fn stale_duplicate_out_of_order_and_invalid_events_do_not_change_state() {
    let snapshot = snapshot(TimerConfig::Elapsed);
    let initial = SceneState::initial(&snapshot).unwrap();
    let stale = SceneSnapshotRevision {
        catalog: CatalogRevision::new(8).unwrap(),
        initial_preset: PresetRevision::new(3).unwrap(),
    };
    assert_eq!(
        reduce_scene_event(
            &snapshot,
            &initial,
            &event(
                1,
                1,
                stale,
                SceneEventKind::ActivateScene {
                    scene_id: id("demo"),
                    preset_revision: PresetRevision::new(3).unwrap(),
                },
            ),
        ),
        Err(SceneError::StaleSnapshot)
    );
    assert_eq!(
        reduce_scene_event(
            &snapshot,
            &initial,
            &event(0, 1, snapshot.revision(), SceneEventKind::TimerStart),
        ),
        Err(SceneError::InvalidSequence)
    );
    assert_eq!(
        reduce_scene_event(
            &snapshot,
            &initial,
            &event(2, 1, snapshot.revision(), SceneEventKind::TimerStart),
        ),
        Err(SceneError::InvalidSequence)
    );
    let started = reduce_scene_event(
        &snapshot,
        &initial,
        &event(1, 10, snapshot.revision(), SceneEventKind::TimerStart),
    )
    .unwrap();
    assert_eq!(
        reduce_scene_event(
            &snapshot,
            &started,
            &event(2, 9, snapshot.revision(), SceneEventKind::TimerPause),
        ),
        Err(SceneError::InvalidLogicalTime)
    );
    assert_eq!(
        reduce_scene_event(
            &snapshot,
            &initial,
            &event(
                1,
                0,
                snapshot.revision(),
                SceneEventKind::ActivateScene {
                    scene_id: id("other"),
                    preset_revision: PresetRevision::new(1).unwrap(),
                },
            ),
        ),
        Err(SceneError::SceneMissing)
    );
    assert_eq!(initial.timer_at(0).unwrap().phase, TimerPhase::Ready);
}

#[test]
fn off_timer_rejects_transitions_after_scene_activation() {
    let snapshot = snapshot(TimerConfig::Off);
    let active = SceneState::initial(&snapshot).unwrap();
    assert_eq!(
        reduce_scene_event(
            &snapshot,
            &active,
            &event(1, 0, snapshot.revision(), SceneEventKind::TimerStart),
        ),
        Err(SceneError::TimerDisabled)
    );
}

#[test]
fn replay_is_deterministic_and_elapsed_timer_can_end_without_recording_intent() {
    let snapshot = snapshot(TimerConfig::Elapsed);
    let events = vec![
        event(1, 10, snapshot.revision(), SceneEventKind::TimerStart),
        event(2, 40, snapshot.revision(), SceneEventKind::TimerPause),
        event(3, 100, snapshot.revision(), SceneEventKind::TimerResume),
        event(4, 175, snapshot.revision(), SceneEventKind::TimerEnd),
    ];
    let first = replay_scene_events(&snapshot, &events).unwrap();
    let second = replay_scene_events(&snapshot, &events).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first.timer_at(500).unwrap(),
        TimerReading {
            phase: TimerPhase::Ended,
            display_ms: Some(105)
        }
    );
}

use super::{scene::*, scene_reducer::*};

fn id(value: &str) -> SceneId {
    SceneId::parse(value).unwrap()
}

fn preset(id_value: &str, revision: u64, timer: TimerConfig) -> ScenePreset {
    ScenePreset::new(
        id(id_value),
        format!("{id_value} scene"),
        PresetRevision::new(revision).unwrap(),
        SceneComposition::ScreenOnly,
        timer,
    )
    .unwrap()
}

fn accepted() -> AcceptedStartSnapshot {
    let catalog = SceneCatalog::new(
        CatalogRevision::new(10).unwrap(),
        vec![
            preset("scene-a", 1, TimerConfig::countdown(1_000).unwrap()),
            preset("scene-b", 2, TimerConfig::Off),
        ],
    )
    .unwrap();
    AcceptedStartSnapshot::accept(&catalog, &id("scene-a")).unwrap()
}

fn event(
    sequence: u64,
    time_ms: u64,
    snapshot: &AcceptedStartSnapshot,
    kind: SceneEventKind,
) -> SceneEvent {
    SceneEvent {
        sequence,
        logical_media_time_ms: time_ms,
        snapshot_revision: snapshot.revision(),
        kind,
    }
}

fn activate(
    sequence: u64,
    time_ms: u64,
    snapshot: &AcceptedStartSnapshot,
    id_value: &str,
    revision: u64,
) -> SceneEvent {
    event(
        sequence,
        time_ms,
        snapshot,
        SceneEventKind::ActivateScene {
            scene_id: id(id_value),
            preset_revision: PresetRevision::new(revision).unwrap(),
        },
    )
}

#[test]
fn frozen_catalog_switches_a_to_b_to_a_without_resetting_the_global_timer() {
    let snapshot = accepted();
    let later_catalog = SceneCatalog::new(
        CatalogRevision::new(11).unwrap(),
        vec![
            preset("scene-a", 3, TimerConfig::Elapsed),
            preset("scene-b", 4, TimerConfig::Elapsed),
        ],
    )
    .unwrap();
    assert_eq!(snapshot.presets().len(), 2);
    assert!(snapshot
        .preset(&id("scene-b"), PresetRevision::new(2).unwrap())
        .is_some());
    assert!(snapshot
        .preset(&id("scene-b"), PresetRevision::new(4).unwrap())
        .is_none());
    assert_eq!(
        later_catalog
            .preset(&id("scene-b"))
            .unwrap()
            .revision()
            .get(),
        4
    );

    let initial = SceneState::initial(&snapshot).unwrap();
    assert_eq!(initial.active_scene_id(), &id("scene-a"));
    let started = reduce_scene_event(
        &snapshot,
        &initial,
        &event(1, 0, &snapshot, SceneEventKind::TimerStart),
    )
    .unwrap();
    let on_b = reduce_scene_event(
        &snapshot,
        &started,
        &activate(2, 300, &snapshot, "scene-b", 2),
    )
    .unwrap();
    assert_eq!(on_b.active_scene_id(), &id("scene-b"));
    assert_eq!(on_b.timer_at(500).unwrap().display_ms, Some(500));
    let back_on_a =
        reduce_scene_event(&snapshot, &on_b, &activate(3, 650, &snapshot, "scene-a", 1)).unwrap();
    assert_eq!(back_on_a.active_scene_id(), &id("scene-a"));
    assert_eq!(back_on_a.timer_at(650).unwrap().display_ms, Some(350));
}

#[test]
fn scene_switches_reject_same_unknown_and_wrong_frozen_revisions() {
    let snapshot = accepted();
    let state = SceneState::initial(&snapshot).unwrap();
    assert_eq!(
        reduce_scene_event(&snapshot, &state, &activate(1, 0, &snapshot, "scene-a", 1)),
        Err(SceneError::SceneAlreadyActive)
    );
    assert_eq!(
        reduce_scene_event(&snapshot, &state, &activate(1, 0, &snapshot, "scene-b", 3)),
        Err(SceneError::SceneRevisionMismatch)
    );
    assert_eq!(
        reduce_scene_event(&snapshot, &state, &activate(1, 0, &snapshot, "unknown", 1)),
        Err(SceneError::SceneMissing)
    );
}

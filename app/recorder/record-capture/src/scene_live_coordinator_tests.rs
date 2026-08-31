use std::fs;
use std::time::{Duration, Instant};

use record_core::scene::ScenePreset;
use record_core::scene_projection::SceneFixtureDescriptor;
use record_core::{
    AcceptedStartSnapshot, CatalogRevision, PipCorner, PipShape, PipSizePercent, PresenterPip,
    PresetRevision, SceneCatalog, SceneComposition, SceneEventKind, SceneId, TimerConfig,
};
use record_recovery::CaptureRoot;

use crate::{LogicalSessionClock, PrivateSceneCoordinator, SessionTransition};

fn id(value: &str) -> SceneId {
    SceneId::parse(value).unwrap()
}

fn accepted_screen_pair() -> AcceptedStartSnapshot {
    let preset = |id_value: &str, revision: u64| {
        ScenePreset::new(
            id(id_value),
            format!("{id_value} scene"),
            PresetRevision::new(revision).unwrap(),
            SceneComposition::ScreenOnly,
            TimerConfig::Elapsed,
        )
        .unwrap()
    };
    let catalog = SceneCatalog::new(
        CatalogRevision::new(4).unwrap(),
        vec![preset("screen-a", 1), preset("screen-b", 2)],
    )
    .unwrap();
    AcceptedStartSnapshot::accept(&catalog, &id("screen-a")).unwrap()
}

fn accepted_with_presenter() -> AcceptedStartSnapshot {
    let preset = ScenePreset::new(
        id("presenter"),
        "Presenter",
        PresetRevision::new(1).unwrap(),
        SceneComposition::PresenterPip(PresenterPip::new(
            PipCorner::TopRight,
            PipSizePercent::new(25).unwrap(),
            PipShape::RoundedRect,
        )),
        TimerConfig::Off,
    )
    .unwrap();
    let catalog = SceneCatalog::new(CatalogRevision::new(5).unwrap(), vec![preset]).unwrap();
    AcceptedStartSnapshot::accept(&catalog, &id("presenter")).unwrap()
}

fn setup(capture_id: &str) -> (tempfile::TempDir, CaptureRoot) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project.cutproj");
    fs::create_dir(&project).unwrap();
    let root = CaptureRoot::for_project(&project).unwrap();
    root.create_capture_dir(capture_id).unwrap();
    (temp, root)
}

fn logical_ms(clock: &LogicalSessionClock, at: Instant) -> u64 {
    u64::try_from(clock.timestamp_at(at).unwrap().as_millis()).unwrap()
}

fn activate(scene: &str, revision: u64) -> SceneEventKind {
    SceneEventKind::ActivateScene {
        scene_id: id(scene),
        preset_revision: PresetRevision::new(revision).unwrap(),
    }
}

#[test]
fn coordinator_replays_a_to_b_to_a_on_pause_safe_logical_time() {
    let (_temp, root) = setup("scene-live-reopen");
    let mut scenes =
        PrivateSceneCoordinator::create(&root, "scene-live-reopen", accepted_screen_pair())
            .unwrap();
    let origin = Instant::now();
    let mut clock = LogicalSessionClock::new(None);
    clock.apply_at(SessionTransition::Start, origin);

    assert_eq!(
        scenes.append_at(logical_ms(&clock, origin), SceneEventKind::TimerStart),
        Ok(SceneFixtureDescriptor {
            active_scene_id: id("screen-a"),
            composition: SceneComposition::ScreenOnly,
            timer: record_core::scene_projection::SceneTimerFixture::Elapsed {
                phase: record_core::TimerPhase::Running,
                elapsed_ms: 0,
            },
        })
    );
    let on_b = origin + Duration::from_millis(300);
    let fixture = scenes
        .append_at(logical_ms(&clock, on_b), activate("screen-b", 2))
        .unwrap();
    assert_eq!(fixture.active_scene_id, id("screen-b"));
    assert_eq!(
        fixture.timer,
        record_core::scene_projection::SceneTimerFixture::Elapsed {
            phase: record_core::TimerPhase::Running,
            elapsed_ms: 300,
        }
    );

    clock.apply_at(SessionTransition::PauseRequested, on_b);
    clock.apply_at(
        SessionTransition::PauseCompleted,
        on_b + Duration::from_millis(1),
    );
    clock.apply_at(
        SessionTransition::ResumeRequested,
        origin + Duration::from_millis(1_000),
    );
    clock.apply_at(
        SessionTransition::ResumeCompleted,
        origin + Duration::from_millis(1_100),
    );
    let on_a = origin + Duration::from_millis(1_450);
    assert_eq!(logical_ms(&clock, on_a), 650);
    let fixture = scenes
        .append_at(logical_ms(&clock, on_a), activate("screen-a", 1))
        .unwrap();
    assert_eq!(fixture.active_scene_id, id("screen-a"));
    assert_eq!(
        fixture.timer,
        record_core::scene_projection::SceneTimerFixture::Elapsed {
            phase: record_core::TimerPhase::Running,
            elapsed_ms: 650,
        }
    );
    drop(scenes);

    let mut reopened = PrivateSceneCoordinator::reopen(&root, "scene-live-reopen").unwrap();
    assert_eq!(
        reopened.project_at(650).unwrap(),
        SceneFixtureDescriptor {
            active_scene_id: id("screen-a"),
            composition: SceneComposition::ScreenOnly,
            timer: record_core::scene_projection::SceneTimerFixture::Elapsed {
                phase: record_core::TimerPhase::Running,
                elapsed_ms: 650,
            },
        }
    );
    let next = reopened.append_at(800, activate("screen-b", 2)).unwrap();
    assert_eq!(next.active_scene_id, id("screen-b"));
    assert_eq!(
        next.timer,
        record_core::scene_projection::SceneTimerFixture::Elapsed {
            phase: record_core::TimerPhase::Running,
            elapsed_ms: 800,
        }
    );
}

#[test]
fn presenter_layouts_cannot_create_a_live_screen_owner() {
    let (_temp, root) = setup("scene-live-camera-refused");
    let error = PrivateSceneCoordinator::create(
        &root,
        "scene-live-camera-refused",
        accepted_with_presenter(),
    )
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("accepts only Screen-only layouts"));
}

#[test]
fn terminal_projection_freezes_the_elapsed_timer_and_reopens_from_its_journal() {
    let (_temp, root) = setup("scene-live-terminal-projection");
    let mut scenes = PrivateSceneCoordinator::create_default_screen_only(
        &root,
        "scene-live-terminal-projection",
    )
    .unwrap();

    scenes.append_at(0, SceneEventKind::TimerStart).unwrap();
    assert_eq!(
        scenes.finish_at(275).unwrap().timer,
        record_core::scene_projection::SceneTimerFixture::Elapsed {
            phase: record_core::TimerPhase::Ended,
            elapsed_ms: 275,
        }
    );
    let projection = scenes.completed_projection_at(275).unwrap();
    let encoded = serde_json::to_value(&projection).unwrap();
    assert_eq!(
        encoded["schema"],
        "shellx-record/private-screen-scene-projection@1"
    );
    assert_eq!(encoded["logical_media_time_ms"], 275);
    assert_eq!(encoded["scene"]["composition"], "ScreenOnly");
    assert_eq!(encoded["scene"]["timer"]["Elapsed"]["phase"], "Ended");
    assert_eq!(encoded["scene"]["timer"]["Elapsed"]["elapsed_ms"], 275);
    assert_eq!(encoded["journal_sha256"].as_str().map(str::len), Some(64));

    // A second terminal request cannot add a duplicate TimerEnd record.
    assert_eq!(
        scenes.finish_at(275).unwrap().timer,
        record_core::scene_projection::SceneTimerFixture::Elapsed {
            phase: record_core::TimerPhase::Ended,
            elapsed_ms: 275,
        }
    );
    drop(scenes);

    let reopened =
        PrivateSceneCoordinator::reopen(&root, "scene-live-terminal-projection").unwrap();
    assert_eq!(
        reopened.project_at(275).unwrap().timer,
        record_core::scene_projection::SceneTimerFixture::Elapsed {
            phase: record_core::TimerPhase::Ended,
            elapsed_ms: 275,
        }
    );
}

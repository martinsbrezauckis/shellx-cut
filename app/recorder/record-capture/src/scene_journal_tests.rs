use std::fs::{self, OpenOptions};
use std::io::Write;

use record_core::scene::ScenePreset;
use record_core::{
    AcceptedStartSnapshot, CatalogRevision, PipCorner, PipShape, PipSizePercent, PresenterPip,
    PresetRevision, SceneCatalog, SceneComposition, SceneEvent, SceneEventKind, SceneId,
    TimerConfig, TimerPhase,
};
use record_recovery::CaptureRoot;
use serde_json::{json, Value};

use crate::scene_journal::{SceneJournalOwner, MAX_SCENE_JOURNAL_BYTES, MAX_SCENE_JOURNAL_EVENTS};

fn id(value: &str) -> SceneId {
    SceneId::parse(value).unwrap()
}

fn snapshot() -> AcceptedStartSnapshot {
    let a = ScenePreset::new(
        id("scene-a"),
        "Scene A",
        PresetRevision::new(1).unwrap(),
        SceneComposition::ScreenOnly,
        TimerConfig::countdown(1_000).unwrap(),
    )
    .unwrap();
    let b = ScenePreset::new(
        id("scene-b"),
        "Scene B",
        PresetRevision::new(2).unwrap(),
        SceneComposition::PresenterPip(PresenterPip::new(
            PipCorner::BottomRight,
            PipSizePercent::new(25).unwrap(),
            PipShape::Circle,
        )),
        TimerConfig::Off,
    )
    .unwrap();
    let catalog = SceneCatalog::new(CatalogRevision::new(1).unwrap(), vec![a, b]).unwrap();
    AcceptedStartSnapshot::accept(&catalog, &id("scene-a")).unwrap()
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

fn activate(
    sequence: u64,
    at_ms: u64,
    snapshot: &AcceptedStartSnapshot,
    scene: &str,
) -> SceneEvent {
    event(
        sequence,
        at_ms,
        snapshot,
        SceneEventKind::ActivateScene {
            scene_id: id(scene),
            preset_revision: PresetRevision::new(if scene == "scene-a" { 1 } else { 2 }).unwrap(),
        },
    )
}

fn setup(capture_id: &str) -> (tempfile::TempDir, CaptureRoot) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project.cutproj");
    fs::create_dir(&project).unwrap();
    let root = CaptureRoot::for_project(&project).unwrap();
    root.create_capture_dir(capture_id).unwrap();
    (temp, root)
}

fn lines(path: &std::path::Path) -> Vec<Vec<u8>> {
    let bytes = fs::read(path).unwrap();
    assert_eq!(bytes.last(), Some(&b'\n'));
    bytes[..bytes.len() - 1]
        .split(|byte| *byte == b'\n')
        .map(Vec::from)
        .collect()
}

fn write_lines(path: &std::path::Path, lines: &[Vec<u8>]) {
    let mut bytes = lines.join(&b'\n');
    bytes.push(b'\n');
    fs::write(path, bytes).unwrap();
}

fn mutate_event(path: &std::path::Path, line: usize, mutate: impl FnOnce(&mut Value)) {
    let mut lines = lines(path);
    let mut value: Value = serde_json::from_slice(&lines[line]).unwrap();
    mutate(&mut value);
    lines[line] = serde_json::to_vec(&value).unwrap();
    write_lines(path, &lines);
}

fn seeded_two_events(root: &CaptureRoot, capture_id: &str) -> std::path::PathBuf {
    let snapshot = snapshot();
    let mut owner = SceneJournalOwner::create_new(root, capture_id, snapshot.clone()).unwrap();
    owner
        .append_event(event(1, 0, &snapshot, SceneEventKind::TimerStart))
        .unwrap();
    owner
        .append_event(event(2, 100, &snapshot, SceneEventKind::TimerPause))
        .unwrap();
    let path = owner.path().to_path_buf();
    drop(owner);
    path
}

#[test]
fn journal_headers_the_accepted_snapshot_then_replays_a_to_b_to_a() {
    let (_temp, root) = setup("scene-switch");
    let snapshot = snapshot();
    let owner = SceneJournalOwner::create_new(&root, "scene-switch", snapshot.clone()).unwrap();
    let journal_path = owner.path().to_path_buf();
    drop(owner);
    let header_only = fs::read(&journal_path).unwrap();
    assert_eq!(header_only.last(), Some(&b'\n'));
    let mut owner = SceneJournalOwner::open(&root, "scene-switch").unwrap();
    owner
        .append_event(activate(1, 100, &snapshot, "scene-b"))
        .unwrap();
    owner
        .append_event(activate(2, 200, &snapshot, "scene-a"))
        .unwrap();
    drop(owner);

    let replay = SceneJournalOwner::replay(&root, "scene-switch").unwrap();
    assert_eq!(replay.snapshot(), &snapshot);
    assert_eq!(replay.events().len(), 2);
    assert_eq!(replay.state().active_scene_id(), &id("scene-a"));
}

#[test]
fn replayed_timer_is_pause_safe_and_timeup_allows_exactly_one_end() {
    let (_temp, root) = setup("scene-timer");
    let snapshot = snapshot();
    let mut owner = SceneJournalOwner::create_new(&root, "scene-timer", snapshot.clone()).unwrap();
    owner
        .append_event(event(1, 0, &snapshot, SceneEventKind::TimerStart))
        .unwrap();
    owner
        .append_event(event(2, 400, &snapshot, SceneEventKind::TimerPause))
        .unwrap();
    assert_eq!(
        owner.replayed().state().timer_at(900).unwrap().display_ms,
        // Countdown display is the remaining duration, not elapsed capture
        // time: the 400 ms active prefix is frozen while paused.
        Some(600)
    );
    owner
        .append_event(event(3, 900, &snapshot, SceneEventKind::TimerResume))
        .unwrap();
    assert_eq!(
        owner.replayed().state().timer_at(1_500).unwrap().phase,
        TimerPhase::TimeUp
    );
    owner
        .append_event(event(4, 1_500, &snapshot, SceneEventKind::TimerEnd))
        .unwrap();
    assert_eq!(
        owner.replayed().state().timer_at(1_500).unwrap().phase,
        TimerPhase::Ended
    );
    assert!(owner
        .append_event(event(5, 1_500, &snapshot, SceneEventKind::TimerEnd))
        .is_err());
    assert_eq!(owner.replayed().events().len(), 4);
}

#[test]
fn only_an_unterminated_final_record_is_recovered() {
    let (_temp, root) = setup("scene-torn");
    let snapshot = snapshot();
    let mut owner = SceneJournalOwner::create_new(&root, "scene-torn", snapshot.clone()).unwrap();
    owner
        .append_event(event(1, 0, &snapshot, SceneEventKind::TimerStart))
        .unwrap();
    let journal_path = owner.path().to_path_buf();
    drop(owner);
    let mut tail = OpenOptions::new().append(true).open(&journal_path).unwrap();
    tail.write_all(b"{\"entry\":\"event\"").unwrap();
    drop(tail);

    assert!(SceneJournalOwner::replay(&root, "scene-torn").is_err());
    let reopened = SceneJournalOwner::open(&root, "scene-torn").unwrap();
    assert_eq!(reopened.replayed().events().len(), 1);
    drop(reopened);
    assert_eq!(fs::read(journal_path).unwrap().last(), Some(&b'\n'));
}

#[test]
fn malformed_or_changed_middle_lines_fail_closed() {
    for (capture_id, changed) in [("scene-malformed", false), ("scene-changed", true)] {
        let (_temp, root) = setup(capture_id);
        let journal_path = seeded_two_events(&root, capture_id);
        if changed {
            let mut content = lines(&journal_path);
            content[1].push(b' ');
            write_lines(&journal_path, &content);
        } else {
            let mut content = lines(&journal_path);
            content[1] = b"{malformed".to_vec();
            write_lines(&journal_path, &content);
        }
        let before = fs::read(&journal_path).unwrap();

        assert!(SceneJournalOwner::open(&root, capture_id).is_err());
        assert_eq!(fs::read(journal_path).unwrap(), before);
    }
}

#[test]
fn duplicate_stale_and_future_sequences_fail_closed() {
    for (capture_id, sequence) in [
        ("scene-duplicate", 1),
        ("scene-stale", 0),
        ("scene-future", 3),
    ] {
        let (_temp, root) = setup(capture_id);
        let journal_path = seeded_two_events(&root, capture_id);
        mutate_event(&journal_path, 2, |value| {
            value["event"]["sequence"] = json!(sequence)
        });
        let before = fs::read(&journal_path).unwrap();

        assert!(SceneJournalOwner::open(&root, capture_id).is_err());
        assert_eq!(fs::read(journal_path).unwrap(), before);
    }
}

#[test]
fn snapshot_revision_mismatch_fails_closed() {
    let (_temp, root) = setup("scene-snapshot-mismatch");
    let journal_path = seeded_two_events(&root, "scene-snapshot-mismatch");
    mutate_event(&journal_path, 1, |value| {
        value["event"]["snapshot_revision"]["catalog"] = json!(2)
    });
    let before = fs::read(&journal_path).unwrap();

    assert!(SceneJournalOwner::open(&root, "scene-snapshot-mismatch").is_err());
    assert_eq!(fs::read(journal_path).unwrap(), before);
}

#[test]
fn one_owner_retry_and_reopen_do_not_duplicate_durable_events() {
    let (_temp, root) = setup("scene-retry");
    let snapshot = snapshot();
    let event = event(1, 0, &snapshot, SceneEventKind::TimerStart);
    let mut owner = SceneJournalOwner::create_new(&root, "scene-retry", snapshot).unwrap();
    owner.append_event(event.clone()).unwrap();
    let journal_path = owner.path().to_path_buf();
    assert!(SceneJournalOwner::open(&root, "scene-retry").is_err());
    drop(owner);
    let before = fs::read(&journal_path).unwrap();
    let mut reopened = SceneJournalOwner::open(&root, "scene-retry").unwrap();

    reopened.append_event(event).unwrap();
    assert_eq!(reopened.replayed().events().len(), 1);
    drop(reopened);
    assert_eq!(fs::read(journal_path).unwrap(), before);
}

#[test]
fn journal_enforces_event_and_size_caps_without_mutating_the_leaf() {
    let (_temp, root) = setup("scene-event-cap");
    let snapshot = snapshot();
    let mut owner =
        SceneJournalOwner::create_new(&root, "scene-event-cap", snapshot.clone()).unwrap();
    for sequence in 1..=MAX_SCENE_JOURNAL_EVENTS as u64 {
        owner
            .append_event(activate(
                sequence,
                sequence,
                &snapshot,
                if sequence % 2 == 1 {
                    "scene-b"
                } else {
                    "scene-a"
                },
            ))
            .unwrap();
    }
    let before = owner.durable_sha256().unwrap();
    assert!(owner
        .append_event(activate(
            MAX_SCENE_JOURNAL_EVENTS as u64 + 1,
            129,
            &snapshot,
            "scene-b"
        ))
        .is_err());
    assert_eq!(owner.durable_sha256().unwrap(), before);
    let journal_path = owner.path().to_path_buf();
    drop(owner);

    fs::write(
        &journal_path,
        vec![b'x'; MAX_SCENE_JOURNAL_BYTES as usize + 1],
    )
    .unwrap();
    let oversized = fs::read(&journal_path).unwrap();
    assert!(SceneJournalOwner::open(&root, "scene-event-cap").is_err());
    assert_eq!(fs::read(journal_path).unwrap(), oversized);
}

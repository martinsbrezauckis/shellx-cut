use super::*;

fn marker(t_ms: u64) -> StudioEvent {
    StudioEvent {
        logical_ts: None,
        t_ms,
        source: "recording".into(),
        kind: "marker".into(),
        visible: None,
        x: None,
        y: None,
        size: None,
        shape: None,
        radius: None,
        label: Some("Mark".into()),
        background: None,
    }
}

#[tokio::test]
async fn owner_serializes_overlapping_event_requests() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(STUDIO_EVENTS_FILENAME);
    let owner = std::sync::Arc::new(tokio::sync::Mutex::new(StudioJournalOwner::default()));
    let mut background = marker(90);
    background.source = "background".into();
    background.kind = "style".into();
    background.label = None;
    background.background = Some("solid".into());
    let first_owner = owner.clone();
    let first_path = path.clone();
    let first = async move { first_owner.lock().await.append(&first_path, marker(120)) };
    let second_owner = owner.clone();
    let second_path = path.clone();
    let second = async move { second_owner.lock().await.append(&second_path, background) };
    let (first, second) = tokio::join!(first, second);
    first.unwrap();
    second.unwrap();
    let log = read_studio_journal(&path).unwrap();

    assert_eq!(log.events.len(), 2);
    assert_eq!(log.events[0].logical_ts, Some(1));
    assert_eq!(log.events[1].logical_ts, Some(2));
    assert_eq!(log.events[1].t_ms, 90, "client timing is preserved");
}

#[test]
fn recovers_a_crash_truncated_tail_before_the_next_append() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(STUDIO_EVENTS_FILENAME);
    let mut owner = StudioJournalOwner::default();
    owner.append(&path, marker(20)).unwrap();
    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(br#"{\"version\":1,\"logical_ts\":2,\"event\":"#)
        .unwrap();

    assert_eq!(read_studio_journal(&path).unwrap().events.len(), 1);
    let recovered = owner.append(&path, marker(40)).unwrap();
    assert_eq!(recovered.events.len(), 2);
    assert_eq!(recovered.events[1].logical_ts, Some(2));
    assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 2);
}

#[test]
fn first_append_is_readable_after_a_new_journal_is_created() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(STUDIO_EVENTS_FILENAME);
    StudioJournalOwner::default()
        .append(&path, marker(20))
        .unwrap();
    assert!(std::fs::read(&path).unwrap().ends_with(b"\n"));
    assert_eq!(read_studio_journal(&path).unwrap().events.len(), 1);
}

#[cfg(unix)]
#[test]
fn linked_journal_leaf_is_rejected_without_touching_its_target() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside");
    std::fs::write(&outside, b"").unwrap();
    let path = temp.path().join(STUDIO_EVENTS_FILENAME);
    symlink(&outside, &path).unwrap();

    assert!(StudioJournalOwner::default()
        .append(&path, marker(20))
        .is_err());
    assert!(read_studio_journal(&path).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"");

    std::fs::remove_file(&path).unwrap();
    std::fs::remove_file(&outside).unwrap();
    symlink(&outside, &path).unwrap();
    assert!(StudioJournalOwner::default()
        .append(&path, marker(20))
        .is_err());
    assert!(
        !outside.exists(),
        "dangling journal link must not create its target"
    );
}

#[test]
fn append_stops_at_the_size_limit_without_mutating_the_journal() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(STUDIO_EVENTS_FILENAME);
    let record = StudioJournalRecord {
        version: 1,
        logical_ts: 1,
        event: marker(20),
    };
    let record_len = serde_json::to_vec(&record).unwrap().len() + 1;
    let initial = vec![b'x'; MAX_STUDIO_EVENTS_JOURNAL_BYTES as usize - record_len];
    std::fs::write(&path, &initial).unwrap();
    append_record(&path, &record, initial.len() as u64).unwrap();
    let at_limit = std::fs::read(&path).unwrap();
    assert_eq!(at_limit.len(), MAX_STUDIO_EVENTS_JOURNAL_BYTES as usize);
    assert!(append_record(&path, &record, at_limit.len() as u64).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), at_limit);
}

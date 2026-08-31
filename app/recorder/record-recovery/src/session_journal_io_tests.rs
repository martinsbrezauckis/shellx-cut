use std::fs;

use crate::{
    CaptureRoot, CheckpointSequenceRange, DurableStateTransition, RecordingSessionIntent,
    RecordingSessionJournalEntry, RecordingSessionJournalFile, RecordingSessionState,
    RecordingStream, SealedRun, SessionTerminal, StreamFragment, StreamFragmentFacts,
    TerminalDisposition, RECORDING_SESSION_JOURNAL_FILE,
};

fn intent() -> RecordingSessionIntent {
    RecordingSessionIntent::new(
        "session-io-1",
        100,
        100,
        30.0,
        None,
        false,
        "opaque-target-io",
        vec![RecordingStream::ScreenVideo],
    )
}

fn transition(
    sequence: u64,
    state: RecordingSessionState,
    logical_offset_ms: u64,
    observed_unix_ms: u64,
) -> DurableStateTransition {
    DurableStateTransition {
        sequence,
        state,
        logical_offset_ms,
        observed_unix_ms,
    }
}

fn terminal(logical_end_ms: u64, observed_unix_ms: u64) -> SessionTerminal {
    SessionTerminal {
        disposition: TerminalDisposition::Completed,
        logical_end_ms,
        observed_unix_ms,
    }
}

fn run(artifact: &str) -> SealedRun {
    SealedRun {
        sequence: 0,
        observed_start_ms: 100,
        observed_end_ms: 200,
        logical_start_ms: 0,
        logical_end_ms: 100,
        checkpoints: CheckpointSequenceRange { first: 0, last: 0 },
        fragments: vec![StreamFragment {
            stream: RecordingStream::ScreenVideo,
            checkpoint_sequence: Some(0),
            stream_sequence: 0,
            artifact: artifact.into(),
            bytes: 1,
            sha256: "a".repeat(64),
            facts: StreamFragmentFacts {
                start_offset_ms: 0,
                end_offset_ms: 100,
                media_duration_ms: 100,
                decoded_video_frames: Some(1),
                avg_frame_rate: None,
                r_frame_rate: None,
            },
        }],
    }
}

fn valid_entries(artifact: &str) -> Vec<RecordingSessionJournalEntry> {
    vec![
        RecordingSessionJournalEntry::Intent(Box::new(intent())),
        RecordingSessionJournalEntry::Transition(transition(
            0,
            RecordingSessionState::Started,
            0,
            100,
        )),
        RecordingSessionJournalEntry::Run(run(artifact)),
        RecordingSessionJournalEntry::Transition(transition(
            1,
            RecordingSessionState::Stopping,
            100,
            300,
        )),
        RecordingSessionJournalEntry::Terminal(terminal(100, 400)),
    ]
}

fn setup_capture(capture_id: &str) -> (tempfile::TempDir, CaptureRoot) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project.cutproj");
    fs::create_dir(&project).unwrap();
    let root = CaptureRoot::for_project(&project).unwrap();
    root.create_capture_dir(capture_id).unwrap();
    (temp, root)
}

fn journal_path(root: &CaptureRoot, capture_id: &str) -> std::path::PathBuf {
    root.capture_file(capture_id, RECORDING_SESSION_JOURNAL_FILE)
        .unwrap()
}

fn write_entries(path: &std::path::Path, entries: &[RecordingSessionJournalEntry]) {
    let mut bytes = entries
        .iter()
        .map(serde_json::to_vec)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join(&b'\n');
    bytes.push(b'\n');
    fs::write(path, bytes).unwrap();
}

#[test]
fn creates_one_canonical_intent_entry_inside_the_fixed_capture_leaf() {
    let (_temp, root) = setup_capture("capture-io-create");
    let file =
        RecordingSessionJournalFile::create_new(&root, "capture-io-create", intent()).unwrap();
    let expected_path = journal_path(&root, "capture-io-create");
    let expected_bytes = [
        serde_json::to_vec(&RecordingSessionJournalEntry::Intent(Box::new(intent()))).unwrap(),
        b"\n".to_vec(),
    ]
    .concat();

    assert_eq!(file.path(), expected_path);
    assert_eq!(fs::read(file.path()).unwrap(), expected_bytes);
    assert_eq!(
        file.journal().entries(),
        vec![RecordingSessionJournalEntry::Intent(Box::new(intent()))]
    );
}

#[test]
fn open_replays_integer_v1_jsonl_fps_as_its_exact_f64_equivalent() {
    let (_temp, root) = setup_capture("capture-io-integer-fps");
    let path = journal_path(&root, "capture-io-integer-fps");
    let entries = valid_entries("screen/video-0.mp4");
    let mut serialized_entries = entries
        .iter()
        .map(serde_json::to_vec)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let canonical_fps = b"\"fps\":30.0";
    let legacy_fps = b"\"fps\":30";
    let fps_start = serialized_entries[0]
        .windows(canonical_fps.len())
        .position(|window| window == canonical_fps)
        .unwrap();
    serialized_entries[0].splice(
        fps_start..fps_start + canonical_fps.len(),
        legacy_fps.iter().copied(),
    );
    let mut bytes = serialized_entries.join(&b'\n');
    bytes.push(b'\n');
    fs::write(&path, &bytes).unwrap();

    let reopened = RecordingSessionJournalFile::open(&root, "capture-io-integer-fps").unwrap();

    assert!(bytes
        .windows(b"\"fps\":30".len())
        .any(|window| window == b"\"fps\":30"));
    assert_eq!(reopened.journal().intent().fps, 30.0);
}

#[test]
fn collision_does_not_replace_the_existing_journal() {
    let (_temp, root) = setup_capture("capture-io-collision");
    let first =
        RecordingSessionJournalFile::create_new(&root, "capture-io-collision", intent()).unwrap();
    let before = fs::read(first.path()).unwrap();

    assert!(
        RecordingSessionJournalFile::create_new(&root, "capture-io-collision", intent()).is_err()
    );
    assert_eq!(fs::read(first.path()).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn planted_symlink_is_rejected_without_touching_its_target() {
    use std::os::unix::fs::symlink;

    let (_temp, root) = setup_capture("capture-io-link");
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("sentinel.jsonl");
    fs::write(&sentinel, b"outside remains untouched").unwrap();
    let path = journal_path(&root, "capture-io-link");
    symlink(&sentinel, &path).unwrap();

    assert!(RecordingSessionJournalFile::create_new(&root, "capture-io-link", intent()).is_err());
    assert!(RecordingSessionJournalFile::open(&root, "capture-io-link").is_err());
    assert_eq!(fs::read(&sentinel).unwrap(), b"outside remains untouched");
}

#[test]
fn absolute_and_traversal_capture_ids_do_not_create_artifacts_outside_capture_root() {
    let (_temp, root) = setup_capture("capture-io-safe");
    let cache = root.cache_dir().to_path_buf();
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("sentinel.jsonl");
    fs::write(&sentinel, b"outside remains untouched").unwrap();

    for capture_id in [
        outside.path().to_string_lossy().as_ref(),
        "../outside",
        "nested/capture",
        "nested\\capture",
    ] {
        assert!(RecordingSessionJournalFile::create_new(&root, capture_id, intent()).is_err());
    }

    assert_eq!(fs::read(&sentinel).unwrap(), b"outside remains untouched");
    assert!(!cache.join("outside").exists());
}

#[test]
fn absolute_and_traversal_stream_artifacts_fail_closed_without_writing_elsewhere() {
    let (_temp, root) = setup_capture("capture-io-artifacts");
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("outside.mp4");
    fs::write(&sentinel, b"outside remains untouched").unwrap();
    let path = journal_path(&root, "capture-io-artifacts");

    for artifact in [
        sentinel.to_string_lossy().as_ref(),
        "../outside.mp4",
        "screen/../outside.mp4",
    ] {
        write_entries(&path, &valid_entries(artifact));
        let before = fs::read(&path).unwrap();
        assert!(RecordingSessionJournalFile::open(&root, "capture-io-artifacts").is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read(&sentinel).unwrap(), b"outside remains untouched");
    }
}

#[test]
fn truncated_final_jsonl_entry_is_not_replayed_or_repaired() {
    let (_temp, root) = setup_capture("capture-io-torn");
    let path = journal_path(&root, "capture-io-torn");
    let mut bytes = valid_entries("screen/video-0.mp4")
        .iter()
        .map(serde_json::to_vec)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join(&b'\n');
    bytes.push(b'\n');
    bytes.pop();
    fs::write(&path, &bytes).unwrap();

    assert!(RecordingSessionJournalFile::replay(&root, "capture-io-torn").is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(!path.parent().unwrap().join("quarantine").exists());
}

#[test]
fn duplicate_intent_and_terminal_entries_are_rejected_without_read_path_mutation() {
    let (_temp, root) = setup_capture("capture-io-duplicate");
    let path = journal_path(&root, "capture-io-duplicate");
    let cases = [
        vec![
            RecordingSessionJournalEntry::Intent(Box::new(intent())),
            RecordingSessionJournalEntry::Intent(Box::new(intent())),
        ],
        {
            let mut entries = valid_entries("screen/video-0.mp4");
            entries.push(RecordingSessionJournalEntry::Terminal(terminal(100, 400)));
            entries
        },
    ];

    for entries in cases {
        write_entries(&path, &entries);
        let before = fs::read(&path).unwrap();
        assert!(RecordingSessionJournalFile::open(&root, "capture-io-duplicate").is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

#[test]
fn append_after_terminal_and_duplicate_terminal_leave_journal_unchanged() {
    let (_temp, root) = setup_capture("capture-io-terminal");
    let mut file =
        RecordingSessionJournalFile::create_new(&root, "capture-io-terminal", intent()).unwrap();
    file.append_transition(transition(0, RecordingSessionState::Started, 0, 100))
        .unwrap();
    file.append_transition(transition(1, RecordingSessionState::Stopping, 0, 200))
        .unwrap();
    file.seal_terminal(terminal(0, 300)).unwrap();
    let before = fs::read(file.path()).unwrap();

    assert!(file
        .append_entry(RecordingSessionJournalEntry::Intent(Box::new(intent())))
        .is_err());
    assert!(file
        .append_entry(RecordingSessionJournalEntry::Terminal(terminal(0, 300)))
        .is_err());
    assert!(file
        .append_transition(transition(2, RecordingSessionState::Started, 0, 400))
        .is_err());
    assert_eq!(fs::read(file.path()).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn append_rejects_a_regular_leaf_replacement_instead_of_writing_the_old_inode() {
    let (_temp, root) = setup_capture("capture-io-replaced");
    let mut file =
        RecordingSessionJournalFile::create_new(&root, "capture-io-replaced", intent()).unwrap();
    let path = file.path().to_path_buf();
    let replacement = path.parent().unwrap().join(".replacement.jsonl");
    fs::write(&replacement, b"replacement journal remains authoritative").unwrap();
    fs::rename(&replacement, &path).unwrap();
    let before = fs::read(&path).unwrap();

    assert!(file
        .append_transition(transition(0, RecordingSessionState::Started, 0, 100))
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn read_only_replay_returns_exact_state_without_mutating_the_capture_directory() {
    let (_temp, root) = setup_capture("capture-io-read-only");
    let expected = valid_entries("screen/video-0.mp4");
    {
        let mut file =
            RecordingSessionJournalFile::create_new(&root, "capture-io-read-only", intent())
                .unwrap();
        file.append_transition(transition(0, RecordingSessionState::Started, 0, 100))
            .unwrap();
        file.seal_run(run("screen/video-0.mp4")).unwrap();
        file.append_transition(transition(1, RecordingSessionState::Stopping, 100, 300))
            .unwrap();
        file.seal_terminal(terminal(100, 400)).unwrap();
    }
    let path = journal_path(&root, "capture-io-read-only");
    let capture_dir = path.parent().unwrap();
    let before_bytes = fs::read(&path).unwrap();
    let mut before_children = fs::read_dir(capture_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    before_children.sort();

    let replayed = RecordingSessionJournalFile::replay(&root, "capture-io-read-only").unwrap();

    let mut after_children = fs::read_dir(capture_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    after_children.sort();
    assert_eq!(replayed.entries(), expected);
    assert_eq!(fs::read(&path).unwrap(), before_bytes);
    assert_eq!(after_children, before_children);
    assert!(!capture_dir.join("quarantine").exists());
}

#[test]
fn restart_reopens_and_replays_exact_durable_entries() {
    let (_temp, root) = setup_capture("capture-io-restart");
    let expected = valid_entries("screen/video-0.mp4");
    {
        let mut file =
            RecordingSessionJournalFile::create_new(&root, "capture-io-restart", intent()).unwrap();
        file.append_transition(transition(0, RecordingSessionState::Started, 0, 100))
            .unwrap();
        file.seal_run(run("screen/video-0.mp4")).unwrap();
        file.append_transition(transition(1, RecordingSessionState::Stopping, 100, 300))
            .unwrap();
        file.seal_terminal(terminal(100, 400)).unwrap();
    }

    let reopened = RecordingSessionJournalFile::open(&root, "capture-io-restart").unwrap();
    assert_eq!(reopened.journal().entries(), expected);
    assert_eq!(fs::read(reopened.path()).unwrap().last(), Some(&b'\n'));
}

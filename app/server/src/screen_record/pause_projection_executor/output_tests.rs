use super::*;
use record_recovery::{PrivateStaging, RecordingSessionIntent, RecordingStream};

const CAPTURE_ID: &str = "receipt-audio";
const MICROPHONE_BYTES: &[u8] = b"sealed microphone leaf";
const SYSTEM_BYTES: &[u8] = b"sealed system leaf";

fn fixture() -> (tempfile::TempDir, CaptureRoot, RecordingSessionJournal) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project.cutproj");
    std::fs::create_dir(&project).unwrap();
    let root = CaptureRoot::for_project(&project).unwrap();
    root.create_capture_dir(CAPTURE_ID).unwrap();
    let intent = RecordingSessionIntent::new(
        CAPTURE_ID,
        1,
        100,
        30.0,
        None,
        false,
        "private-target",
        vec![RecordingStream::ScreenVideo],
    );
    let journal = RecordingSessionJournal::new(intent).unwrap();
    (temp, root, journal)
}

fn selected_audio(capture_dir: &Path) -> ProjectionAudioContract {
    let contract = ProjectionAudioContract::test_with_selected_audio(
        Some(MICROPHONE_BYTES),
        Some(SYSTEM_BYTES),
        Some(17),
    );
    std::fs::write(capture_dir.join("mic.wav"), MICROPHONE_BYTES).unwrap();
    std::fs::write(capture_dir.join("system.wav"), SYSTEM_BYTES).unwrap();
    std::fs::write(
        capture_dir.join(crate::screen_record::system_audio::SYSTEM_AUDIO_TIMING_FILE),
        contract.test_system_timing_bytes().unwrap(),
    )
    .unwrap();
    contract
}

fn payloads() -> ProjectionPayloads {
    ProjectionPayloads {
        events: b"sealed events".to_vec(),
        project: b"sealed project".to_vec(),
    }
}

fn stage_source(capture_dir: &Path) -> PrivateStaging {
    let stage = PrivateStaging::create(capture_dir, "output-receipt-test", "source.mp4").unwrap();
    std::fs::write(stage.path(), b"sealed source").unwrap();
    stage
}

fn publish_completed(
    paths: &OutputPaths<'_>,
    journal: &RecordingSessionJournal,
    payloads: &ProjectionPayloads,
    audio: &ProjectionAudioContract,
) {
    let stage = stage_source(paths.capture_dir());
    publish(paths, journal, payloads, stage.path(), 1, &[], audio).unwrap();
}

#[test]
fn completed_receipt_binds_every_selected_audio_leaf_and_exact_timing_bytes() {
    let (_temp, root, journal) = fixture();
    let paths = OutputPaths::new(&root, CAPTURE_ID).unwrap();
    let audio = selected_audio(paths.capture_dir());
    let payloads = payloads();
    publish_completed(&paths, &journal, &payloads, &audio);

    let receipt = read_receipt(&paths.receipt).unwrap().unwrap();
    assert_eq!(receipt.audio, audio);
    assert!(completed(&paths, &journal, &payloads, 1, &[], &audio)
        .unwrap()
        .is_some());
    verify_completed_audio(&paths, &audio).unwrap();

    std::fs::remove_file(paths.capture_dir().join("mic.wav")).unwrap();
    assert!(completed(&paths, &journal, &payloads, 1, &[], &audio).is_err());
    assert!(verify_completed_audio(&paths, &audio).is_err());
    std::fs::write(paths.capture_dir().join("mic.wav"), MICROPHONE_BYTES).unwrap();

    let replacement = paths.capture_dir().join("replacement-system.wav");
    std::fs::write(&replacement, b"replaced system leaf").unwrap();
    std::fs::rename(&replacement, paths.capture_dir().join("system.wav")).unwrap();
    assert!(completed(&paths, &journal, &payloads, 1, &[], &audio).is_err());
    assert!(verify_completed_audio(&paths, &audio).is_err());
    std::fs::write(paths.capture_dir().join("system.wav"), SYSTEM_BYTES).unwrap();

    let parseable_but_changed = crate::screen_record::system_audio::SystemAudioTiming {
        schema: "shellx-cut/system-audio-timing/1".into(),
        first_packet_offset_ms: Some(18),
    };
    std::fs::write(
        paths
            .capture_dir()
            .join(crate::screen_record::system_audio::SYSTEM_AUDIO_TIMING_FILE),
        serde_json::to_vec(&parseable_but_changed).unwrap(),
    )
    .unwrap();
    assert!(completed(&paths, &journal, &payloads, 1, &[], &audio).is_err());
    assert!(verify_completed_audio(&paths, &audio).is_err());
}

#[test]
fn outer_publication_refuses_audio_changed_after_audio_publish() {
    let (_temp, root, journal) = fixture();
    let paths = OutputPaths::new(&root, CAPTURE_ID).unwrap();
    let audio = selected_audio(paths.capture_dir());
    let payloads = payloads();
    std::fs::write(
        paths.capture_dir().join("mic.wav"),
        b"tampered microphone leaf",
    )
    .unwrap();

    let stage = stage_source(paths.capture_dir());
    assert!(publish(&paths, &journal, &payloads, stage.path(), 1, &[], &audio).is_err());
    assert!(!paths.source().exists());
    assert!(!paths.receipt.exists());
}

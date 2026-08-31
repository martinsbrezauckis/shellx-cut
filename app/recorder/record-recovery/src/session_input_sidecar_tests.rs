use std::fs;

use crate::{
    CaptureRoot, DurableStateTransition, RecordingInputSidecarPin, RecordingProjectBinding,
    RecordingProjectIdentity, RecordingSessionIntent, RecordingSessionJournal,
    RecordingSessionJournalEntry, RecordingSessionJournalFile, RecordingSessionState,
    RecordingStream, RECORDING_PROJECT_ID_SCHEMA,
};

fn setup() -> (tempfile::TempDir, CaptureRoot) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project.cutproj");
    fs::create_dir(&project).unwrap();
    let root = CaptureRoot::for_project(&project).unwrap();
    root.create_capture_dir("input-sidecar-session").unwrap();
    (temp, root)
}

fn intent() -> RecordingSessionIntent {
    RecordingSessionIntent::new(
        "input-sidecar-session",
        100,
        100,
        30.0,
        None,
        false,
        "opaque-screen-target",
        vec![RecordingStream::ScreenVideo],
    )
    .with_project_binding(RecordingProjectBinding {
        identity: RecordingProjectIdentity {
            schema: RECORDING_PROJECT_ID_SCHEMA.into(),
            origin_path_sha256: format!("sha256:{}", "a".repeat(64)),
            project_name: "private-test".into(),
        },
        accepted_revision: "op_000001".into(),
    })
    .requiring_input_sidecars()
}

#[test]
fn input_sidecar_marker_admits_only_the_exact_ordered_selected_audio_set() {
    for streams in [
        vec![RecordingStream::ScreenVideo],
        vec![
            RecordingStream::ScreenVideo,
            RecordingStream::MicrophoneAudio,
        ],
        vec![RecordingStream::ScreenVideo, RecordingStream::SystemAudio],
        vec![
            RecordingStream::ScreenVideo,
            RecordingStream::MicrophoneAudio,
            RecordingStream::SystemAudio,
        ],
    ] {
        let intent = RecordingSessionIntent::new(
            "input-sidecar-session",
            100,
            100,
            30.0,
            None,
            false,
            "opaque-screen-target",
            streams,
        )
        .with_project_binding(RecordingProjectBinding {
            identity: RecordingProjectIdentity {
                schema: RECORDING_PROJECT_ID_SCHEMA.into(),
                origin_path_sha256: format!("sha256:{}", "a".repeat(64)),
                project_name: "private-test".into(),
            },
            accepted_revision: "op_000001".into(),
        })
        .requiring_input_sidecars();
        assert!(RecordingSessionJournal::new(intent).is_ok());
    }
}

#[test]
fn input_sidecar_marker_refuses_duplicate_reordered_or_unsupported_streams_and_keys() {
    for streams in [
        vec![
            RecordingStream::MicrophoneAudio,
            RecordingStream::ScreenVideo,
        ],
        vec![RecordingStream::ScreenVideo, RecordingStream::ScreenVideo],
        vec![RecordingStream::ScreenVideo, RecordingStream::CameraVideo],
        vec![RecordingStream::ScreenVideo, RecordingStream::InputEvents],
    ] {
        let intent = RecordingSessionIntent::new(
            "input-sidecar-session",
            100,
            100,
            30.0,
            None,
            false,
            "opaque-screen-target",
            streams,
        )
        .requiring_input_sidecars();
        assert!(RecordingSessionJournal::new(intent).is_err());
    }
    let keys = RecordingSessionIntent::new(
        "input-sidecar-session",
        100,
        100,
        30.0,
        None,
        true,
        "opaque-screen-target",
        vec![RecordingStream::ScreenVideo],
    )
    .requiring_input_sidecars();
    assert!(RecordingSessionJournal::new(keys).is_err());
}

#[test]
fn invalid_input_pin_does_not_mutate_the_cloned_or_durable_journal() {
    let (_temp, root) = setup();
    let mut file =
        RecordingSessionJournalFile::create_new(&root, "input-sidecar-session", intent()).unwrap();
    file.append_transition(DurableStateTransition {
        sequence: 0,
        state: RecordingSessionState::Started,
        logical_offset_ms: 0,
        observed_unix_ms: 100,
    })
    .unwrap();
    let before = fs::read(file.path()).unwrap();
    let invalid = RecordingInputSidecarPin {
        run_sequence: 0,
        file_name: "not-the-fixed-run-name.json".into(),
        sha256: "a".repeat(64),
        ready_transition_sequence: 0,
        ready_unix_ms: 100,
        native_ready_unix_ms: 100,
        native_ready_raw_ms: 0,
        raw_start_ms: 0,
        raw_end_ms: 100,
    };

    assert!(file
        .append_entry(RecordingSessionJournalEntry::InputSidecar(invalid))
        .is_err());
    assert_eq!(fs::read(file.path()).unwrap(), before);
    assert!(file.journal().input_sidecar(0).is_none());
}

#[test]
fn sidecar_required_run_cannot_omit_a_selected_audio_stream() {
    let mut intent = intent();
    intent.requested_streams = vec![
        RecordingStream::ScreenVideo,
        RecordingStream::MicrophoneAudio,
    ];
    let mut journal = RecordingSessionJournal::new(intent).unwrap();
    journal
        .append_transition(DurableStateTransition {
            sequence: 0,
            state: RecordingSessionState::Started,
            logical_offset_ms: 0,
            observed_unix_ms: 100,
        })
        .unwrap();
    journal
        .append_input_sidecar(RecordingInputSidecarPin {
            run_sequence: 0,
            file_name: "recording-input-run-000000.json".into(),
            sha256: "a".repeat(64),
            ready_transition_sequence: 0,
            ready_unix_ms: 100,
            native_ready_unix_ms: 100,
            native_ready_raw_ms: 0,
            raw_start_ms: 0,
            raw_end_ms: 100,
        })
        .unwrap();
    let screen_only = crate::SealedRun {
        sequence: 0,
        observed_start_ms: 0,
        observed_end_ms: 100,
        logical_start_ms: 0,
        logical_end_ms: 100,
        checkpoints: crate::CheckpointSequenceRange { first: 0, last: 0 },
        fragments: vec![crate::StreamFragment {
            stream: RecordingStream::ScreenVideo,
            checkpoint_sequence: Some(0),
            stream_sequence: 0,
            artifact: "screen.mp4".into(),
            bytes: 1,
            sha256: "a".repeat(64),
            facts: crate::StreamFragmentFacts {
                start_offset_ms: 0,
                end_offset_ms: 100,
                media_duration_ms: 100,
                decoded_video_frames: Some(1),
                avg_frame_rate: None,
                r_frame_rate: None,
            },
        }],
    };
    assert!(journal.seal_run(screen_only.clone()).is_err());

    let mut duplicate_audio = screen_only;
    for sequence in 0..2 {
        duplicate_audio.fragments.push(crate::StreamFragment {
            stream: RecordingStream::MicrophoneAudio,
            checkpoint_sequence: None,
            stream_sequence: sequence,
            artifact: format!("microphone-{sequence}.wav"),
            bytes: 1,
            sha256: "b".repeat(64),
            facts: crate::StreamFragmentFacts {
                start_offset_ms: sequence * 50,
                end_offset_ms: (sequence + 1) * 50,
                media_duration_ms: 50,
                decoded_video_frames: None,
                avg_frame_rate: None,
                r_frame_rate: None,
            },
        });
    }
    assert!(journal.seal_run(duplicate_audio).is_err());
}

#[test]
fn a_synced_input_pin_reopens_as_the_only_next_run_input() {
    let (_temp, root) = setup();
    let mut file =
        RecordingSessionJournalFile::create_new(&root, "input-sidecar-session", intent()).unwrap();
    file.append_transition(DurableStateTransition {
        sequence: 0,
        state: RecordingSessionState::Started,
        logical_offset_ms: 0,
        observed_unix_ms: 100,
    })
    .unwrap();
    let pin = RecordingInputSidecarPin {
        run_sequence: 0,
        file_name: "recording-input-run-000000.json".into(),
        sha256: "b".repeat(64),
        ready_transition_sequence: 0,
        ready_unix_ms: 100,
        native_ready_unix_ms: 100,
        native_ready_raw_ms: 0,
        raw_start_ms: 0,
        raw_end_ms: 100,
    };
    file.append_entry(RecordingSessionJournalEntry::InputSidecar(pin.clone()))
        .unwrap();
    drop(file);

    let reopened = RecordingSessionJournalFile::open(&root, "input-sidecar-session").unwrap();
    assert_eq!(reopened.journal().input_sidecar(0), Some(&pin));
    assert!(reopened.journal().input_sidecar(1).is_none());
    assert!(matches!(
        reopened.journal().entries().last(),
        Some(RecordingSessionJournalEntry::InputSidecar(value)) if value == &pin
    ));
}

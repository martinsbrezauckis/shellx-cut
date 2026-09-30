use super::*;
use crate::screen_record::pause_projection::SealedLegacyProjectionRun;
use record_core::{CursorCorrelation, EventTrack, Settings};
use record_recovery::{
    CheckpointSequenceRange, DurableStateTransition, RecordingInputSidecarPin,
    RecordingProjectBinding, RecordingSessionIntent, RecordingSessionJournal,
    RecordingSessionJournalFile, RecordingSessionState, RecordingStream, SealedRun, StreamFragment,
    StreamFragmentFacts,
};
use sha2::{Digest, Sha256};
use std::fs;

const CAPTURE: &str = "sidecar-capture";

fn root() -> (tempfile::TempDir, CaptureRoot) {
    let temp = tempfile::tempdir().unwrap();
    let store = cut_core::ProjectStore::create(temp.path(), "private-sidecar", None).unwrap();
    let root = CaptureRoot::for_project(&store.dir).unwrap();
    root.create_capture_dir(CAPTURE).unwrap();
    (temp, root)
}

fn binding(root: &CaptureRoot) -> RecordingProjectBinding {
    admit_project_binding(root).unwrap()
}

fn readiness() -> DurableStateTransition {
    DurableStateTransition {
        sequence: 0,
        state: RecordingSessionState::Started,
        logical_offset_ms: 0,
        observed_unix_ms: 1_000,
    }
}

fn run() -> SealedRun {
    SealedRun {
        sequence: 0,
        observed_start_ms: 0,
        observed_end_ms: 100,
        logical_start_ms: 0,
        logical_end_ms: 100,
        checkpoints: CheckpointSequenceRange { first: 0, last: 0 },
        fragments: vec![StreamFragment {
            stream: RecordingStream::ScreenVideo,
            checkpoint_sequence: Some(0),
            stream_sequence: 0,
            artifact: "checkpoints/segment-000000.mp4".into(),
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

fn run_with_audio(streams: &[RecordingStream]) -> SealedRun {
    let mut sealed = run();
    for stream in streams {
        let artifact = match stream {
            RecordingStream::MicrophoneAudio => {
                "recording-microphone-generation-00000000000000000001.wav"
            }
            RecordingStream::SystemAudio => "recording-system-generation-00000000000000000001.wav",
            _ => panic!("test accepts only owned audio streams"),
        };
        sealed.fragments.push(StreamFragment {
            stream: *stream,
            checkpoint_sequence: None,
            stream_sequence: 0,
            artifact: artifact.into(),
            bytes: 48,
            sha256: "a".repeat(64),
            facts: StreamFragmentFacts {
                start_offset_ms: 0,
                end_offset_ms: 100,
                media_duration_ms: 100,
                decoded_video_frames: None,
                avg_frame_rate: None,
                r_frame_rate: None,
            },
        });
    }
    sealed
        .fragments
        .sort_by_key(|fragment| (fragment.stream, fragment.stream_sequence));
    sealed
}

fn materialize_audio(root: &CaptureRoot, run: &mut SealedRun) {
    for fragment in run.fragments.iter_mut().filter(|fragment| {
        matches!(
            fragment.stream,
            RecordingStream::MicrophoneAudio | RecordingStream::SystemAudio
        )
    }) {
        let path = root.capture_file(CAPTURE, &fragment.artifact).unwrap();
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 1_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..100 {
            writer.write_sample::<i16>(0).unwrap();
        }
        writer.finalize().unwrap();
        let bytes = fs::read(&path).unwrap();
        fragment.bytes = u64::try_from(bytes.len()).unwrap();
        fragment.sha256 = format!("{:x}", Sha256::digest(bytes));
    }
}

fn evidence(run: SealedRun) -> SealedRunEvidence {
    evidence_with_native_ready(run, 1_000)
}

fn evidence_with_native_ready(run: SealedRun, native_ready_unix_ms: u64) -> SealedRunEvidence {
    let settings = Settings {
        width: 1920,
        height: 1080,
        fps: 30.0,
        audio_rate: 48_000,
    };
    let events = EventTrack {
        duration_ms: 100,
        screen_w: 1920,
        screen_h: 1080,
        monitors: Vec::new(),
        cursor: Vec::new(),
        clicks: Vec::new(),
        scrolls: Vec::new(),
        keys: Vec::new(),
        cursor_correlation: CursorCorrelation::default(),
    };
    let selected_streams = run
        .fragments
        .iter()
        .map(|fragment| fragment.stream)
        .collect::<Vec<_>>();
    let audio = run
        .fragments
        .iter()
        .filter(|fragment| {
            matches!(
                fragment.stream,
                RecordingStream::MicrophoneAudio | RecordingStream::SystemAudio
            )
        })
        .map(|fragment| {
            crate::screen_record::windows_pause_evidence::RecordingAudioDraft::test_audio(
                fragment.stream,
                1,
                fragment.artifact.clone(),
                fragment.bytes,
                fragment.sha256.clone(),
                50,
                150,
            )
        })
        .collect();
    SealedRunEvidence::new(
        0,
        run,
        SealedLegacyProjectionRun::new(0, 0, 100, settings, events),
        selected_streams,
    )
    .with_recording_input(
        crate::screen_record::windows_pause_evidence::RecordingInputDraft::test_screen_only_with_native_ready(
            "shellx-monitor-v1:windows:test".into(),
            10,
            20,
            1920,
            1080,
            1920,
            1080,
            30,
            native_ready_unix_ms,
            50,
            150,
        )
        .with_test_audio(audio),
    )
}

fn journal(
    run: SealedRun,
    pin: RecordingInputSidecarPin,
    project_binding: RecordingProjectBinding,
) -> RecordingSessionJournal {
    let intent = RecordingSessionIntent::new(
        CAPTURE,
        1_000,
        100,
        30.0,
        None,
        false,
        "shellx-monitor-v1:windows:test",
        run.fragments
            .iter()
            .map(|fragment| fragment.stream)
            .collect(),
    )
    .with_project_binding(project_binding)
    .requiring_input_sidecars();
    let mut journal = RecordingSessionJournal::new(intent).unwrap();
    journal.append_transition(readiness()).unwrap();
    journal.append_input_sidecar(pin).unwrap();
    journal.seal_run(run).unwrap();
    journal
}

#[test]
fn publishes_once_then_reopens_only_the_identical_sidecar() {
    let (_temp, root) = root();
    let evidence = evidence(run());
    let ready = readiness();
    let owner = WindowsPauseInputSidecarOwner::new(root.clone(), CAPTURE.into(), binding(&root));
    let first = owner.publish_or_reopen(&evidence, &ready).unwrap();
    let retry = owner.publish_or_reopen(&evidence, &ready).unwrap();

    assert_eq!(first, retry);
    validation::verify_pinned_run(&root, CAPTURE, evidence.run(), &ready, &first).unwrap();
}

#[test]
fn missing_or_tampered_pinned_sidecar_is_rejected_without_input_inference() {
    let (_temp, root) = root();
    let evidence = evidence(run());
    let ready = readiness();
    let project = binding(&root);
    let owner = WindowsPauseInputSidecarOwner::new(root.clone(), CAPTURE.into(), project.clone());
    let pin = owner.publish_or_reopen(&evidence, &ready).unwrap();
    let journal = journal(evidence.run().clone(), pin.clone(), project);
    let path = root.capture_file(CAPTURE, &pin.file_name).unwrap();

    fs::remove_file(&path).unwrap();
    assert!(validation::verify_pinned_inputs(&root, CAPTURE, &journal).is_err());
    fs::write(&path, b"tampered").unwrap();
    assert!(validation::verify_pinned_inputs(&root, CAPTURE, &journal).is_err());
}

#[test]
fn recovery_replays_only_the_exact_durable_sidecar_pin() {
    let (_temp, root) = root();
    let evidence = evidence(run());
    let ready = readiness();
    let project = binding(&root);
    let owner = WindowsPauseInputSidecarOwner::new(root.clone(), CAPTURE.into(), project.clone());
    let pin = owner.publish_or_reopen(&evidence, &ready).unwrap();
    let intent = RecordingSessionIntent::new(
        CAPTURE,
        1_000,
        100,
        30.0,
        None,
        false,
        "shellx-monitor-v1:windows:test",
        vec![RecordingStream::ScreenVideo],
    )
    .with_project_binding(project)
    .requiring_input_sidecars();
    let mut file = RecordingSessionJournalFile::create_new(&root, CAPTURE, intent).unwrap();
    file.append_transition(ready).unwrap();
    file.append_entry(record_recovery::RecordingSessionJournalEntry::InputSidecar(
        pin.clone(),
    ))
    .unwrap();
    file.seal_run(evidence.run().clone()).unwrap();
    drop(file);

    validation::verify_recovery_capture(&root, CAPTURE).unwrap();
    fs::write(
        root.capture_file(CAPTURE, &pin.file_name).unwrap(),
        b"tampered",
    )
    .unwrap();
    assert!(validation::verify_recovery_capture(&root, CAPTURE).is_err());
}

#[test]
fn preexisting_different_sidecar_fails_without_replacement() {
    let (_temp, root) = root();
    let evidence = evidence(run());
    let path = root
        .capture_file(CAPTURE, &file_name(evidence.run().sequence))
        .unwrap();
    fs::write(&path, b"different-private-sidecar").unwrap();
    let owner = WindowsPauseInputSidecarOwner::new(root.clone(), CAPTURE.into(), binding(&root));

    assert!(owner.publish_or_reopen(&evidence, &readiness()).is_err());
    assert_eq!(fs::read(path).unwrap(), b"different-private-sidecar");
}

#[test]
fn stale_project_identity_is_rejected_even_with_a_rehashed_journal_pin() {
    let (_temp, root) = root();
    let evidence = evidence(run());
    let ready = readiness();
    let project = binding(&root);
    let owner = WindowsPauseInputSidecarOwner::new(root.clone(), CAPTURE.into(), project.clone());
    let pin = owner.publish_or_reopen(&evidence, &ready).unwrap();
    let path = root.capture_file(CAPTURE, &pin.file_name).unwrap();
    let mut sidecar: InputSidecar = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    sidecar.body.project.identity.project_name = "stale-project".into();
    sidecar.fingerprint_sha256 = digest(&canonical(&sidecar.body).unwrap());
    let bytes = canonical(&sidecar).unwrap();
    fs::write(path, &bytes).unwrap();
    let stale_pin = RecordingInputSidecarPin {
        sha256: digest(&bytes),
        ..pin
    };

    assert!(validation::verify_pinned_inputs(
        &root,
        CAPTURE,
        &journal(evidence.run().clone(), stale_pin, project),
    )
    .is_err());
}

#[test]
fn microphone_or_system_audio_is_pinned_only_when_selected() {
    for stream in [
        RecordingStream::MicrophoneAudio,
        RecordingStream::SystemAudio,
    ] {
        let (_temp, root) = root();
        let mut sealed = run_with_audio(&[stream]);
        materialize_audio(&root, &mut sealed);
        let evidence = evidence(sealed);
        let ready = readiness();
        let project = binding(&root);
        let owner =
            WindowsPauseInputSidecarOwner::new(root.clone(), CAPTURE.into(), project.clone());
        let pin = owner.publish_or_reopen(&evidence, &ready).unwrap();
        let journal = journal(evidence.run().clone(), pin.clone(), project);
        let bytes = fs::read(root.capture_file(CAPTURE, &pin.file_name).unwrap()).unwrap();
        let sidecar: InputSidecar = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(sidecar.body.audio.len(), 1);
        let source = &sidecar.body.audio[0];
        assert_eq!(source.stream(), stream);
        assert_eq!(source.source_generation, 1);
        assert_eq!(source.raw_start_ms, 50);
        assert_eq!(source.raw_end_ms, 150);
        assert_eq!(source.logical_start_ms, 0);
        assert_eq!(source.logical_end_ms, 100);
        assert!(validation::verify_pinned_inputs(&root, CAPTURE, &journal).is_ok());
    }
}

#[test]
fn combined_audio_sidecar_recovery_rejects_rehashed_stale_or_duplicate_source() {
    let (_temp, root) = root();
    let mut sealed = run_with_audio(&[
        RecordingStream::MicrophoneAudio,
        RecordingStream::SystemAudio,
    ]);
    materialize_audio(&root, &mut sealed);
    let evidence = evidence(sealed);
    let ready = readiness();
    let project = binding(&root);
    let owner = WindowsPauseInputSidecarOwner::new(root.clone(), CAPTURE.into(), project.clone());
    let pin = owner.publish_or_reopen(&evidence, &ready).unwrap();
    let path = root.capture_file(CAPTURE, &pin.file_name).unwrap();
    let journal = journal(evidence.run().clone(), pin.clone(), project);

    assert!(validation::verify_pinned_inputs(&root, CAPTURE, &journal).is_ok());
    let intent = RecordingSessionIntent::new(
        CAPTURE,
        1_000,
        100,
        30.0,
        None,
        false,
        "shellx-monitor-v1:windows:test",
        evidence
            .run()
            .fragments
            .iter()
            .map(|fragment| fragment.stream)
            .collect(),
    )
    .with_project_binding(journal.intent().project_binding.clone().unwrap())
    .requiring_input_sidecars();
    let mut replay = RecordingSessionJournalFile::create_new(&root, CAPTURE, intent).unwrap();
    replay.append_transition(ready.clone()).unwrap();
    replay
        .append_entry(record_recovery::RecordingSessionJournalEntry::InputSidecar(
            pin.clone(),
        ))
        .unwrap();
    replay.seal_run(evidence.run().clone()).unwrap();
    drop(replay);
    assert!(validation::verify_recovery_capture(&root, CAPTURE).is_ok());
    let mut sidecar: InputSidecar = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    sidecar.body.audio[0].source_generation += 1;
    sidecar.fingerprint_sha256 = digest(&canonical(&sidecar.body).unwrap());
    let bytes = canonical(&sidecar).unwrap();
    fs::write(&path, &bytes).unwrap();
    let stale_pin = RecordingInputSidecarPin {
        sha256: digest(&bytes),
        ..pin.clone()
    };
    assert!(
        validation::verify_pinned_run(&root, CAPTURE, evidence.run(), &ready, &stale_pin).is_err()
    );

    let mut sidecar: InputSidecar = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    sidecar.body.audio[0].source_generation = 1;
    let mut missing = sidecar.clone();
    missing.body.audio.remove(1);
    missing.fingerprint_sha256 = digest(&canonical(&missing.body).unwrap());
    let bytes = canonical(&missing).unwrap();
    fs::write(&path, &bytes).unwrap();
    let missing_pin = RecordingInputSidecarPin {
        sha256: digest(&bytes),
        ..pin.clone()
    };
    assert!(
        validation::verify_pinned_run(&root, CAPTURE, evidence.run(), &ready, &missing_pin)
            .is_err()
    );

    sidecar.body.audio.push(sidecar.body.audio[0].clone());
    sidecar.fingerprint_sha256 = digest(&canonical(&sidecar.body).unwrap());
    let bytes = canonical(&sidecar).unwrap();
    fs::write(&path, &bytes).unwrap();
    let duplicate_pin = RecordingInputSidecarPin {
        sha256: digest(&bytes),
        ..pin
    };
    assert!(
        validation::verify_pinned_run(&root, CAPTURE, evidence.run(), &ready, &duplicate_pin)
            .is_err()
    );
}

#[test]
fn recovery_refuses_missing_replaced_or_tampered_selected_wav() {
    let (_temp, root) = root();
    let mut sealed = run_with_audio(&[RecordingStream::MicrophoneAudio]);
    materialize_audio(&root, &mut sealed);
    let evidence = evidence(sealed);
    let ready = readiness();
    let project = binding(&root);
    let owner = WindowsPauseInputSidecarOwner::new(root.clone(), CAPTURE.into(), project.clone());
    let pin = owner.publish_or_reopen(&evidence, &ready).unwrap();
    let journal = journal(evidence.run().clone(), pin, project);
    let wav = root
        .capture_file(
            CAPTURE,
            "recording-microphone-generation-00000000000000000001.wav",
        )
        .unwrap();

    assert!(validation::verify_pinned_inputs(&root, CAPTURE, &journal).is_ok());
    fs::remove_file(&wav).unwrap();
    assert!(validation::verify_pinned_inputs(&root, CAPTURE, &journal).is_err());

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 1_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&wav, spec).unwrap();
    for _ in 0..99 {
        writer.write_sample::<i16>(1).unwrap();
    }
    writer.finalize().unwrap();
    assert!(validation::verify_pinned_inputs(&root, CAPTURE, &journal).is_err());
    fs::write(&wav, b"not-a-wav").unwrap();
    assert!(validation::verify_pinned_inputs(&root, CAPTURE, &journal).is_err());
}

#[cfg(unix)]
#[test]
fn recovery_refuses_a_selected_wav_replaced_by_a_link() {
    use std::os::unix::fs::symlink;

    let (temp, root) = root();
    let mut sealed = run_with_audio(&[RecordingStream::SystemAudio]);
    materialize_audio(&root, &mut sealed);
    let evidence = evidence(sealed);
    let ready = readiness();
    let project = binding(&root);
    let owner = WindowsPauseInputSidecarOwner::new(root.clone(), CAPTURE.into(), project.clone());
    let pin = owner.publish_or_reopen(&evidence, &ready).unwrap();
    let journal = journal(evidence.run().clone(), pin, project);
    let wav = root
        .capture_file(
            CAPTURE,
            "recording-system-generation-00000000000000000001.wav",
        )
        .unwrap();
    let outside = temp.path().join("outside.wav");
    fs::write(&outside, b"outside").unwrap();
    fs::remove_file(&wav).unwrap();
    symlink(&outside, &wav).unwrap();

    assert!(validation::verify_pinned_inputs(&root, CAPTURE, &journal).is_err());
}

#[path = "windows_pause_input_sidecar_tests/binding.rs"]
mod binding;

#[path = "windows_pause_input_sidecar_tests/bounds.rs"]
mod bounds;

use super::*;
use record_recovery::{CaptureStart, CheckpointFacts, ManifestOwner, MediaFacts};
use sha2::{Digest, Sha256};
use std::fs::{remove_file, rename, write};

fn checkpoint(root: &Path) -> Checkpoint {
    let mut owner = ManifestOwner::begin(root, CaptureStart::new("pause", 100)).unwrap();
    let staging = owner.begin_segment(0, 0).unwrap();
    write(&staging, b"closed screen video").unwrap();
    owner
        .publish(
            0,
            &staging,
            CheckpointFacts {
                start_ms: 0,
                end_ms: 100,
                event_offset_ms: 0,
                audio_offset_ms: None,
            },
            MediaFacts {
                duration_ms: 100,
                decoded_video_frames: 1,
                has_audio: false,
                width: None,
                height: None,
                codec_name: None,
                avg_frame_rate: None,
                r_frame_rate: None,
            },
        )
        .unwrap()
}

fn audio(stream: RecordingStream, bytes: &[u8]) -> WindowsSealedAudioRun {
    let artifact = match stream {
        RecordingStream::MicrophoneAudio => {
            "recording-microphone-generation-00000000000000000001.wav"
        }
        RecordingStream::SystemAudio => "recording-system-generation-00000000000000000001.wav",
        _ => panic!("test requires a supported audio owner"),
    };
    WindowsSealedAudioRun {
        stream,
        source_generation: 1,
        artifact: artifact.into(),
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
        media_duration_ms: 100,
        native_ready_unix_ms: 1_000,
        native_ready_raw_ms: 1,
        raw_start_ms: 0,
        raw_end_ms: 100,
    }
}

#[test]
fn owned_manifest_matching_leaf_and_hash_are_accepted() {
    let root = tempfile::tempdir().unwrap();
    let checkpoint = checkpoint(root.path());
    assert!(
        LocalWindowsPauseArtifactVerifier::new(root.path().to_path_buf())
            .verify(&checkpoint)
            .is_ok()
    );
}

#[test]
fn wrong_manifest_hash_or_length_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let checkpoint = checkpoint(root.path());
    let verifier = LocalWindowsPauseArtifactVerifier::new(root.path().to_path_buf());
    let mut wrong_hash = checkpoint.clone();
    wrong_hash.sha256 = "b".repeat(64);
    assert!(verifier.verify(&wrong_hash).is_err());
    let mut wrong_length = checkpoint;
    wrong_length.bytes += 1;
    assert!(verifier.verify(&wrong_length).is_err());
}

#[test]
fn replacement_after_open_has_a_different_file_identity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("segment.mp4");
    let replacement = directory.path().join("replacement.mp4");
    write(&path, b"first").unwrap();
    let first = open_nofollow(&path).unwrap();
    write(&replacement, b"second").unwrap();
    rename(replacement, &path).unwrap();
    let current = open_nofollow(&path).unwrap();
    assert!(!same_open_file(&first, &current).unwrap());
}

#[test]
fn replacement_between_hash_and_reopen_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let checkpoint = checkpoint(root.path());
    let verifier = LocalWindowsPauseArtifactVerifier::new(root.path().to_path_buf());
    assert!(verifier
        .verify_after_hash(&checkpoint, |path| {
            let replacement = path.with_extension("replacement");
            write(&replacement, b"closed screen video").unwrap();
            rename(replacement, path).unwrap();
        })
        .is_err());
}

#[test]
fn exact_selected_audio_leaf_hash_and_fixed_generation_name_are_required() {
    let root = tempfile::tempdir().unwrap();
    let bytes = b"closed microphone wav";
    let source = audio(RecordingStream::MicrophoneAudio, bytes);
    let path = root.path().join(&source.artifact);
    write(&path, bytes).unwrap();
    let verifier = LocalWindowsPauseArtifactVerifier::new(root.path().to_path_buf());
    assert!(verifier.verify_audio(&source).is_ok());
    write(&path, b"tampered microphone wav").unwrap();
    assert!(verifier.verify_audio(&source).is_err());
    let mut wrong_generation = source;
    wrong_generation.source_generation = 2;
    assert!(verifier.verify_audio(&wrong_generation).is_err());
}

#[test]
fn symlink_leaf_is_rejected_before_open() {
    let root = tempfile::tempdir().unwrap();
    let checkpoint = checkpoint(root.path());
    let path = root.path().join(&checkpoint.file);
    let outside = root.path().join("outside.mp4");
    write(&outside, b"closed screen video").unwrap();
    remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    assert!(
        LocalWindowsPauseArtifactVerifier::new(root.path().to_path_buf())
            .verify(&checkpoint)
            .is_err()
    );
}

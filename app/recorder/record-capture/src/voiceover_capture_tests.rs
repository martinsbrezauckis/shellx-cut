use std::path::Path;

use super::{
    cancel_terminal, sealed_terminal, validate_target, CapturedMicrophone, VoiceoverCaptureOutcome,
};
use crate::mic::MicRecordingGate;

fn capture(path: Option<&Path>, microphone_lost: bool) -> CapturedMicrophone {
    CapturedMicrophone {
        path: path.map(|path| path.to_string_lossy().into_owned()),
        microphone_lost,
        first_packet_offset_ms: path.map(|_| 0),
    }
}

fn wav(path: &Path, samples: &[i16]) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 1_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for sample in samples {
        writer.write_sample(*sample).unwrap();
    }
    writer.finalize().unwrap();
}

#[test]
fn saved_prefix_is_measured_and_device_loss_remains_explicit() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("voiceover.wav");
    wav(&path, &[1, 2, 3]);
    let VoiceoverCaptureOutcome::DeviceLostSavedPrefix(artifact) =
        sealed_terminal(capture(Some(&path), true), &path)
    else {
        panic!("saved prefix must stay distinct from an ordinary take");
    };
    assert_eq!(artifact.duration_ms, 3);
    assert_eq!(artifact.sample_rate_hz, 1_000);
    assert_eq!(artifact.channels, 1);
    assert_eq!(artifact.sha256.len(), 64);
}

#[test]
fn zero_samples_never_fabricates_an_artifact() {
    let path = Path::new("voiceover.wav");
    assert_eq!(
        sealed_terminal(capture(None, false), path),
        VoiceoverCaptureOutcome::ZeroSamples
    );
}

#[test]
fn device_loss_without_a_prefix_is_not_reported_as_an_empty_take() {
    let path = Path::new("voiceover.wav");
    assert_eq!(
        sealed_terminal(capture(None, true), path),
        VoiceoverCaptureOutcome::DeviceLostNoSamples
    );
}

#[test]
fn recording_gate_is_one_way_and_has_no_origin_before_the_take() {
    let gate = MicRecordingGate::default();
    assert_eq!(gate.origin(), None);
    let first = gate.arm().unwrap();
    assert_eq!(gate.origin(), Some(first));
    assert!(gate.arm().is_err());
}

#[test]
fn cancellation_removes_only_the_sealed_regular_artifact() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("voiceover.wav");
    wav(&path, &[1]);
    assert_eq!(cancel_terminal(&path), VoiceoverCaptureOutcome::Cancelled);
    assert!(!path.exists());
}

#[test]
fn target_mismatch_refuses_adoption() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("voiceover.wav");
    let other = temp.path().join("other.wav");
    wav(&other, &[1]);
    assert!(matches!(
        sealed_terminal(capture(Some(&other), false), &target),
        VoiceoverCaptureOutcome::Failed(_)
    ));
}

#[test]
fn existing_target_is_refused_before_any_microphone_worker_can_start() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("voiceover.wav");
    wav(&path, &[1]);
    let error = validate_target(&path).unwrap_err();
    assert_eq!(error.code, record_core::error_codes::GUARDRAIL);
}

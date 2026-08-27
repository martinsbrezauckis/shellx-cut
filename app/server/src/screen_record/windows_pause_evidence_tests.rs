use super::*;
use crate::screen_record::windows_pause_adapter::WindowsPauseEvidenceFactory;
use record_capture::windows_pause_pilot::{
    WindowsPausePilotAcceptedCapture, WindowsPausePilotCaptureRange,
    WindowsPausePilotCheckpointRange, WindowsSealedWgcCheckpoint,
};
use record_recovery::{Checkpoint, CheckpointFacts, MediaFacts};
use std::time::{Duration, Instant};

struct AcceptArtifacts;
impl WindowsPauseArtifactVerifier for AcceptArtifacts {
    fn verify(&self, _: &Checkpoint) -> Result<(), WindowsPauseAdapterError> {
        Ok(())
    }
}

struct RejectArtifacts;
impl WindowsPauseArtifactVerifier for RejectArtifacts {
    fn verify(&self, _: &Checkpoint) -> Result<(), WindowsPauseAdapterError> {
        Err(WindowsPauseAdapterError::EvidenceRejected)
    }
}

fn at(origin: Instant, ms: u64) -> Instant {
    origin + Duration::from_millis(ms)
}
fn settings() -> Settings {
    Settings {
        width: 640,
        height: 360,
        fps: 30.0,
        audio_rate: 48_000,
    }
}
fn accepted() -> WindowsPausePilotAcceptedCapture {
    WindowsPausePilotAcceptedCapture {
        settings: settings(),
        range: WindowsPausePilotCaptureRange {
            origin_x: 0,
            origin_y: 0,
            width: 640,
            height: 360,
        },
    }
}
fn started(origin: Instant, raw: u64, at_ms: u64) -> WindowsPausePilotStarted {
    WindowsPausePilotStarted {
        physical_generation: 1,
        observed_start_ms: raw,
        monotonic_at: at(origin, at_ms),
        unix_ms: 1_000 + at_ms,
        accepted: accepted(),
    }
}
fn checkpoint(sequence: u64, start_ms: u64, end_ms: u64) -> WindowsSealedWgcCheckpoint {
    WindowsSealedWgcCheckpoint {
        physical_generation: sequence + 1,
        start_ms,
        end_ms,
        checkpoint: Checkpoint {
            sequence,
            file: format!("checkpoints/segment-{sequence:06}.mp4"),
            bytes: 1,
            sha256: "a".repeat(64),
            media: Some(MediaFacts {
                duration_ms: end_ms - start_ms,
                decoded_video_frames: 1,
                has_audio: false,
                avg_frame_rate: None,
                r_frame_rate: None,
            }),
            facts: CheckpointFacts {
                start_ms,
                end_ms,
                event_offset_ms: start_ms,
                audio_offset_ms: None,
            },
        },
    }
}
fn run(start_ms: u64, end_ms: u64) -> WindowsSealedScreenRun {
    WindowsSealedScreenRun {
        observed_start_ms: start_ms,
        observed_end_ms: end_ms,
        accepted: accepted(),
        range: WindowsPausePilotCheckpointRange {
            first_physical_generation: 1,
            last_physical_generation: 1,
            first_checkpoint_sequence: 0,
            last_checkpoint_sequence: 0,
        },
        checkpoints: vec![checkpoint(0, start_ms, end_ms)],
    }
}
fn factory(
    first: &WindowsPausePilotStarted,
) -> CalibratedWindowsPauseEvidenceFactory<AcceptArtifacts> {
    let mut factory = CalibratedWindowsPauseEvidenceFactory::new(AcceptArtifacts);
    factory.stage_started(None, first).unwrap();
    factory
        .set_session_origin(SessionTimeOrigin::observed(
            first.monotonic_at,
            first.unix_ms,
        ))
        .unwrap();
    factory
}

#[test]
fn initial_nonzero_raw_start_seals_server_zero_without_raw_delta_normalization() {
    let origin = Instant::now();
    let first = started(origin, 77, 0);
    let evidence = factory(&first)
        .verify_and_build(1, &run(77, 177), at(origin, 130))
        .unwrap();
    assert_eq!(evidence.run().sequence, 0);
    assert_eq!(evidence.run().observed_start_ms, 0);
    assert_eq!(evidence.run().observed_end_ms, 130);
    assert_eq!(evidence.run().logical_end_ms, 100);
}

#[test]
fn raw_start_generation_and_post_close_window_must_be_exact() {
    let origin = Instant::now();
    let first = started(origin, 77, 0);
    assert!(factory(&first)
        .verify_and_build(1, &run(78, 178), at(origin, 130))
        .is_err());
    assert!(factory(&first)
        .verify_and_build(1, &run(77, 177), at(origin, 99))
        .is_err());

    let mut resumed = factory(&first);
    resumed
        .verify_and_build(1, &run(77, 177), at(origin, 130))
        .unwrap();
    let second = started(origin, 999, 200);
    resumed.stage_started(Some(2), &second).unwrap();
    assert!(resumed
        .verify_and_build(3, &run(999, 1_099), at(origin, 320))
        .is_err());
}

#[test]
fn resumed_raw_binding_uses_exact_server_start_and_raw_logical_span() {
    let origin = Instant::now();
    let first = started(origin, 77, 0);
    let mut factory = factory(&first);
    factory
        .verify_and_build(1, &run(77, 177), at(origin, 130))
        .unwrap();
    let second = started(origin, 999, 200);
    factory.stage_started(Some(2), &second).unwrap();
    let evidence = factory
        .verify_and_build(2, &run(999, 1_099), at(origin, 320))
        .unwrap();
    assert_eq!(evidence.run().sequence, 1);
    assert_eq!(evidence.run().observed_start_ms, 200);
    assert_eq!(evidence.run().observed_end_ms, 320);
    assert_eq!(evidence.run().logical_start_ms, 100);
    assert_eq!(evidence.run().logical_end_ms, 200);
}

#[test]
fn settings_range_checkpoint_and_artifact_rejections_are_fail_closed() {
    let origin = Instant::now();
    let first = started(origin, 77, 0);
    let mut bad_range = run(77, 177);
    bad_range.accepted.range.width = 1;
    assert!(factory(&first)
        .verify_and_build(1, &bad_range, at(origin, 130))
        .is_err());
    let mut bad_checkpoint = run(77, 177);
    bad_checkpoint.checkpoints[0]
        .checkpoint
        .facts
        .event_offset_ms = 1;
    assert!(factory(&first)
        .verify_and_build(1, &bad_checkpoint, at(origin, 130))
        .is_err());
    let mut artifacts = CalibratedWindowsPauseEvidenceFactory::new(RejectArtifacts);
    artifacts.stage_started(None, &first).unwrap();
    artifacts
        .set_session_origin(SessionTimeOrigin::observed(
            first.monotonic_at,
            first.unix_ms,
        ))
        .unwrap();
    assert!(artifacts
        .verify_and_build(1, &run(77, 177), at(origin, 130))
        .is_err());
}

#[test]
fn physical_checkpoint_gap_is_provenance_but_overlap_is_rejected() {
    let origin = Instant::now();
    let first = started(origin, 77, 0);
    let mut gapped = run(77, 177);
    gapped.range.last_physical_generation = 2;
    gapped.range.last_checkpoint_sequence = 1;
    gapped.checkpoints = vec![checkpoint(0, 77, 117), checkpoint(1, 130, 177)];
    assert!(factory(&first)
        .verify_and_build(1, &gapped, at(origin, 130))
        .is_ok());
    let mut overlap = gapped;
    overlap.checkpoints[1].start_ms = 116;
    overlap.checkpoints[1].checkpoint.facts.start_ms = 116;
    overlap.checkpoints[1].checkpoint.facts.event_offset_ms = 116;
    assert!(factory(&first)
        .verify_and_build(1, &overlap, at(origin, 130))
        .is_err());
}

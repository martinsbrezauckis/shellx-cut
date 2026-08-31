use super::journal::VoiceoverTerminalResult;
use super::journal_io::VoiceoverTakeJournalFile;
use super::session::take_id_for;
use super::session::{VoiceoverTakePreparation, VoiceoverTakeSession};
use super::*;
use cut_core::ProjectSettings;
use record_recovery::CaptureRoot;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Debug)]
struct FakeCapture {
    ready: bool,
    stop: VoiceoverFinalize,
    cancel: VoiceoverFinalize,
    status: VoiceoverFinalize,
    status_once: Option<VoiceoverFinalize>,
    begin_calls: Arc<AtomicUsize>,
    stop_calls: Arc<AtomicUsize>,
    cancel_calls: Arc<AtomicUsize>,
}

impl VoiceoverCaptureControl for FakeCapture {
    fn microphone_ready(&self) -> bool {
        self.ready
    }

    fn begin_recording(&mut self) -> Result<(), CutError> {
        self.begin_calls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn stop(&mut self) -> VoiceoverFinalize {
        self.stop_calls.fetch_add(1, Ordering::Relaxed);
        self.stop.clone()
    }

    fn cancel(&mut self) -> VoiceoverFinalize {
        self.cancel_calls.fetch_add(1, Ordering::Relaxed);
        self.cancel.clone()
    }

    fn status(&mut self) -> VoiceoverFinalize {
        self.status_once
            .take()
            .unwrap_or_else(|| self.status.clone())
    }
}

fn project() -> Project {
    Project::new("voiceover", ProjectSettings::default())
}

fn request(cue: VoiceoverCue) -> VoiceoverTimelineRequest {
    VoiceoverTimelineRequest {
        request_id: "voiceover-request-1".into(),
        expected_revision: "op_000004".into(),
        audio_track: "a1t".into(),
        cue,
    }
}

fn capture(ready: bool) -> (FakeCapture, Arc<AtomicUsize>) {
    let begin_calls = Arc::new(AtomicUsize::new(0));
    let capture = FakeCapture {
        ready,
        stop: VoiceoverFinalize::Finalizing,
        cancel: VoiceoverFinalize::Finished(VoiceoverCaptureOutcome::Cancelled),
        status: VoiceoverFinalize::Finalizing,
        status_once: None,
        begin_calls: Arc::clone(&begin_calls),
        stop_calls: Arc::new(AtomicUsize::new(0)),
        cancel_calls: Arc::new(AtomicUsize::new(0)),
    };
    (capture, begin_calls)
}

fn attach_durable(owner: &mut VoiceoverTimelineOwner<FakeCapture>, project_dir: &std::path::Path) {
    let take = owner.active.as_ref().unwrap().take.clone();
    let VoiceoverTakePreparation::New(session) =
        VoiceoverTakeSession::prepare(project_dir, &take).unwrap()
    else {
        panic!("test take must receive a new private durable session");
    };
    owner.active.as_mut().unwrap().session = Some(*session);
}

#[test]
fn admission_refuses_stale_revision_non_audio_and_locked_track() {
    let project = project();
    let stale = admit(
        &project,
        "op_000005",
        request(VoiceoverCue::Playhead { at_ms: 20 }),
    );
    assert_eq!(stale.unwrap_err().code, error_codes::CONFLICT);

    let mut wrong = request(VoiceoverCue::Playhead { at_ms: 20 });
    wrong.audio_track = "v1".into();
    assert_eq!(
        admit(&project, "op_000004", wrong).unwrap_err().code,
        error_codes::INVALID_ARGS
    );

    let mut locked = project;
    locked.track_mut("a1t").unwrap().locked = true;
    assert_eq!(
        admit(
            &locked,
            "op_000004",
            request(VoiceoverCue::Playhead { at_ms: 20 })
        )
        .unwrap_err()
        .code,
        error_codes::GUARDRAIL
    );
}

#[test]
fn retry_status_returns_only_for_the_exact_live_admission() {
    let mut owner = VoiceoverTimelineOwner::with_countdown(Duration::ZERO);
    owner
        .accept(
            &project(),
            "op_000004",
            request(VoiceoverCue::Playhead { at_ms: 20 }),
            capture(false).0,
        )
        .unwrap();

    let claim = owner.owner_claim().unwrap();
    assert!(owner
        .retry_status(
            &request(VoiceoverCue::Playhead { at_ms: 20 }),
            &Actor::system(),
            Some(&claim.session_id),
        )
        .unwrap()
        .is_some());
    assert_eq!(
        owner
            .retry_status(
                &request(VoiceoverCue::Playhead { at_ms: 21 }),
                &Actor::system(),
                None,
            )
            .unwrap_err()
            .code,
        error_codes::CONFLICT
    );
    assert_eq!(
        owner
            .retry_status(
                &request(VoiceoverCue::Playhead { at_ms: 20 }),
                &Actor::system(),
                Some("stale-owner-session-id"),
            )
            .unwrap_err()
            .code,
        error_codes::CONFLICT
    );
}

#[test]
fn in_out_stops_only_from_observed_program_playhead_after_countdown() {
    let mut owner = VoiceoverTimelineOwner::with_countdown(Duration::ZERO);
    let (capture, begin_calls) = capture(true);
    let accepted = owner
        .accept(
            &project(),
            "op_000004",
            request(VoiceoverCue::InOut {
                in_ms: 120,
                out_ms: 320,
            }),
            capture,
        )
        .unwrap();
    assert!(matches!(
        accepted,
        VoiceoverTimelineStatus::AwaitingMicrophone { .. }
    ));
    assert!(matches!(
        owner.status().unwrap(),
        VoiceoverTimelineStatus::StartProgramPlayback { .. }
    ));
    let request_fingerprint =
        super::fingerprint::for_accepted(&owner.active.as_ref().unwrap().take);
    assert_eq!(begin_calls.load(Ordering::Relaxed), 0);
    assert!(matches!(
        owner
            .playback_started("voiceover-request-1", &request_fingerprint, 0)
            .unwrap(),
        VoiceoverTimelineStatus::Recording { .. }
    ));
    assert_eq!(begin_calls.load(Ordering::Relaxed), 1);
    assert!(matches!(
        owner.observe_program_playhead(319).unwrap(),
        VoiceoverTimelineStatus::Recording { .. }
    ));
    assert!(matches!(
        owner.observe_program_playhead(320).unwrap(),
        VoiceoverTimelineStatus::Finalizing { .. }
    ));
}

#[test]
fn cancellation_is_owned_without_creating_a_materialization() {
    let mut owner = VoiceoverTimelineOwner::with_countdown(Duration::ZERO);
    owner
        .accept(
            &project(),
            "op_000004",
            request(VoiceoverCue::Playhead { at_ms: 10 }),
            capture(true).0,
        )
        .unwrap();
    assert!(matches!(
        owner.cancel().unwrap(),
        VoiceoverTimelineStatus::Finished {
            outcome: VoiceoverCaptureOutcome::Cancelled,
            ..
        }
    ));
    assert_eq!(
        owner.materialization("op_000004").unwrap_err().code,
        error_codes::CONFLICT
    );
    owner.discard_unplaceable().unwrap();
    assert!(matches!(
        owner
            .accept(
                &project(),
                "op_000004",
                request(VoiceoverCue::Playhead { at_ms: 20 }),
                capture(true).0,
            )
            .unwrap(),
        VoiceoverTimelineStatus::AwaitingMicrophone { .. }
    ));
}

#[test]
fn sealed_artifact_keeps_exact_timing_and_refuses_a_stale_placement() {
    let artifact = VoiceoverArtifact {
        path: std::path::PathBuf::from("server-owned/voiceover.wav"),
        sha256: "a".repeat(64),
        bytes: 2_048,
        duration_ms: 640,
        sample_rate_hz: 48_000,
        channels: 1,
    };
    let mut owner = VoiceoverTimelineOwner::with_countdown(Duration::ZERO);
    owner
        .accept(
            &project(),
            "op_000004",
            request(VoiceoverCue::Playhead { at_ms: 10 }),
            FakeCapture {
                ready: true,
                stop: VoiceoverFinalize::Finished(VoiceoverCaptureOutcome::Saved(artifact.clone())),
                cancel: VoiceoverFinalize::Finished(VoiceoverCaptureOutcome::Cancelled),
                status: VoiceoverFinalize::Finalizing,
                status_once: None,
                begin_calls: Arc::new(AtomicUsize::new(0)),
                stop_calls: Arc::new(AtomicUsize::new(0)),
                cancel_calls: Arc::new(AtomicUsize::new(0)),
            },
        )
        .unwrap();
    assert!(matches!(
        owner.stop().unwrap(),
        VoiceoverTimelineStatus::Finished { .. }
    ));
    let materialization = owner.materialization("op_000004").unwrap();
    assert_eq!(materialization.take.start_ms, 10);
    assert_eq!(materialization.artifact, artifact);
    assert_eq!(
        owner.materialization("op_000005").unwrap_err().code,
        error_codes::CONFLICT
    );
}

#[test]
fn native_device_loss_finishes_before_countdown_or_playback_can_be_claimed() {
    let mut owner = VoiceoverTimelineOwner::with_countdown(Duration::ZERO);
    owner
        .accept(
            &project(),
            "op_000004",
            request(VoiceoverCue::Playhead { at_ms: 10 }),
            FakeCapture {
                ready: false,
                stop: VoiceoverFinalize::Finalizing,
                cancel: VoiceoverFinalize::Finished(VoiceoverCaptureOutcome::Cancelled),
                status: VoiceoverFinalize::Finished(VoiceoverCaptureOutcome::DeviceLostNoSamples),
                status_once: None,
                begin_calls: Arc::new(AtomicUsize::new(0)),
                stop_calls: Arc::new(AtomicUsize::new(0)),
                cancel_calls: Arc::new(AtomicUsize::new(0)),
            },
        )
        .unwrap();
    assert!(matches!(
        owner.status().unwrap(),
        VoiceoverTimelineStatus::Finished {
            outcome: VoiceoverCaptureOutcome::DeviceLostNoSamples,
            ..
        }
    ));
    assert_eq!(
        owner.materialization("op_000004").unwrap_err().code,
        error_codes::CONFLICT
    );
}

#[path = "voiceover_timeline_owner_durable_tests.rs"]
mod durable_tests;

#[path = "voiceover_timeline_owner_ownership_tests.rs"]
mod ownership_tests;

#[path = "voiceover_timeline_owner_retry_tests.rs"]
mod retry_tests;

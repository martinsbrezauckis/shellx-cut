use super::*;

#[test]
fn materialization_cannot_consume_a_finish_between_capture_polls_before_journal_seal() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("voiceover.cutproj");
    std::fs::create_dir(&project_dir).unwrap();
    let artifact = VoiceoverArtifact {
        path: project_dir.join("private.wav"),
        sha256: "b".repeat(64),
        bytes: 512,
        duration_ms: 160,
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
                ready: false,
                stop: VoiceoverFinalize::Finalizing,
                cancel: VoiceoverFinalize::Finished(VoiceoverCaptureOutcome::Cancelled),
                status: VoiceoverFinalize::Finished(VoiceoverCaptureOutcome::Saved(artifact)),
                status_once: Some(VoiceoverFinalize::Finalizing),
                begin_calls: Arc::new(AtomicUsize::new(0)),
                stop_calls: Arc::new(AtomicUsize::new(0)),
                cancel_calls: Arc::new(AtomicUsize::new(0)),
            },
        )
        .unwrap();
    let take = owner.active.as_ref().unwrap().take.clone();
    attach_durable(&mut owner, &project_dir);

    assert_eq!(
        owner.materialization("op_000004").unwrap_err().code,
        error_codes::CONFLICT
    );
    let root = CaptureRoot::for_project(&project_dir).unwrap();
    let journal = VoiceoverTakeJournalFile::open(&root, &take_id_for(&take)).unwrap();
    assert!(journal.state().sealed.is_none());
    assert!(journal.state().terminal.is_none());

    assert!(matches!(
        owner.status().unwrap(),
        VoiceoverTimelineStatus::Finished { .. }
    ));
    let journal = VoiceoverTakeJournalFile::open(&root, &take_id_for(&take)).unwrap();
    assert!(journal.state().sealed.is_some());
    assert_eq!(
        journal.state().terminal,
        Some(VoiceoverTerminalResult::Saved)
    );
    assert!(owner.materialization("op_000004").is_ok());
}

#[test]
fn durable_playback_ack_requires_the_latest_claimed_request_and_epoch() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("voiceover.cutproj");
    std::fs::create_dir(&project_dir).unwrap();
    let mut owner = VoiceoverTimelineOwner::with_countdown(Duration::ZERO);
    owner
        .accept(
            &project(),
            "op_000004",
            request(VoiceoverCue::Playhead { at_ms: 10 }),
            capture(true).0,
        )
        .unwrap();
    attach_durable(&mut owner, &project_dir);
    assert!(matches!(
        owner.status().unwrap(),
        VoiceoverTimelineStatus::StartProgramPlayback { .. }
    ));
    let first = owner.claim_preview_bridge().unwrap();
    let latest = owner.claim_preview_bridge().unwrap();
    assert!(owner
        .playback_started(
            &first.request_id,
            &first.request_fingerprint,
            first.bridge_epoch,
        )
        .is_err());
    assert!(owner
        .playback_started(
            "wrong-request",
            &latest.request_fingerprint,
            latest.bridge_epoch,
        )
        .is_err());
    assert!(owner
        .playback_started(
            &latest.request_id,
            &latest.request_fingerprint,
            latest.bridge_epoch + 1,
        )
        .is_err());
    assert!(owner
        .playback_started(&latest.request_id, "", latest.bridge_epoch)
        .is_err());
    assert!(owner
        .playback_started(&latest.request_id, &"a".repeat(64), latest.bridge_epoch)
        .is_err());
    assert!(owner
        .playback_started(&latest.request_id, &latest.request_fingerprint, 0)
        .is_err());
    assert!(matches!(
        owner
            .playback_started(
                &latest.request_id,
                &latest.request_fingerprint,
                latest.bridge_epoch,
            )
            .unwrap(),
        VoiceoverTimelineStatus::Recording { .. }
    ));
}

#[test]
fn delayed_ack_from_a_reused_request_id_cannot_cross_durable_take_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("voiceover.cutproj");
    std::fs::create_dir(&project_dir).unwrap();

    let mut first = VoiceoverTimelineOwner::with_countdown(Duration::ZERO);
    first
        .accept(
            &project(),
            "op_000004",
            request(VoiceoverCue::Playhead { at_ms: 10 }),
            capture(true).0,
        )
        .unwrap();
    attach_durable(&mut first, &project_dir);
    first.status().unwrap();
    let delayed = first.claim_preview_bridge().unwrap();

    let mut retry = VoiceoverTimelineOwner::with_countdown(Duration::ZERO);
    let mut reused_request = request(VoiceoverCue::Playhead { at_ms: 20 });
    reused_request.expected_revision = "op_000005".into();
    retry
        .accept(&project(), "op_000005", reused_request, capture(true).0)
        .unwrap();
    attach_durable(&mut retry, &project_dir);
    retry.status().unwrap();
    let latest = retry.claim_preview_bridge().unwrap();

    assert_eq!(delayed.request_id, latest.request_id);
    assert_eq!(delayed.bridge_epoch, latest.bridge_epoch);
    assert_ne!(delayed.request_fingerprint, latest.request_fingerprint);
    assert!(retry
        .playback_started(
            &delayed.request_id,
            &delayed.request_fingerprint,
            delayed.bridge_epoch,
        )
        .is_err());
    assert!(matches!(
        retry
            .playback_started(
                &latest.request_id,
                &latest.request_fingerprint,
                latest.bridge_epoch,
            )
            .unwrap(),
        VoiceoverTimelineStatus::Recording { .. }
    ));
}

#[test]
fn cancel_is_first_wins_and_never_executes_a_later_stop_or_out() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("voiceover.cutproj");
    std::fs::create_dir(&project_dir).unwrap();
    let stop_calls = Arc::new(AtomicUsize::new(0));
    let cancel_calls = Arc::new(AtomicUsize::new(0));
    let mut owner = VoiceoverTimelineOwner::with_countdown(Duration::ZERO);
    owner
        .accept(
            &project(),
            "op_000004",
            request(VoiceoverCue::InOut {
                in_ms: 10,
                out_ms: 20,
            }),
            FakeCapture {
                ready: false,
                stop: VoiceoverFinalize::Finalizing,
                cancel: VoiceoverFinalize::Finished(VoiceoverCaptureOutcome::Cancelled),
                status: VoiceoverFinalize::Finalizing,
                status_once: None,
                begin_calls: Arc::new(AtomicUsize::new(0)),
                stop_calls: Arc::clone(&stop_calls),
                cancel_calls: Arc::clone(&cancel_calls),
            },
        )
        .unwrap();
    attach_durable(&mut owner, &project_dir);
    owner.cancel().unwrap();
    owner.stop().unwrap();
    owner.observe_program_playhead(20).unwrap();
    assert_eq!(cancel_calls.load(Ordering::Relaxed), 1);
    assert_eq!(stop_calls.load(Ordering::Relaxed), 0);
}

fn saved_capture(artifact: VoiceoverArtifact) -> FakeCapture {
    FakeCapture {
        ready: true,
        stop: VoiceoverFinalize::Finished(VoiceoverCaptureOutcome::Saved(artifact)),
        cancel: VoiceoverFinalize::Finished(VoiceoverCaptureOutcome::Cancelled),
        status: VoiceoverFinalize::Finalizing,
        status_once: None,
        begin_calls: Arc::new(AtomicUsize::new(0)),
        stop_calls: Arc::new(AtomicUsize::new(0)),
        cancel_calls: Arc::new(AtomicUsize::new(0)),
    }
}

fn eligible_artifact() -> VoiceoverArtifact {
    VoiceoverArtifact {
        path: "private/voiceover.wav".into(),
        sha256: "c".repeat(64),
        bytes: 512,
        duration_ms: 160,
        sample_rate_hz: 48_000,
        channels: 1,
    }
}

#[test]
fn only_stop_or_out_with_a_sealed_saved_prefix_are_placement_eligible() {
    let mut stopped = VoiceoverTimelineOwner::with_countdown(Duration::ZERO);
    stopped
        .accept(
            &project(),
            "op_000004",
            request(VoiceoverCue::Playhead { at_ms: 10 }),
            saved_capture(eligible_artifact()),
        )
        .unwrap();
    stopped.stop().unwrap();
    assert!(stopped.materialization("op_000004").is_ok());

    let mut cancelled = VoiceoverTimelineOwner::with_countdown(Duration::ZERO);
    cancelled
        .accept(
            &project(),
            "op_000004",
            request(VoiceoverCue::Playhead { at_ms: 10 }),
            saved_capture(eligible_artifact()),
        )
        .unwrap();
    cancelled.cancel().unwrap();
    assert_eq!(
        cancelled.materialization("op_000004").unwrap_err().code,
        error_codes::CONFLICT
    );

    let mut out = VoiceoverTimelineOwner::with_countdown(Duration::ZERO);
    out.accept(
        &project(),
        "op_000004",
        request(VoiceoverCue::InOut {
            in_ms: 10,
            out_ms: 20,
        }),
        saved_capture(eligible_artifact()),
    )
    .unwrap();
    out.status().unwrap();
    let fingerprint = super::fingerprint::for_accepted(&out.active.as_ref().unwrap().take);
    out.playback_started("voiceover-request-1", &fingerprint, 0)
        .unwrap();
    out.observe_program_playhead(20).unwrap();
    assert!(out.materialization("op_000004").is_ok());
}

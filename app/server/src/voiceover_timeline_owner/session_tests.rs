use super::super::journal::{
    VoiceoverSealedArtifact, VoiceoverTakeJournalEntry, VoiceoverTakeJournalState,
    VoiceoverTakePhase, VoiceoverTerminalIntent, VoiceoverTerminalResult,
};
use super::super::journal_io::VoiceoverTakeJournalFile;
#[cfg(unix)]
use super::super::journal_io::VOICEOVER_TAKE_JOURNAL_FILE;
use super::super::session_recovery::artifact_facts;
use super::*;
use cut_core::error_codes;
use record_capture::{VoiceoverArtifact, VoiceoverCaptureOutcome};
use record_recovery::CaptureRoot;
use sha2::{Digest, Sha256};

fn take() -> AcceptedVoiceoverTimeline {
    AcceptedVoiceoverTimeline {
        request_id: "voiceover-request-1".into(),
        revision: "op_000004".into(),
        audio_track: "a1t".into(),
        start_ms: 120,
        out_ms: Some(320),
    }
}

fn project() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("voiceover.cutproj");
    std::fs::create_dir(&project).unwrap();
    (temp, project)
}

fn new_session(project: &std::path::Path) -> VoiceoverTakeSession {
    let VoiceoverTakePreparation::New(session) =
        VoiceoverTakeSession::prepare(project, &take()).unwrap()
    else {
        panic!("first durable take must own a fresh WAV leaf");
    };
    *session
}

fn artifact(path: &std::path::Path, bytes: &[u8]) -> VoiceoverArtifact {
    std::fs::write(path, bytes).unwrap();
    let mut digest = Sha256::new();
    digest.update(bytes);
    VoiceoverArtifact {
        path: path.into(),
        sha256: format!("{:x}", digest.finalize()),
        bytes: bytes.len() as u64,
        duration_ms: 120,
        sample_rate_hz: 48_000,
        channels: 1,
    }
}

#[test]
fn journal_append_failure_leaves_the_cloned_phase_unchanged() {
    let (_temp, project) = project();
    let mut session = new_session(&project);
    session.fail_next_append_for_test();
    assert!(session.record_phase(VoiceoverTakePhase::Countdown).is_err());
    assert_eq!(
        session.journal.state().phase,
        VoiceoverTakePhase::AwaitingMicrophone
    );
    session.record_phase(VoiceoverTakePhase::Countdown).unwrap();
}

#[test]
fn journal_requires_finalizing_and_an_exact_sealed_terminal_match() {
    let mut state = VoiceoverTakeJournalState::new(intent_for(&take()).unwrap()).unwrap();
    let prefix = VoiceoverSealedArtifact {
        sha256: "c".repeat(64),
        bytes: 512,
        duration_ms: 160,
        sample_rate_hz: 48_000,
        channels: 1,
        device_lost_after_prefix: true,
    };
    assert!(state
        .apply(VoiceoverTakeJournalEntry::Sealed {
            artifact: prefix.clone(),
        })
        .is_err());
    state
        .apply(VoiceoverTakeJournalEntry::Phase {
            phase: VoiceoverTakePhase::Finalizing,
            bridge_epoch: 0,
        })
        .unwrap();
    state
        .apply(VoiceoverTakeJournalEntry::Sealed { artifact: prefix })
        .unwrap();
    assert!(state
        .apply(VoiceoverTakeJournalEntry::Terminal {
            result: VoiceoverTerminalResult::Saved,
        })
        .is_err());
    state
        .apply(VoiceoverTakeJournalEntry::Terminal {
            result: VoiceoverTerminalResult::DeviceLostSavedPrefix,
        })
        .unwrap();
}

#[test]
fn exact_admitted_retry_reuses_one_directory_without_a_caller_digest() {
    let (_temp, project) = project();
    let session = new_session(&project);
    drop(session);

    assert!(matches!(
        VoiceoverTakeSession::prepare(&project, &take()).unwrap(),
        VoiceoverTakePreparation::Existing(VoiceoverRecoveredTake::Interrupted)
    ));

    let mut changed = take();
    changed.revision = "op_000005".into();
    assert_ne!(take_id_for(&take()), take_id_for(&changed));
    assert!(matches!(
        VoiceoverTakeSession::prepare(&project, &changed).unwrap(),
        VoiceoverTakePreparation::New(_)
    ));
}

#[test]
fn terminal_intent_is_first_wins_before_a_nonclaimable_terminal() {
    for intent in [
        VoiceoverTerminalIntent::Stop,
        VoiceoverTerminalIntent::Cancel,
        VoiceoverTerminalIntent::Out,
    ] {
        let (_temp, project) = project();
        let mut session = new_session(&project);
        if intent == VoiceoverTerminalIntent::Out {
            session.record_phase(VoiceoverTakePhase::Countdown).unwrap();
            session
                .record_phase(VoiceoverTakePhase::StartProgramPlayback)
                .unwrap();
            session.record_phase(VoiceoverTakePhase::Recording).unwrap();
        }
        assert!(session.record_terminal_intent(intent).unwrap());
        assert!(!session
            .record_terminal_intent(VoiceoverTerminalIntent::Cancel)
            .unwrap());
        session
            .record_phase(VoiceoverTakePhase::Finalizing)
            .unwrap();
        let result = if intent == VoiceoverTerminalIntent::Cancel {
            VoiceoverCaptureOutcome::Cancelled
        } else {
            VoiceoverCaptureOutcome::ZeroSamples
        };
        session.record_outcome(&result).unwrap();
        drop(session);
        assert!(matches!(
            VoiceoverTakeSession::prepare(&project, &take()).unwrap(),
            VoiceoverTakePreparation::Existing(VoiceoverRecoveredTake::Terminal(_))
        ));
    }
}

#[test]
fn only_hash_verified_sealed_wav_is_recovered_as_commit_eligible() {
    let (_temp, project) = project();
    let mut session = new_session(&project);
    let saved = artifact(session.wav_path(), b"sealed voiceover wav");
    assert!(session
        .record_outcome(&VoiceoverCaptureOutcome::Saved(saved.clone()))
        .is_err());
    session
        .record_phase(VoiceoverTakePhase::Finalizing)
        .unwrap();
    session
        .record_outcome(&VoiceoverCaptureOutcome::Saved(saved))
        .unwrap();
    drop(session);

    assert!(matches!(
        VoiceoverTakeSession::prepare(&project, &take()).unwrap(),
        VoiceoverTakePreparation::Existing(VoiceoverRecoveredTake::CommitEligible(_))
    ));

    let root = CaptureRoot::for_project(&project).unwrap();
    let path = root.capture_file(&take_id_for(&take()), WAV_LEAF).unwrap();
    std::fs::write(path, b"tampered").unwrap();
    assert_eq!(
        VoiceoverTakeSession::prepare(&project, &take())
            .unwrap_err()
            .code,
        error_codes::IO
    );
}

#[test]
fn sealed_crash_is_terminalized_before_it_becomes_commit_eligible() {
    let (_temp, project) = project();
    let mut session = new_session(&project);
    let saved = artifact(session.wav_path(), b"sealed but not terminal");
    session
        .record_phase(VoiceoverTakePhase::Finalizing)
        .unwrap();
    session
        .journal
        .append(VoiceoverTakeJournalEntry::Sealed {
            artifact: artifact_facts(&saved, false).unwrap(),
        })
        .unwrap();
    drop(session);

    assert!(matches!(
        VoiceoverTakeSession::prepare(&project, &take()).unwrap(),
        VoiceoverTakePreparation::Existing(VoiceoverRecoveredTake::CommitEligible(_))
    ));
    let root = CaptureRoot::for_project(&project).unwrap();
    let journal = VoiceoverTakeJournalFile::open(&root, &take_id_for(&take())).unwrap();
    assert_eq!(
        journal.state().terminal,
        Some(VoiceoverTerminalResult::Saved)
    );
}

#[test]
fn bridge_ack_must_echo_the_latest_persisted_request_and_epoch() {
    let (_temp, project) = project();
    let mut session = new_session(&project);
    session.record_phase(VoiceoverTakePhase::Countdown).unwrap();
    session
        .record_phase(VoiceoverTakePhase::StartProgramPlayback)
        .unwrap();
    let first = session.claim_preview_bridge(&take()).unwrap();
    let latest = session.claim_preview_bridge(&take()).unwrap();
    assert_eq!(first.request_id, take().request_id);
    assert_eq!(
        first.request_fingerprint,
        super::super::fingerprint::for_accepted(&take())
    );
    assert!(latest.bridge_epoch > first.bridge_epoch);
    assert!(session
        .validate_preview_ack(
            &first.request_id,
            &first.request_fingerprint,
            first.bridge_epoch,
        )
        .is_err());
    assert!(session
        .validate_preview_ack(
            "other-request",
            &latest.request_fingerprint,
            latest.bridge_epoch,
        )
        .is_err());
    assert!(session
        .validate_preview_ack(
            &latest.request_id,
            &latest.request_fingerprint,
            latest.bridge_epoch + 1,
        )
        .is_err());
    assert!(session
        .validate_preview_ack(&latest.request_id, "", latest.bridge_epoch)
        .is_err());
    assert!(session
        .validate_preview_ack(&latest.request_id, &"a".repeat(64), latest.bridge_epoch)
        .is_err());
    assert!(session
        .validate_preview_ack(&latest.request_id, &latest.request_fingerprint, 0)
        .is_err());
    session
        .validate_preview_ack(
            &latest.request_id,
            &latest.request_fingerprint,
            latest.bridge_epoch,
        )
        .unwrap();
}

#[cfg(unix)]
#[test]
fn reopen_refuses_a_replaced_journal_symlink_without_following_it() {
    use std::os::unix::fs::symlink;

    let (temp, project) = project();
    let session = new_session(&project);
    drop(session);
    let root = CaptureRoot::for_project(&project).unwrap();
    let journal = root
        .capture_file(&take_id_for(&take()), VOICEOVER_TAKE_JOURNAL_FILE)
        .unwrap();
    let outside = temp.path().join("outside-journal.jsonl");
    std::fs::write(&outside, b"outside sentinel\n").unwrap();
    std::fs::remove_file(&journal).unwrap();
    symlink(&outside, &journal).unwrap();

    assert_eq!(
        VoiceoverTakeSession::prepare(&project, &take())
            .unwrap_err()
            .code,
        error_codes::IO
    );
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside sentinel\n");
}

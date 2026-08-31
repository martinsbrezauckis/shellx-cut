//! Project-derived private ownership for one native voiceover take.

use std::path::{Path, PathBuf};

use cut_core::{error_codes, CutError};
use record_capture::VoiceoverCaptureOutcome;
use record_recovery::CaptureRoot;

use super::admission::AcceptedVoiceoverTimeline;
use super::fingerprint;
use super::journal::{
    VoiceoverSealedArtifact, VoiceoverTakeIntent, VoiceoverTakeJournalEntry,
    VoiceoverTakeJournalState, VoiceoverTakePhase, VoiceoverTerminalIntent,
    VoiceoverTerminalResult,
};
use super::journal_io::VoiceoverTakeJournalFile;
use super::session_recovery::{artifact_facts, io_error, recover, reject_existing_wav};

const TAKE_PREFIX: &str = "voiceover-";
pub(super) const WAV_LEAF: &str = "voiceover.wav";

/// A new session owns the only target leaf that native capture may create.
/// Existing session directories are recovery inputs, never a reason to reopen
/// an orphaned microphone device.
#[derive(Debug)]
pub(super) struct VoiceoverTakeSession {
    journal: VoiceoverTakeJournalFile,
    wav_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum VoiceoverRecoveredTake {
    CommitEligible(VoiceoverSealedArtifact),
    Interrupted,
    Terminal(VoiceoverTerminalResult),
}

#[derive(Debug)]
pub(super) enum VoiceoverTakePreparation {
    New(Box<VoiceoverTakeSession>),
    Existing(
        #[cfg_attr(
            not(test),
            allow(
                dead_code,
                reason = "production validates recovered state before refusing orphaned takes"
            )
        )]
        VoiceoverRecoveredTake,
    ),
}

/// Exact durable claim a Preview acknowledgement must echo before recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VoiceoverPreviewBridgeClaim {
    pub(crate) request_id: String,
    pub(crate) request_fingerprint: String,
    pub(crate) bridge_epoch: u64,
}

impl VoiceoverTakeSession {
    /// Persist immutable admitted request identity before a microphone is
    /// reserved or a WAV worker can exist.
    pub(super) fn prepare(
        project_dir: &Path,
        take: &AcceptedVoiceoverTimeline,
    ) -> Result<VoiceoverTakePreparation, CutError> {
        let intent = intent_for(take)?;
        let take_id = take_id(&intent.request_fingerprint);
        let root = CaptureRoot::for_project(project_dir)
            .map_err(|_| io_error("prepare the project-owned voiceover directory"))?;
        if root
            .existing_capture_dir(&take_id)
            .map_err(|_| io_error("inspect the project-owned voiceover directory"))?
            .is_some()
        {
            let mut journal = VoiceoverTakeJournalFile::open(&root, &take_id)?;
            if !journal.state().intent.same_request(&intent) {
                return Err(CutError::new(
                    error_codes::CONFLICT,
                    "voiceover request identity was reused with different input",
                    "retry with the original request payload or allocate a new request id",
                ));
            }
            return Ok(VoiceoverTakePreparation::Existing(recover(
                &mut journal,
                &root,
                &take_id,
            )?));
        }

        root.create_capture_dir(&take_id)
            .map_err(|_| io_error("reserve a project-owned voiceover directory"))?;
        let state = VoiceoverTakeJournalState::new(intent)?;
        let journal = VoiceoverTakeJournalFile::create_new(&root, &take_id, state)?;
        let wav_path = root
            .capture_file(&take_id, WAV_LEAF)
            .map_err(|_| io_error("derive the private voiceover WAV leaf"))?;
        reject_existing_wav(&wav_path)?;
        Ok(VoiceoverTakePreparation::New(Box::new(Self {
            journal,
            wav_path,
        })))
    }

    pub(super) fn wav_path(&self) -> &Path {
        &self.wav_path
    }

    pub(super) fn record_phase(&mut self, phase: VoiceoverTakePhase) -> Result<(), CutError> {
        let (current_phase, bridge_epoch) = {
            let state = self.journal.state();
            (state.phase, state.bridge_epoch)
        };
        if current_phase == phase {
            return Ok(());
        }
        self.journal.append(VoiceoverTakeJournalEntry::Phase {
            phase,
            bridge_epoch,
        })
    }

    /// Reserve the exact durable claim that Preview must acknowledge.
    pub(super) fn claim_preview_bridge(
        &mut self,
        take: &AcceptedVoiceoverTimeline,
    ) -> Result<VoiceoverPreviewBridgeClaim, CutError> {
        let (phase, bridge_epoch, request_id, request_fingerprint) = {
            let state = self.journal.state();
            if state_intent_differs(state, take) {
                return Err(CutError::new(
                    error_codes::CONFLICT,
                    "voiceover bridge claim does not match its admitted request",
                    "claim playback only for the exact persisted take identity",
                ));
            }
            (
                state.phase,
                state.bridge_epoch,
                state.intent.request_id.clone(),
                state.intent.request_fingerprint.clone(),
            )
        };
        if phase != VoiceoverTakePhase::StartProgramPlayback {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "voiceover bridge claim is not awaiting program playback",
                "claim a Preview acknowledgement only for the persisted start-playback phase",
            ));
        }
        let next = bridge_epoch
            .checked_add(1)
            .ok_or_else(|| io_error("advance the voiceover bridge epoch"))?;
        self.journal.append(VoiceoverTakeJournalEntry::Phase {
            phase,
            bridge_epoch: next,
        })?;
        Ok(VoiceoverPreviewBridgeClaim {
            request_id,
            request_fingerprint,
            bridge_epoch: next,
        })
    }

    pub(super) fn validate_preview_ack(
        &self,
        request_id: &str,
        request_fingerprint: &str,
        bridge_epoch: u64,
    ) -> Result<(), CutError> {
        let state = self.journal.state();
        if bridge_epoch != 0
            && state.phase == VoiceoverTakePhase::StartProgramPlayback
            && state.intent.request_id == request_id
            && state.intent.request_fingerprint == request_fingerprint
            && state.bridge_epoch == bridge_epoch
        {
            return Ok(());
        }
        Err(CutError::new(
            error_codes::CONFLICT,
            "voiceover playback acknowledgement is stale or mismatched",
            "acknowledge the exact latest durable request, fingerprint, and bridge epoch",
        ))
    }

    /// The later Out observation is valid only for the exact bridge epoch
    /// whose acknowledgement moved this durable take into Recording. The
    /// capability itself remains in the volatile owner and is intentionally
    /// absent from this journal.
    pub(super) fn validate_preview_observation(
        &self,
        request_id: &str,
        request_fingerprint: &str,
        bridge_epoch: u64,
    ) -> Result<(), CutError> {
        let state = self.journal.state();
        if state.phase == VoiceoverTakePhase::Recording
            && state.intent.request_id == request_id
            && state.intent.request_fingerprint == request_fingerprint
            && state.bridge_epoch == bridge_epoch
            && bridge_epoch > 0
        {
            return Ok(());
        }
        Err(CutError::new(
            error_codes::CONFLICT,
            "voiceover Out bridge claim is stale or mismatched",
            "report Out only for the exact latest Preview request, fingerprint, and epoch",
        ))
    }

    pub(super) fn record_terminal_intent(
        &mut self,
        intent: VoiceoverTerminalIntent,
    ) -> Result<bool, CutError> {
        let state = self.journal.state();
        if state.terminal.is_some() || state.terminal_intent.is_some() {
            return Ok(false);
        }
        self.journal
            .append(VoiceoverTakeJournalEntry::TerminalIntent { intent })?;
        Ok(true)
    }

    pub(super) fn record_outcome(
        &mut self,
        outcome: &VoiceoverCaptureOutcome,
    ) -> Result<(), CutError> {
        let (artifact, result) = match outcome {
            VoiceoverCaptureOutcome::Saved(artifact) => {
                (Some(artifact), VoiceoverTerminalResult::Saved)
            }
            VoiceoverCaptureOutcome::DeviceLostSavedPrefix(artifact) => (
                Some(artifact),
                VoiceoverTerminalResult::DeviceLostSavedPrefix,
            ),
            VoiceoverCaptureOutcome::DeviceLostNoSamples => {
                (None, VoiceoverTerminalResult::DeviceLostNoSamples)
            }
            VoiceoverCaptureOutcome::ZeroSamples => (None, VoiceoverTerminalResult::ZeroSamples),
            VoiceoverCaptureOutcome::Cancelled => (None, VoiceoverTerminalResult::Cancelled),
            VoiceoverCaptureOutcome::CancelCleanupFailed(_) => {
                (None, VoiceoverTerminalResult::CancelCleanupFailed)
            }
            VoiceoverCaptureOutcome::Failed(_) => (None, VoiceoverTerminalResult::Failed),
        };
        let state = self.journal.state();
        if state.terminal == Some(result) {
            return Ok(());
        }
        if state.phase != VoiceoverTakePhase::Finalizing {
            return Err(io_error(
                "record a voiceover terminal only after a durable finalizing phase",
            ));
        }
        if let Some(artifact) = artifact {
            let facts = artifact_facts(
                artifact,
                result == VoiceoverTerminalResult::DeviceLostSavedPrefix,
            )?;
            let sealed = self.journal.state().sealed.clone();
            match &sealed {
                Some(existing) if existing == &facts => {}
                Some(_) => {
                    return Err(io_error(
                        "voiceover outcome does not match its already-sealed artifact",
                    ))
                }
                None => self
                    .journal
                    .append(VoiceoverTakeJournalEntry::Sealed { artifact: facts })?,
            }
        }
        self.journal
            .append(VoiceoverTakeJournalEntry::Terminal { result })
    }

    /// Durable owner path for a native finish. A retry after terminal sync may
    /// only compare the already-recorded result; it must never regress phase.
    pub(super) fn record_finished_outcome(
        &mut self,
        outcome: &VoiceoverCaptureOutcome,
    ) -> Result<(), CutError> {
        if self.journal.state().terminal.is_none() {
            self.record_phase(VoiceoverTakePhase::Finalizing)?;
        }
        self.record_outcome(outcome)
    }

    pub(super) fn record_start_failure(&mut self) -> Result<(), CutError> {
        self.record_phase(VoiceoverTakePhase::Finalizing)?;
        self.journal.append(VoiceoverTakeJournalEntry::Terminal {
            result: VoiceoverTerminalResult::StartFailed,
        })
    }

    #[cfg(test)]
    pub(super) fn fail_next_append_for_test(&mut self) {
        self.journal.fail_next_append_for_test();
    }
}

#[cfg(test)]
pub(super) fn take_id_for(take: &AcceptedVoiceoverTimeline) -> String {
    take_id(&fingerprint::for_accepted(take))
}

fn intent_for(take: &AcceptedVoiceoverTimeline) -> Result<VoiceoverTakeIntent, CutError> {
    let intent = VoiceoverTakeIntent {
        schema: super::journal::VOICEOVER_TAKE_JOURNAL_SCHEMA.into(),
        request_id: take.request_id.clone(),
        request_fingerprint: fingerprint::for_accepted(take),
        accepted_revision: take.revision.clone(),
        audio_track: take.audio_track.clone(),
        start_ms: take.start_ms,
        out_ms: take.out_ms,
        phase: VoiceoverTakePhase::AwaitingMicrophone,
        bridge_epoch: 0,
    };
    intent.validate()?;
    Ok(intent)
}

fn state_intent_differs(
    state: &VoiceoverTakeJournalState,
    take: &AcceptedVoiceoverTimeline,
) -> bool {
    state.intent.request_id != take.request_id
        || state.intent.accepted_revision != take.revision
        || state.intent.audio_track != take.audio_track
        || state.intent.start_ms != take.start_ms
        || state.intent.out_ms != take.out_ms
}

fn take_id(request_fingerprint: &str) -> String {
    format!("{TAKE_PREFIX}{request_fingerprint}")
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

//! Private, replay-validated lifecycle facts for one voiceover take.
//!
//! This is intentionally not a public API contract.  In particular, the fixed
//! WAV leaf is derived by the session owner and never appears in these facts.

use cut_core::{error_codes, CutError};
use serde::{Deserialize, Serialize};

pub(super) const VOICEOVER_TAKE_JOURNAL_SCHEMA: &str = "shellx-cut/voiceover-take-journal/1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct VoiceoverTakeIntent {
    pub(super) schema: String,
    pub(super) request_id: String,
    pub(super) request_fingerprint: String,
    pub(super) accepted_revision: String,
    pub(super) audio_track: String,
    pub(super) start_ms: u64,
    pub(super) out_ms: Option<u64>,
    pub(super) phase: VoiceoverTakePhase,
    pub(super) bridge_epoch: u64,
}

impl VoiceoverTakeIntent {
    pub(super) fn validate(&self) -> Result<(), CutError> {
        if self.schema != VOICEOVER_TAKE_JOURNAL_SCHEMA
            || self.request_id.is_empty()
            || self.request_id.len() > 128
            || self.accepted_revision.is_empty()
            || self.audio_track.is_empty()
            || !is_lower_hex_sha256(&self.request_fingerprint)
            || self.out_ms.is_some_and(|out_ms| out_ms <= self.start_ms)
            || self.phase != VoiceoverTakePhase::AwaitingMicrophone
            || self.bridge_epoch != 0
        {
            return Err(invalid("voiceover take intent is malformed"));
        }
        Ok(())
    }

    pub(super) fn same_request(&self, other: &Self) -> bool {
        self == other
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum VoiceoverTakePhase {
    AwaitingMicrophone,
    Countdown,
    StartProgramPlayback,
    Recording,
    Finalizing,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum VoiceoverTerminalIntent {
    Stop,
    Cancel,
    Out,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct VoiceoverSealedArtifact {
    pub(super) sha256: String,
    pub(super) bytes: u64,
    pub(super) duration_ms: u64,
    pub(super) sample_rate_hz: u32,
    pub(super) channels: u16,
    pub(super) device_lost_after_prefix: bool,
}

impl VoiceoverSealedArtifact {
    pub(super) fn validate(&self) -> Result<(), CutError> {
        if !is_lower_hex_sha256(&self.sha256)
            || self.bytes == 0
            || self.duration_ms == 0
            || self.sample_rate_hz == 0
            || self.channels == 0
        {
            return Err(invalid("voiceover sealed artifact facts are malformed"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum VoiceoverTerminalResult {
    Saved,
    DeviceLostSavedPrefix,
    DeviceLostNoSamples,
    ZeroSamples,
    Cancelled,
    CancelCleanupFailed,
    Failed,
    StartFailed,
    Interrupted,
}

impl VoiceoverTerminalResult {
    fn requires_sealed_artifact(self) -> bool {
        matches!(self, Self::Saved | Self::DeviceLostSavedPrefix)
    }

    fn matches_sealed_artifact(self, artifact: &VoiceoverSealedArtifact) -> bool {
        match self {
            Self::Saved => !artifact.device_lost_after_prefix,
            Self::DeviceLostSavedPrefix => artifact.device_lost_after_prefix,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "entry", rename_all = "snake_case")]
pub(super) enum VoiceoverTakeJournalEntry {
    Intent {
        intent: VoiceoverTakeIntent,
    },
    Phase {
        phase: VoiceoverTakePhase,
        bridge_epoch: u64,
    },
    TerminalIntent {
        intent: VoiceoverTerminalIntent,
    },
    Sealed {
        artifact: VoiceoverSealedArtifact,
    },
    Terminal {
        result: VoiceoverTerminalResult,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VoiceoverTakeJournalState {
    pub(super) intent: VoiceoverTakeIntent,
    pub(super) phase: VoiceoverTakePhase,
    pub(super) bridge_epoch: u64,
    pub(super) terminal_intent: Option<VoiceoverTerminalIntent>,
    pub(super) sealed: Option<VoiceoverSealedArtifact>,
    pub(super) terminal: Option<VoiceoverTerminalResult>,
}

impl VoiceoverTakeJournalState {
    pub(super) fn new(intent: VoiceoverTakeIntent) -> Result<Self, CutError> {
        intent.validate()?;
        let phase = intent.phase;
        let bridge_epoch = intent.bridge_epoch;
        Ok(Self {
            intent,
            phase,
            bridge_epoch,
            terminal_intent: None,
            sealed: None,
            terminal: None,
        })
    }

    pub(super) fn replay(entries: Vec<VoiceoverTakeJournalEntry>) -> Result<Self, CutError> {
        let Some(VoiceoverTakeJournalEntry::Intent { intent }) = entries.first() else {
            return Err(invalid(
                "voiceover take journal is missing its immutable intent",
            ));
        };
        let mut state = Self::new(intent.clone())?;
        for entry in entries.into_iter().skip(1) {
            state.apply(entry)?;
        }
        Ok(state)
    }

    pub(super) fn apply(&mut self, entry: VoiceoverTakeJournalEntry) -> Result<(), CutError> {
        match entry {
            VoiceoverTakeJournalEntry::Intent { .. } => {
                Err(invalid("voiceover take journal has a duplicate intent"))
            }
            VoiceoverTakeJournalEntry::Phase {
                phase,
                bridge_epoch,
            } => self.apply_phase(phase, bridge_epoch),
            VoiceoverTakeJournalEntry::TerminalIntent { intent } => {
                if self.terminal.is_some() || self.terminal_intent.is_some() {
                    return Err(invalid("voiceover take has multiple terminal intents"));
                }
                if intent == VoiceoverTerminalIntent::Out
                    && self.phase != VoiceoverTakePhase::Recording
                {
                    return Err(invalid("voiceover Out was not observed while recording"));
                }
                self.terminal_intent = Some(intent);
                Ok(())
            }
            VoiceoverTakeJournalEntry::Sealed { artifact } => {
                artifact.validate()?;
                if self.phase != VoiceoverTakePhase::Finalizing
                    || self.terminal.is_some()
                    || self.sealed.is_some()
                {
                    return Err(invalid("voiceover take has multiple sealed artifacts"));
                }
                self.sealed = Some(artifact);
                Ok(())
            }
            VoiceoverTakeJournalEntry::Terminal { result } => {
                let sealed_matches = self
                    .sealed
                    .as_ref()
                    .is_some_and(|artifact| result.matches_sealed_artifact(artifact));
                if self.phase != VoiceoverTakePhase::Finalizing
                    || self.terminal.is_some()
                    || (result.requires_sealed_artifact() && !sealed_matches)
                    || (!result.requires_sealed_artifact() && self.sealed.is_some())
                {
                    return Err(invalid(
                        "voiceover take terminal result does not match its artifact",
                    ));
                }
                self.phase = VoiceoverTakePhase::Finished;
                self.terminal = Some(result);
                Ok(())
            }
        }
    }

    fn apply_phase(
        &mut self,
        phase: VoiceoverTakePhase,
        bridge_epoch: u64,
    ) -> Result<(), CutError> {
        if self.terminal.is_some() || bridge_epoch < self.bridge_epoch {
            return Err(invalid(
                "voiceover take phase regressed after a durable transition",
            ));
        }
        if phase == self.phase {
            if bridge_epoch == self.bridge_epoch || bridge_epoch == self.bridge_epoch + 1 {
                self.bridge_epoch = bridge_epoch;
                return Ok(());
            }
            return Err(invalid("voiceover bridge epoch skipped a durable claim"));
        }
        if bridge_epoch != self.bridge_epoch || !allowed_transition(self.phase, phase) {
            return Err(invalid("voiceover take phase transition is invalid"));
        }
        self.phase = phase;
        Ok(())
    }
}

fn allowed_transition(from: VoiceoverTakePhase, to: VoiceoverTakePhase) -> bool {
    matches!(
        (from, to),
        (
            VoiceoverTakePhase::AwaitingMicrophone,
            VoiceoverTakePhase::Countdown
        ) | (
            VoiceoverTakePhase::Countdown,
            VoiceoverTakePhase::StartProgramPlayback
        ) | (
            VoiceoverTakePhase::StartProgramPlayback,
            VoiceoverTakePhase::Recording
        ) | (
            VoiceoverTakePhase::AwaitingMicrophone
                | VoiceoverTakePhase::Countdown
                | VoiceoverTakePhase::StartProgramPlayback
                | VoiceoverTakePhase::Recording,
            VoiceoverTakePhase::Finalizing
        )
    )
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn invalid(detail: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::CONFLICT,
        "voiceover take journal is invalid",
        detail.into(),
    )
}

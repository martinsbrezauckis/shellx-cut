//! Immutable per-run input evidence for native pause owners.
//!
//! The fixed name comes from a journal-pinned run sequence. No recovery path
//! enumerates this directory or reconstructs selection facts from a project.

use super::run_seal_coordinator::SealedRunEvidence;
use super::windows_pause_evidence::{RecordingAudioDraft, RecordingInputDraft};
use record_recovery::{
    CaptureRoot, DurableStateTransition, RecordingInputSidecarPin, RecordingProjectBinding,
    RecordingStream, SealedRun,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod project;
mod validation;
mod wav_validation;
#[cfg(any(windows, target_os = "macos", test))]
pub(crate) use project::admit_project_binding;
pub(crate) use project::verify_project_binding;
pub(crate) use validation::{
    pins_digest, verified_audio_sources, verify_pinned_inputs, verify_recovery_capture,
    VerifiedInputAudio,
};

const SCHEMA: &str = "shellx-cut/private-recording-input/2";
pub(crate) struct WindowsPauseInputSidecarOwner {
    root: CaptureRoot,
    capture_id: String,
    project: RecordingProjectBinding,
}

impl WindowsPauseInputSidecarOwner {
    pub(crate) fn new(
        root: CaptureRoot,
        capture_id: String,
        project: RecordingProjectBinding,
    ) -> Self {
        Self {
            root,
            capture_id,
            project,
        }
    }

    pub(crate) fn publish_or_reopen(
        &self,
        evidence: &SealedRunEvidence,
        readiness: &DurableStateTransition,
    ) -> Result<RecordingInputSidecarPin, String> {
        let draft = evidence
            .recording_input()
            .ok_or_else(|| "sealed native run lacks accepted input facts".to_string())?;
        let sidecar = InputSidecar::new(
            &self.capture_id,
            &self.project,
            evidence.run(),
            draft,
            readiness,
        )?;
        let bytes = canonical(&sidecar)?;
        let pin = RecordingInputSidecarPin {
            run_sequence: evidence.run().sequence,
            file_name: file_name(evidence.run().sequence),
            sha256: digest(&bytes),
            ready_transition_sequence: readiness.sequence,
            ready_unix_ms: readiness.observed_unix_ms,
            native_ready_unix_ms: draft.native_ready_unix_ms(),
            native_ready_raw_ms: draft.native_ready_raw_ms(),
            raw_start_ms: draft.raw_start_ms(),
            raw_end_ms: draft.raw_end_ms(),
        };
        match self
            .root
            .publish_new_capture_file(&self.capture_id, &pin.file_name, &bytes)
        {
            Ok(_) => {}
            Err(_) => {
                validation::existing_matches(&self.root, &self.capture_id, &pin, &bytes)?;
            }
        }
        validation::verify_pinned_run(
            &self.root,
            &self.capture_id,
            evidence.run(),
            readiness,
            &pin,
        )?;
        Ok(pin)
    }
}

pub(super) fn file_name(sequence: u64) -> String {
    format!("recording-input-run-{sequence:06}.json")
}

pub(super) fn canonical<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, String> {
    serde_json::to_vec(value).map_err(|_| "canonicalize recording-input sidecar".to_string())
}

pub(super) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct InputSidecar {
    body: InputSidecarBody,
    fingerprint_sha256: String,
}

impl InputSidecar {
    fn new(
        capture_id: &str,
        project: &RecordingProjectBinding,
        run: &SealedRun,
        draft: &RecordingInputDraft,
        readiness: &DurableStateTransition,
    ) -> Result<Self, String> {
        let clocks = InputClocks::from_draft(draft, run, readiness)?;
        let body = InputSidecarBody {
            schema: SCHEMA.into(),
            capture_id: capture_id.into(),
            project: project.clone(),
            run: run.clone(),
            selection: InputSelection::from_draft(draft, &project.accepted_revision)?,
            audio: InputAudioSource::from_drafts(
                draft.audio(),
                run,
                clocks.raw_start_ms,
                clocks.raw_end_ms,
                clocks.native_ready_unix_ms,
            )?,
            clocks,
        };
        Ok(Self {
            fingerprint_sha256: digest(&canonical(&body)?),
            body,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct InputSidecarBody {
    schema: String,
    capture_id: String,
    project: RecordingProjectBinding,
    run: SealedRun,
    selection: InputSelection,
    clocks: InputClocks,
    /// Every selected native audio owner contributes a closed, hash-pinned
    /// leaf here. Logical times come solely from the accepted sealed run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    audio: Vec<InputAudioSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct InputAudioSource {
    #[serde(rename = "type")]
    source_type: String,
    source_generation: u64,
    artifact: String,
    bytes: u64,
    sha256: String,
    media_duration_ms: u64,
    native_ready_unix_ms: u64,
    native_ready_raw_ms: u64,
    raw_start_ms: u64,
    raw_end_ms: u64,
    logical_start_ms: u64,
    logical_end_ms: u64,
}

impl InputAudioSource {
    fn from_drafts(
        drafts: &[RecordingAudioDraft],
        run: &SealedRun,
        expected_raw_start_ms: u64,
        expected_raw_end_ms: u64,
        expected_native_ready_unix_ms: u64,
    ) -> Result<Vec<Self>, String> {
        let mut sources = drafts
            .iter()
            .map(|draft| {
                let source_type = match draft.stream() {
                    RecordingStream::MicrophoneAudio => "microphone_audio",
                    RecordingStream::SystemAudio => "system_audio",
                    _ => {
                        return Err("recording-input sidecar has an unsupported audio source".into())
                    }
                };
                Ok(Self {
                    source_type: source_type.into(),
                    source_generation: draft.source_generation(),
                    artifact: draft.artifact().into(),
                    bytes: draft.bytes(),
                    sha256: draft.sha256().into(),
                    media_duration_ms: draft.media_duration_ms(),
                    native_ready_unix_ms: draft.native_ready_unix_ms(),
                    native_ready_raw_ms: draft.native_ready_raw_ms(),
                    raw_start_ms: draft.raw_start_ms(),
                    raw_end_ms: draft.raw_end_ms(),
                    logical_start_ms: run.logical_start_ms,
                    logical_end_ms: run.logical_end_ms,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        sources.sort_by_key(|source| source.stream());
        if sources
            .windows(2)
            .any(|pair| pair[0].stream() == pair[1].stream())
            || !Self::exactly_covers_run(&sources, run)
            || sources.iter().any(|source| {
                !source.matches_run(run)
                    || source.raw_start_ms != expected_raw_start_ms
                    || source.raw_end_ms != expected_raw_end_ms
                    || Some(source.native_ready_unix_ms)
                        != expected_native_ready_unix_ms
                            .checked_add(source.native_ready_raw_ms - source.raw_start_ms)
            })
        {
            return Err("recording-input sidecar audio sources are invalid".into());
        }
        Ok(sources)
    }

    fn exactly_covers_run(sources: &[Self], run: &SealedRun) -> bool {
        let expected = run
            .fragments
            .iter()
            .filter_map(|fragment| {
                matches!(
                    fragment.stream,
                    RecordingStream::MicrophoneAudio | RecordingStream::SystemAudio
                )
                .then_some(fragment.stream)
            })
            .collect::<Vec<_>>();
        expected.windows(2).all(|pair| pair[0] < pair[1])
            && sources.iter().map(Self::stream).eq(expected)
    }

    fn stream(&self) -> RecordingStream {
        match self.source_type.as_str() {
            "microphone_audio" => RecordingStream::MicrophoneAudio,
            "system_audio" => RecordingStream::SystemAudio,
            _ => RecordingStream::CameraVideo,
        }
    }

    fn valid_for_run(&self, run: &SealedRun, clocks: &InputClocks) -> bool {
        matches!(
            self.source_type.as_str(),
            "microphone_audio" | "system_audio"
        ) && self.matches_run(run)
            && self.raw_start_ms == clocks.raw_start_ms
            && self.raw_end_ms == clocks.raw_end_ms
            && Some(self.native_ready_unix_ms)
                == clocks
                    .native_ready_unix_ms
                    .checked_add(self.native_ready_raw_ms - self.raw_start_ms)
    }

    fn expected_artifact(&self) -> Option<String> {
        let prefix = match self.source_type.as_str() {
            "microphone_audio" => "recording-microphone-generation-",
            "system_audio" => "recording-system-generation-",
            _ => return None,
        };
        Some(format!("{prefix}{:020}.wav", self.source_generation))
    }

    fn matches_run(&self, run: &SealedRun) -> bool {
        let Some(fragment) = run.fragments.iter().find(|fragment| {
            fragment.stream == self.stream()
                && fragment.checkpoint_sequence.is_none()
                && fragment.stream_sequence == 0
        }) else {
            return false;
        };
        self.source_generation != 0
            && !self.artifact.is_empty()
            && self.bytes != 0
            && self.sha256.len() == 64
            && self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            && self.media_duration_ms != 0
            && self.native_ready_unix_ms != 0
            && self.raw_end_ms > self.raw_start_ms
            && self.native_ready_raw_ms >= self.raw_start_ms
            && self.native_ready_raw_ms <= self.raw_end_ms
            && self.logical_start_ms == run.logical_start_ms
            && self.logical_end_ms == run.logical_end_ms
            && self.logical_end_ms > self.logical_start_ms
            && self.raw_end_ms - self.raw_start_ms == self.logical_end_ms - self.logical_start_ms
            && self.media_duration_ms <= self.raw_end_ms - self.raw_start_ms
            && fragment.artifact == self.artifact
            && fragment.bytes == self.bytes
            && fragment.sha256 == self.sha256
            && fragment.facts.start_offset_ms == 0
            && fragment.facts.end_offset_ms == self.logical_end_ms - self.logical_start_ms
            && fragment.facts.media_duration_ms == self.media_duration_ms
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct InputSelection {
    accepted_revision: String,
    source: String,
    display_id: String,
    window_id: Option<String>,
    region: Option<Rect>,
    display: Rect,
    output_width: u32,
    output_height: u32,
    scale_x: Ratio,
    scale_y: Ratio,
    crop: Rect,
    fps: u32,
}

impl InputSelection {
    fn from_draft(draft: &RecordingInputDraft, accepted_revision: &str) -> Result<Self, String> {
        if draft.display_id().is_empty()
            || draft.display_width() == 0
            || draft.display_height() == 0
            || draft.output_width() != draft.display_width()
            || draft.output_height() != draft.display_height()
            || draft.fps() == 0
            || draft.raw_end_ms() <= draft.raw_start_ms()
        {
            return Err("accepted Windows recording input facts are invalid".into());
        }
        let display = Rect {
            x: draft.display_origin_x(),
            y: draft.display_origin_y(),
            width: draft.display_width(),
            height: draft.display_height(),
        };
        Ok(Self {
            accepted_revision: accepted_revision.into(),
            source: "screen_video".into(),
            display_id: draft.display_id().into(),
            window_id: None,
            region: None,
            display: display.clone(),
            output_width: draft.output_width(),
            output_height: draft.output_height(),
            scale_x: Ratio {
                numerator: 1,
                denominator: 1,
            },
            scale_y: Ratio {
                numerator: 1,
                denominator: 1,
            },
            crop: Rect {
                x: 0,
                y: 0,
                width: draft.display_width(),
                height: draft.display_height(),
            },
            fps: draft.fps(),
        })
    }

    fn valid(&self) -> bool {
        !self.accepted_revision.is_empty()
            && self.source == "screen_video"
            && !self.display_id.is_empty()
            && self.window_id.is_none()
            && self.region.is_none()
            && self.display.width > 0
            && self.display.height > 0
            && self.output_width == self.display.width
            && self.output_height == self.display.height
            && self.scale_x
                == Ratio {
                    numerator: 1,
                    denominator: 1,
                }
            && self.scale_y
                == Ratio {
                    numerator: 1,
                    denominator: 1,
                }
            && self.crop
                == Rect {
                    x: 0,
                    y: 0,
                    width: self.display.width,
                    height: self.display.height,
                }
            && self.fps > 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct InputClocks {
    ready_unix_ms: u64,
    native_ready_unix_ms: u64,
    ready_transition_sequence: u64,
    native_ready_raw_ms: u64,
    raw_start_ms: u64,
    raw_end_ms: u64,
    session_observed_start_ms: u64,
    session_observed_end_ms: u64,
    logical_start_ms: u64,
    logical_end_ms: u64,
}

impl InputClocks {
    fn from_draft(
        draft: &RecordingInputDraft,
        run: &SealedRun,
        readiness: &DurableStateTransition,
    ) -> Result<Self, String> {
        if !matches!(
            readiness.state,
            record_recovery::RecordingSessionState::Started
                | record_recovery::RecordingSessionState::Resumed
        ) || readiness.logical_offset_ms != run.logical_start_ms
            || draft.native_ready_raw_ms() != draft.raw_start_ms()
            || draft.raw_end_ms() <= draft.raw_start_ms()
            || run.logical_end_ms <= run.logical_start_ms
            || draft.raw_end_ms().checked_sub(draft.raw_start_ms())
                != run.logical_end_ms.checked_sub(run.logical_start_ms)
        {
            return Err("durable readiness transition does not admit this sidecar run".into());
        }
        Ok(Self {
            ready_unix_ms: readiness.observed_unix_ms,
            native_ready_unix_ms: draft.native_ready_unix_ms(),
            ready_transition_sequence: readiness.sequence,
            native_ready_raw_ms: draft.native_ready_raw_ms(),
            raw_start_ms: draft.raw_start_ms(),
            raw_end_ms: draft.raw_end_ms(),
            session_observed_start_ms: run.observed_start_ms,
            session_observed_end_ms: run.observed_end_ms,
            logical_start_ms: run.logical_start_ms,
            logical_end_ms: run.logical_end_ms,
        })
    }
    fn matches(
        &self,
        run: &SealedRun,
        readiness: &DurableStateTransition,
        pin: &RecordingInputSidecarPin,
    ) -> bool {
        // `ready_unix_ms` is the authoritative durable transition timestamp.
        // WGC's paired native Unix observation is an independently sampled raw
        // clock, so it is sealed by the same exact journal pin rather than
        // falsely equated to the coordinator's origin-derived durable value.
        self.ready_transition_sequence == readiness.sequence
            && self.ready_unix_ms == readiness.observed_unix_ms
            && self.ready_transition_sequence == pin.ready_transition_sequence
            && self.ready_unix_ms == pin.ready_unix_ms
            && self.native_ready_unix_ms == pin.native_ready_unix_ms
            && self.native_ready_raw_ms == pin.native_ready_raw_ms
            && self.raw_start_ms == pin.raw_start_ms
            && self.raw_end_ms == pin.raw_end_ms
            && self.native_ready_raw_ms == self.raw_start_ms
            && self.raw_end_ms > self.raw_start_ms
            && run.logical_end_ms > run.logical_start_ms
            && self.raw_end_ms - self.raw_start_ms == run.logical_end_ms - run.logical_start_ms
            && self.session_observed_start_ms == run.observed_start_ms
            && self.session_observed_end_ms == run.observed_end_ms
            && self.logical_start_ms == run.logical_start_ms
            && self.logical_end_ms == run.logical_end_ms
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Rect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Ratio {
    numerator: u32,
    denominator: u32,
}

#[cfg(test)]
#[path = "windows_pause_input_sidecar_tests.rs"]
mod tests;

//! Private microphone-only capture ownership for Timeline voiceover.
//! A server owner must bind a sealed take to one atomic timeline operation.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use record_core::{error_codes, RecordError, Result};
use sha2::{Digest, Sha256};

use crate::mic::{CapturedMicrophone, MicRecordingGate};
use crate::MicrophoneSource;

/// A sealed local WAV measured from Cut-written bytes; never a public path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceoverArtifact {
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
    pub duration_ms: u64,
    pub sample_rate_hz: u32,
    pub channels: u16,
}

/// The terminal outcome is deliberately more precise than a generic success.
/// A lost device may retain a real sealed prefix; zero samples never pretends a
/// WAV exists; cancellation only succeeds once its private artifact is gone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceoverCaptureOutcome {
    Saved(VoiceoverArtifact),
    DeviceLostSavedPrefix(VoiceoverArtifact),
    DeviceLostNoSamples,
    ZeroSamples,
    Cancelled,
    CancelCleanupFailed(RecordError),
    Failed(RecordError),
}

/// Non-blocking Stop/Cancel result; native finalization is polled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceoverFinalize {
    Finalizing,
    Finished(VoiceoverCaptureOutcome),
}

/// One explicit microphone-only take. It owns no monitor playback and never
/// enables direct microphone monitoring, leaving Timeline playback/mix policy
/// unchanged and preventing feedback by default.
pub struct VoiceoverCaptureSession {
    target: PathBuf,
    stop: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    recording_gate: Arc<MicRecordingGate>,
    worker: Option<JoinHandle<Result<CapturedMicrophone>>>,
    cancelled: bool,
    terminal: Option<VoiceoverCaptureOutcome>,
}

impl VoiceoverCaptureSession {
    /// Start a take only after an explicit user intent reached the future
    /// server owner. The exact source is resolved and the native device is
    /// reserved before a worker or WAV staging file can exist.
    pub fn start(target: PathBuf, source: MicrophoneSource) -> Result<Self> {
        validate_target(&target)?;
        let reserved = crate::mic_endpoint::reserve_microphone_capture(&source)?;
        let stop = Arc::new(AtomicBool::new(false));
        let ready = Arc::new(AtomicBool::new(false));
        let recording_gate = Arc::new(MicRecordingGate::default());
        let worker = crate::mic_endpoint::spawn_reserved_microphone_capture(
            target.to_string_lossy().into_owned(),
            Arc::clone(&stop),
            Arc::clone(&ready),
            Instant::now(),
            reserved,
            Some(Arc::clone(&recording_gate)),
        );
        Ok(Self {
            target,
            stop,
            ready,
            recording_gate,
            worker: Some(worker),
            cancelled: false,
            terminal: None,
        })
    }

    /// True only after CPAL delivered a real input callback. It never opens a
    /// device, starts playback, or infers signal from a selected endpoint.
    pub fn microphone_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    /// Retain samples only after real readiness plus visual pre-roll.
    /// The armed `Instant` is the WAV timing origin; earlier callbacks are dropped.
    pub fn begin_recording(&mut self) -> Result<()> {
        if self.terminal.is_some() || self.worker.is_none() {
            return Err(RecordError::new(
                error_codes::CAPTURE,
                "voiceover session is no longer active",
                "start a new take instead of trying to restart a completed or failed one",
            ));
        }
        if !self.microphone_ready() {
            return Err(RecordError::new(
                error_codes::CAPTURE,
                "microphone is not ready",
                "wait for a real input callback before starting the voiceover clock",
            ));
        }
        self.recording_gate.arm().map(|_| ())
    }

    /// Whether the one-way recording clock has been armed for this take.
    pub fn recording_started(&self) -> bool {
        self.recording_gate.origin().is_some()
    }

    /// Stop normally. A returned `Finalizing` preserves the session ownership;
    /// call [`Self::status`] until it reports the sealed outcome.
    pub fn stop(&mut self) -> VoiceoverFinalize {
        self.stop.store(true, Ordering::Relaxed);
        self.collect_terminal()
    }

    /// Cancel a take. Any completed regular-file WAV is deleted only after the
    /// worker returns; a failed cleanup is surfaced rather than called cancelled.
    pub fn cancel(&mut self) -> VoiceoverFinalize {
        self.cancelled = true;
        self.stop.store(true, Ordering::Relaxed);
        self.collect_terminal()
    }

    /// Non-blocking terminal check for a Stop/Cancel requested earlier.
    pub fn status(&mut self) -> VoiceoverFinalize {
        self.collect_terminal()
    }

    fn collect_terminal(&mut self) -> VoiceoverFinalize {
        if let Some(terminal) = self.terminal.clone() {
            return VoiceoverFinalize::Finished(terminal);
        }
        let Some(worker) = self.worker.as_ref() else {
            return VoiceoverFinalize::Finalizing;
        };
        if !worker.is_finished() {
            return VoiceoverFinalize::Finalizing;
        }
        let worker = self.worker.take().expect("finished worker stays owned");
        let terminal = match worker.join() {
            Ok(Ok(_captured)) if self.cancelled => cancel_terminal(&self.target),
            Ok(Ok(captured)) => sealed_terminal(captured, &self.target),
            Ok(Err(error)) => VoiceoverCaptureOutcome::Failed(error),
            Err(_) => VoiceoverCaptureOutcome::Failed(RecordError::new(
                error_codes::CAPTURE,
                "voiceover worker stopped unexpectedly",
                "the native microphone worker panicked before returning a terminal outcome",
            )),
        };
        self.terminal = Some(terminal.clone());
        VoiceoverFinalize::Finished(terminal)
    }
}

impl Drop for VoiceoverCaptureSession {
    fn drop(&mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        self.stop.store(true, Ordering::Relaxed);
        let target = self.target.clone();
        let _ = std::thread::Builder::new()
            .name("cut-voiceover-cancel-reaper".into())
            .spawn(move || {
                let _ = worker.join();
                let _ = discard_regular_file(&target);
            });
    }
}

fn sealed_terminal(captured: CapturedMicrophone, target: &Path) -> VoiceoverCaptureOutcome {
    let Some(path) = captured.path else {
        return if captured.microphone_lost {
            VoiceoverCaptureOutcome::DeviceLostNoSamples
        } else {
            VoiceoverCaptureOutcome::ZeroSamples
        };
    };
    let produced = PathBuf::from(path);
    if produced != target {
        return VoiceoverCaptureOutcome::Failed(RecordError::new(
            error_codes::CAPTURE,
            "voiceover artifact escaped its reserved target",
            "the microphone worker returned a path different from its server-owned target",
        ));
    }
    match measure_wav(target) {
        Ok(artifact) if captured.microphone_lost => {
            VoiceoverCaptureOutcome::DeviceLostSavedPrefix(artifact)
        }
        Ok(artifact) => VoiceoverCaptureOutcome::Saved(artifact),
        Err(error) => VoiceoverCaptureOutcome::Failed(error),
    }
}

fn cancel_terminal(target: &Path) -> VoiceoverCaptureOutcome {
    match discard_regular_file(target) {
        Ok(()) => VoiceoverCaptureOutcome::Cancelled,
        Err(error) => VoiceoverCaptureOutcome::CancelCleanupFailed(error),
    }
}

fn validate_target(target: &Path) -> Result<()> {
    if target
        .extension()
        .is_none_or(|extension| extension != "wav")
    {
        return Err(RecordError::new(
            error_codes::INVALID_ARGS,
            "voiceover target must be a WAV file",
            "the reusable voiceover owner writes only independently editable WAV audio",
        ));
    }
    let parent = target.parent().ok_or_else(|| {
        RecordError::new(
            error_codes::INVALID_ARGS,
            "voiceover target has no parent directory",
            "the server owner must reserve a project-owned artifact directory first",
        )
    })?;
    let metadata = std::fs::symlink_metadata(parent).map_err(|error| {
        RecordError::new(
            error_codes::IO,
            "inspect voiceover target directory",
            error.to_string(),
        )
    })?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(RecordError::new(
            error_codes::IO,
            "voiceover target directory is unsafe",
            "the server owner must provide an existing non-symlink project-owned directory",
        ));
    }
    match std::fs::symlink_metadata(target) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(RecordError::new(
            error_codes::GUARDRAIL,
            "voiceover target already exists",
            "the server owner must allocate a fresh project-owned WAV name before capture",
        )),
        Err(error) => Err(RecordError::new(
            error_codes::IO,
            "inspect voiceover target",
            error.to_string(),
        )),
    }
}

fn discard_regular_file(path: &Path) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(RecordError::new(
                error_codes::IO,
                "inspect cancelled voiceover artifact",
                error.to_string(),
            ))
        }
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(RecordError::new(
            error_codes::IO,
            "cancelled voiceover artifact is unsafe to remove",
            "the reserved artifact path is no longer a plain regular file",
        ));
    }
    std::fs::remove_file(path).map_err(|error| {
        RecordError::new(
            error_codes::IO,
            "remove cancelled voiceover artifact",
            error.to_string(),
        )
    })
}

fn measure_wav(path: &Path) -> Result<VoiceoverArtifact> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        RecordError::new(error_codes::IO, "inspect voiceover WAV", error.to_string())
    })?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() || metadata.len() == 0 {
        return Err(RecordError::new(
            error_codes::IO,
            "voiceover WAV is not a sealed regular file",
            "the microphone worker did not publish a readable non-empty WAV artifact",
        ));
    }
    let reader = hound::WavReader::open(path).map_err(|error| {
        RecordError::new(error_codes::IO, "read voiceover WAV", error.to_string())
    })?;
    let spec = reader.spec();
    if spec.sample_rate == 0 || spec.channels == 0 || reader.duration() == 0 {
        return Err(RecordError::new(
            error_codes::CAPTURE,
            "voiceover WAV has no samples",
            "the microphone worker published an empty or invalid WAV artifact",
        ));
    }
    let frames = u64::from(reader.duration());
    let duration_ms = frames
        .saturating_mul(1_000)
        .saturating_div(u64::from(spec.sample_rate));
    let sha256 = hash_file(path)?;
    Ok(VoiceoverArtifact {
        path: path.to_path_buf(),
        sha256,
        bytes: metadata.len(),
        duration_ms,
        sample_rate_hz: spec.sample_rate,
        channels: spec.channels,
    })
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|error| {
        RecordError::new(error_codes::IO, "hash voiceover WAV", error.to_string())
    })?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            RecordError::new(error_codes::IO, "hash voiceover WAV", error.to_string())
        })?;
        if read == 0 {
            return Ok(format!("{:x}", digest.finalize()));
        }
        digest.update(&buffer[..read]);
    }
}

#[cfg(test)]
#[path = "voiceover_capture_tests.rs"]
mod tests;

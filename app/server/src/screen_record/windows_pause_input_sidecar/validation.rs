use super::{canonical, digest, file_name, InputSidecar, SCHEMA};
use record_recovery::{
    is_plain_regular_file, CaptureRoot, DurableStateTransition, RecordingInputSidecarPin,
    RecordingSessionIntent, RecordingSessionJournal, RecordingSessionJournalFile, SealedRun,
};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::Path;

// A v2 sidecar embeds one canonical run already admitted by the session
// journal's byte limit. Its remaining valid fields are fixed scalar metadata,
// the bounded capture/target/project identities, and at most two audio sources.
// Those fields fit comfortably within a second journal budget, preserving long
// runs with many checkpoint fragments without imposing a smaller recording cap.
pub(super) const MAX_INPUT_SIDECAR_BYTES: u64 = 2 * record_recovery::MAX_SESSION_JOURNAL_BYTES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedInputAudio {
    stream: record_recovery::RecordingStream,
    artifact: String,
    bytes: u64,
    sha256: String,
    media_duration_ms: u64,
    native_ready_raw_ms: u64,
    raw_start_ms: u64,
    logical_start_ms: u64,
}

impl VerifiedInputAudio {
    pub(crate) fn stream(&self) -> record_recovery::RecordingStream {
        self.stream
    }

    pub(crate) fn artifact(&self) -> &str {
        &self.artifact
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    pub(crate) fn sha256(&self) -> &str {
        &self.sha256
    }

    pub(crate) fn media_duration_ms(&self) -> u64 {
        self.media_duration_ms
    }

    pub(crate) fn native_ready_offset_ms(&self) -> u64 {
        self.native_ready_raw_ms - self.raw_start_ms
    }

    pub(crate) fn logical_start_ms(&self) -> u64 {
        self.logical_start_ms
    }

    #[cfg(test)]
    pub(crate) fn test_audio(
        stream: record_recovery::RecordingStream,
        artifact: impl Into<String>,
        bytes: u64,
        sha256: impl Into<String>,
        media_duration_ms: u64,
        native_ready_raw_ms: u64,
        raw_start_ms: u64,
        logical_start_ms: u64,
    ) -> Self {
        Self {
            stream,
            artifact: artifact.into(),
            bytes,
            sha256: sha256.into(),
            media_duration_ms,
            native_ready_raw_ms,
            raw_start_ms,
            logical_start_ms,
        }
    }
}

pub(super) fn existing_matches(
    root: &CaptureRoot,
    capture_id: &str,
    pin: &RecordingInputSidecarPin,
    expected: &[u8],
) -> Result<(), String> {
    let path = root
        .capture_file(capture_id, &pin.file_name)
        .map_err(|_| "resolve existing recording-input sidecar".to_string())?;
    (read_local(&path, expected.len() as u64)? == expected)
        .then_some(())
        .ok_or_else(|| "recording-input sidecar already exists with different bytes".into())
}

pub(super) fn verify_pinned_run(
    root: &CaptureRoot,
    capture_id: &str,
    run: &SealedRun,
    readiness: &DurableStateTransition,
    pin: &RecordingInputSidecarPin,
) -> Result<(), String> {
    let dir = root
        .existing_capture_dir(capture_id)
        .map_err(|_| "resolve recording-input capture directory".to_string())?
        .ok_or_else(|| "recording-input capture directory is missing".to_string())?;
    verify_one(&dir, capture_id, None, run, readiness, pin).map(|_| ())
}

pub(crate) fn verify_pinned_inputs(
    root: &CaptureRoot,
    capture_id: &str,
    journal: &RecordingSessionJournal,
) -> Result<Vec<RecordingInputSidecarPin>, String> {
    if journal.intent().input_sidecars_required {
        let binding = journal
            .intent()
            .project_binding
            .as_ref()
            .ok_or_else(|| "required sidecar journal has no project binding".to_string())?;
        super::verify_project_binding(root, binding)?;
    }
    let dir = root
        .existing_capture_dir(capture_id)
        .map_err(|_| "resolve recording-input capture directory".to_string())?
        .ok_or_else(|| "recording-input capture directory is missing".to_string())?;
    verify_pinned_inputs_at(&dir, capture_id, journal)
}

pub(crate) fn verify_recovery_capture(root: &CaptureRoot, capture_id: &str) -> Result<(), String> {
    let journal = RecordingSessionJournalFile::replay(root, capture_id)
        .map_err(|_| "recording-input journal cannot be replayed".to_string())?;
    verify_pinned_inputs(root, capture_id, &journal).map(|_| ())
}

/// Return only leaves already proved by the same no-follow sidecar recovery
/// contract. Projection uses these private descriptors to stage compact audio;
/// it never infers a track from directory contents or a project field.
pub(crate) fn verified_audio_sources(
    root: &CaptureRoot,
    capture_id: &str,
    journal: &RecordingSessionJournal,
) -> Result<Vec<VerifiedInputAudio>, String> {
    if !journal.intent().input_sidecars_required {
        return Ok(Vec::new());
    }
    let binding = journal
        .intent()
        .project_binding
        .as_ref()
        .ok_or_else(|| "required sidecar journal has no project binding".to_string())?;
    super::verify_project_binding(root, binding)?;
    let dir = root
        .existing_capture_dir(capture_id)
        .map_err(|_| "resolve recording-input capture directory".to_string())?
        .ok_or_else(|| "recording-input capture directory is missing".to_string())?;
    if journal.intent().session_id != capture_id {
        return Err("recording-input journal capture identity differs from recovery target".into());
    }
    let mut sources = Vec::new();
    for run in journal.sealed_runs() {
        let pin = journal.input_sidecar(run.sequence).ok_or_else(|| {
            "sealed Windows run has no pinned recording-input sidecar".to_string()
        })?;
        let readiness = journal
            .transitions()
            .iter()
            .find(|transition| transition.sequence == pin.ready_transition_sequence)
            .ok_or_else(|| "recording-input pin has no durable readiness transition".to_string())?;
        sources.extend(verify_one(
            &dir,
            capture_id,
            Some(journal.intent()),
            run,
            readiness,
            pin,
        )?);
    }
    Ok(sources)
}

pub(crate) fn pins_digest(pins: &[RecordingInputSidecarPin]) -> Result<String, String> {
    canonical(pins).map(|bytes| digest(&bytes))
}

fn verify_pinned_inputs_at(
    capture_dir: &Path,
    capture_id: &str,
    journal: &RecordingSessionJournal,
) -> Result<Vec<RecordingInputSidecarPin>, String> {
    if !journal.intent().input_sidecars_required {
        return Ok(Vec::new());
    }
    if journal.intent().session_id != capture_id {
        return Err("recording-input journal capture identity differs from recovery target".into());
    }
    journal
        .sealed_runs()
        .iter()
        .map(|run| {
            let pin = journal
                .input_sidecar(run.sequence)
                .ok_or_else(|| {
                    "sealed Windows run has no pinned recording-input sidecar".to_string()
                })?
                .clone();
            let readiness = journal
                .transitions()
                .iter()
                .find(|transition| transition.sequence == pin.ready_transition_sequence)
                .ok_or_else(|| {
                    "recording-input pin has no durable readiness transition".to_string()
                })?;
            verify_one(
                capture_dir,
                capture_id,
                Some(journal.intent()),
                run,
                readiness,
                &pin,
            )?;
            Ok(pin)
        })
        .collect()
}

fn verify_one(
    capture_dir: &Path,
    capture_id: &str,
    intent: Option<&RecordingSessionIntent>,
    run: &SealedRun,
    readiness: &DurableStateTransition,
    pin: &RecordingInputSidecarPin,
) -> Result<Vec<VerifiedInputAudio>, String> {
    if pin.run_sequence != run.sequence || pin.file_name != file_name(run.sequence) {
        return Err("recording-input sidecar pin does not name its exact run".into());
    }
    let bytes = read_local(&capture_dir.join(&pin.file_name), MAX_INPUT_SIDECAR_BYTES)?;
    if digest(&bytes) != pin.sha256 {
        return Err("recording-input sidecar hash does not match its journal pin".into());
    }
    let sidecar: InputSidecar = serde_json::from_slice(&bytes)
        .map_err(|_| "recording-input sidecar is malformed".to_string())?;
    if canonical(&sidecar)? != bytes
        || sidecar.fingerprint_sha256 != digest(&canonical(&sidecar.body)?)
        || sidecar.body.schema != SCHEMA
        || sidecar.body.capture_id != capture_id
        || intent
            .is_some_and(|intent| intent.project_binding.as_ref() != Some(&sidecar.body.project))
        || sidecar.body.selection.accepted_revision != sidecar.body.project.accepted_revision
        || sidecar.body.run != *run
        || !sidecar.body.selection.valid()
        || intent.is_some_and(|intent| {
            sidecar.body.selection.display_id != intent.target_descriptor
                || f64::from(sidecar.body.selection.fps) != intent.fps
        })
        || !sidecar.body.clocks.matches(run, readiness, pin)
        || sidecar
            .body
            .audio
            .iter()
            .any(|source| !source.valid_for_run(run, &sidecar.body.clocks))
        || sidecar
            .body
            .audio
            .windows(2)
            .any(|pair| pair[0].stream() >= pair[1].stream())
        || !super::InputAudioSource::exactly_covers_run(&sidecar.body.audio, run)
    {
        return Err("recording-input sidecar is stale, ambiguous, or tampered".into());
    }
    super::wav_validation::verify_audio_sources(capture_dir, &sidecar.body.audio, run)?;
    Ok(sidecar
        .body
        .audio
        .iter()
        .map(|source| VerifiedInputAudio {
            stream: source.stream(),
            artifact: source.artifact.clone(),
            bytes: source.bytes,
            sha256: source.sha256.clone(),
            media_duration_ms: source.media_duration_ms,
            native_ready_raw_ms: source.native_ready_raw_ms,
            raw_start_ms: source.raw_start_ms,
            logical_start_ms: source.logical_start_ms,
        })
        .collect())
}

fn read_local(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    if !is_plain_regular_file(path).map_err(|_| "inspect recording-input sidecar".to_string())? {
        return Err("recording-input sidecar is not a local regular file".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options
        .open(path)
        .map_err(|_| "open recording-input sidecar without following links".to_string())?;
    ensure_open_regular(&file)?;
    if file
        .metadata()
        .map_err(|_| "read recording-input sidecar metadata".to_string())?
        .len()
        > limit
    {
        return Err("recording-input sidecar exceeds its byte limit".into());
    }
    read_bounded(file, limit)
}

fn read_bounded(reader: impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| "read recording-input sidecar".to_string())?;
    if bytes.len() as u64 > limit {
        return Err("recording-input sidecar exceeds its byte limit".into());
    }
    Ok(bytes)
}

/// Re-check the opened handle after the no-follow open. This rejects a Windows
/// reparse leaf that appeared between the path preflight and handle creation.
fn ensure_open_regular(file: &File) -> Result<(), String> {
    let metadata = file
        .metadata()
        .map_err(|_| "read recording-input sidecar metadata".to_string())?;
    (metadata.file_type().is_file() && !is_reparse(&metadata))
        .then_some(())
        .ok_or_else(|| "opened recording-input sidecar is not regular".into())
}

#[cfg(windows)]
fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

#[cfg(not(windows))]
fn is_reparse(_metadata: &std::fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod bounds_tests {
    use super::read_bounded;

    #[test]
    fn read_limit_rejects_growth_after_metadata_admission() {
        assert_eq!(read_bounded(&b"abc"[..], 3).unwrap(), b"abc");
        assert_eq!(
            read_bounded(&b"abcd"[..], 3).unwrap_err(),
            "recording-input sidecar exceeds its byte limit"
        );
    }
}

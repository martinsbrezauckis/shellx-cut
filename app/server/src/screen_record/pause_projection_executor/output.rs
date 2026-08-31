use std::path::{Path, PathBuf};

use cut_core::{error_codes, CutError};
use record_core::RecordingProject;
use record_recovery::{
    is_plain_regular_file, CaptureRoot, RecordingInputSidecarPin, RecordingSessionJournal,
};
use serde::{Deserialize, Serialize};

use super::artifacts::{read_nofollow, sha256};
use super::audio::{PreparedPauseProjectionAudio, ProjectionAudioContract};
use super::types::{PauseProjectionExecution, PauseProjectionResult};
use crate::screen_record::pause_projection::LegacyRootProjectionPlan;

const PROJECT_OUTPUT: &str = "project.json";
const RECEIPT_OUTPUT: &str = ".pause-projection.receipt.json";
const RECEIPT_SCHEMA: &str = "shellx-cut/pause-projection-receipt/1";

pub(super) struct OutputPaths<'a> {
    root: &'a CaptureRoot,
    capture_id: &'a str,
    capture_dir: PathBuf,
    source: PathBuf,
    events: PathBuf,
    project: PathBuf,
    receipt: PathBuf,
}

impl<'a> OutputPaths<'a> {
    pub(super) fn new(root: &'a CaptureRoot, capture_id: &'a str) -> Result<Self, CutError> {
        let source = path(root, capture_id, "source.mp4")?;
        let events = path(root, capture_id, "events.json")?;
        let project = path(root, capture_id, PROJECT_OUTPUT)?;
        let receipt = path(root, capture_id, RECEIPT_OUTPUT)?;
        let capture_dir = source
            .parent()
            .ok_or_else(|| invalid("source output has no capture parent"))?
            .to_path_buf();
        Ok(Self {
            root,
            capture_id,
            capture_dir,
            source,
            events,
            project,
            receipt,
        })
    }

    pub(super) fn capture_dir(&self) -> &Path {
        &self.capture_dir
    }

    pub(super) fn source(&self) -> &Path {
        &self.source
    }
}

pub(super) struct ProjectionPayloads {
    events: Vec<u8>,
    project: Vec<u8>,
}

impl ProjectionPayloads {
    pub(super) fn from_plan(
        plan: &LegacyRootProjectionPlan,
        audio: &PreparedPauseProjectionAudio,
    ) -> Result<Self, CutError> {
        let events = serde_json::to_vec_pretty(&plan.merged_events().events)
            .map_err(|error| invalid(format!("serialize merged pause events: {error}")))?;
        let mut project = RecordingProject::new(
            "source.mp4",
            plan.merged_events().settings,
            plan.merged_events().events.clone(),
        );
        project.audio = audio.microphone_file().map(str::to_owned);
        project.capture_cadence = plan.capture_cadence().cloned();
        let project = serde_json::to_vec_pretty(&project)
            .map_err(|error| invalid(format!("serialize merged pause project: {error}")))?;
        Ok(Self { events, project })
    }
}

pub(super) fn completed(
    paths: &OutputPaths<'_>,
    journal: &RecordingSessionJournal,
    payloads: &ProjectionPayloads,
    expected_duration_ms: u64,
    input_pins: &[RecordingInputSidecarPin],
    audio: &ProjectionAudioContract,
) -> Result<Option<PauseProjectionResult>, CutError> {
    let Some(receipt) = read_receipt(&paths.receipt)? else {
        return Ok(None);
    };
    let expected_journal = journal_digest(journal)?;
    let expected_inputs = input_pins_digest(input_pins)?;
    if receipt.schema != RECEIPT_SCHEMA
        || receipt.journal_sha256 != expected_journal
        || receipt.source_duration_ms != expected_duration_ms
        || receipt.input_sidecars_sha256 != expected_inputs
        || receipt.audio != *audio
    {
        return Err(invalid(
            "existing pause projection receipt does not match durable session evidence",
        ));
    }
    if sha256(&paths.source)? != receipt.source_sha256
        || sha256(&paths.events)? != receipt.events_sha256
        || sha256(&paths.project)? != receipt.project_sha256
        || read_nofollow(&paths.events)? != payloads.events
        || read_nofollow(&paths.project)? != payloads.project
    {
        return Err(invalid(
            "existing pause projection outputs do not match their private receipt",
        ));
    }
    audio.verify_published(paths.capture_dir())?;
    Ok(Some(PauseProjectionResult {
        execution: PauseProjectionExecution::AlreadyComplete,
        source_duration_ms: receipt.source_duration_ms,
    }))
}

/// Capture-output handoff has no mutable projection inputs left to compare.
/// It still reopens the completed receipt and every receipt-bound audio leaf,
/// so a post-receipt replacement cannot be returned as a successful capture.
pub(super) fn verify_completed_audio(
    paths: &OutputPaths<'_>,
    expected_audio: &ProjectionAudioContract,
) -> Result<(), CutError> {
    let receipt = read_receipt(&paths.receipt)?.ok_or_else(|| {
        invalid("completed private pause projection has no durable projection receipt")
    })?;
    if receipt.schema != RECEIPT_SCHEMA || receipt.audio != *expected_audio {
        return Err(invalid(
            "completed private pause projection receipt does not match sealed audio evidence",
        ));
    }
    receipt.audio.verify_published(paths.capture_dir())
}

pub(super) fn publish(
    paths: &OutputPaths<'_>,
    journal: &RecordingSessionJournal,
    payloads: &ProjectionPayloads,
    staged_source: &Path,
    source_duration_ms: u64,
    input_pins: &[RecordingInputSidecarPin],
    audio: &ProjectionAudioContract,
) -> Result<PauseProjectionResult, CutError> {
    audio.verify_published(paths.capture_dir())?;
    publish_source(paths, staged_source)?;
    audio.verify_published(paths.capture_dir())?;
    publish_bytes(paths, "events.json", &paths.events, &payloads.events)?;
    audio.verify_published(paths.capture_dir())?;
    publish_bytes(paths, PROJECT_OUTPUT, &paths.project, &payloads.project)?;
    audio.verify_published(paths.capture_dir())?;
    let receipt = ProjectionReceipt {
        schema: RECEIPT_SCHEMA.into(),
        journal_sha256: journal_digest(journal)?,
        source_sha256: sha256(&paths.source)?,
        events_sha256: sha256(&paths.events)?,
        project_sha256: sha256(&paths.project)?,
        source_duration_ms,
        input_sidecars_sha256: input_pins_digest(input_pins)?,
        audio: audio.clone(),
    };
    let bytes = serde_json::to_vec(&receipt)
        .map_err(|error| invalid(format!("serialize pause projection receipt: {error}")))?;
    publish_bytes(paths, RECEIPT_OUTPUT, &paths.receipt, &bytes)?;
    audio.verify_published(paths.capture_dir())?;
    Ok(PauseProjectionResult {
        execution: PauseProjectionExecution::Published,
        source_duration_ms,
    })
}

fn publish_source(paths: &OutputPaths<'_>, staged: &Path) -> Result<(), CutError> {
    if let Some(()) = local_leaf(&paths.source)? {
        if sha256(&paths.source)? == sha256(staged)? {
            return Ok(());
        }
        return Err(invalid(
            "existing source.mp4 differs from the newly staged sealed projection",
        ));
    }
    record_recovery::publish_new_synced(staged, &paths.source)
        .map_err(|error| invalid(format!("publish source.mp4 without replacement: {error}")))
}

fn publish_bytes(
    paths: &OutputPaths<'_>,
    name: &str,
    path: &Path,
    expected: &[u8],
) -> Result<(), CutError> {
    match local_leaf(path)? {
        Some(()) if read_nofollow(path)? == expected => Ok(()),
        Some(()) => Err(invalid(format!(
            "existing {name} differs from the sealed projection"
        ))),
        None => paths
            .root
            .publish_new_capture_file(paths.capture_id, name, expected)
            .map(|_| ())
            .map_err(|error| invalid(format!("publish {name} without replacement: {error}"))),
    }
}

fn read_receipt(path: &Path) -> Result<Option<ProjectionReceipt>, CutError> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(invalid(format!(
            "inspect pause projection receipt: {error}"
        ))),
        Ok(_) => {
            if !is_plain_regular_file(path).map_err(|error| invalid(error.to_string()))? {
                return Err(invalid(
                    "pause projection receipt is not a local regular file",
                ));
            }
            serde_json::from_slice(&read_nofollow(path)?)
                .map(Some)
                .map_err(|error| invalid(format!("pause projection receipt is malformed: {error}")))
        }
    }
}

fn local_leaf(path: &Path) -> Result<Option<()>, CutError> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(invalid(format!("inspect projection output: {error}"))),
        Ok(_) if is_plain_regular_file(path).map_err(|error| invalid(error.to_string()))? => {
            Ok(Some(()))
        }
        Ok(_) => Err(invalid(
            "projection output is linked or is not a local regular file",
        )),
    }
}

fn path(root: &CaptureRoot, capture_id: &str, name: &str) -> Result<PathBuf, CutError> {
    root.capture_file(capture_id, name)
        .map_err(|error| invalid(format!("resolve fixed projection output: {error}")))
}

fn journal_digest(journal: &RecordingSessionJournal) -> Result<String, CutError> {
    let bytes = serde_json::to_vec(&journal.entries())
        .map_err(|error| invalid(format!("serialize durable session identity: {error}")))?;
    use sha2::{Digest, Sha256};
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn input_pins_digest(pins: &[RecordingInputSidecarPin]) -> Result<Option<String>, CutError> {
    (!pins.is_empty())
        .then(|| super::super::windows_pause_input_sidecar::pins_digest(pins))
        .transpose()
        .map_err(invalid)
}

#[derive(Debug, Deserialize, Serialize)]
struct ProjectionReceipt {
    schema: String,
    journal_sha256: String,
    source_sha256: String,
    events_sha256: String,
    project_sha256: String,
    source_duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_sidecars_sha256: Option<String>,
    audio: ProjectionAudioContract,
}

fn invalid(detail: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "cannot execute pause-session legacy projection",
        detail.into(),
    )
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;

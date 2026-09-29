//! Parsing and repair of the append-only JSONL journal.

use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::contract::{
    valid_receipt, CaptureManifest, CaptureStart, Checkpoint, ManifestError, OpenCheckpoint,
    RecoveryReceipt, SCHEMA,
};
use crate::manifest::{
    checkpoint_name, io, is_plain_dir, is_plain_regular_file, staging_name, valid_capture_id,
    MANIFEST_FILE,
};

// A recording checkpoint is small and is appended every 15 seconds. Leave
// room for long sessions while refusing planted project files before parsing.
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Entry {
    Start(CaptureStart),
    Open(OpenCheckpoint),
    Checkpoint(Checkpoint),
    Receipt(RecoveryReceipt),
}

pub(crate) fn read(root: &Path) -> Result<CaptureManifest, ManifestError> {
    let path = root.join(MANIFEST_FILE);
    if !is_plain_dir(root)? || !is_plain_regular_file(&path)? {
        return Err(ManifestError::Invalid(
            "manifest root or file is not a local regular path".into(),
        ));
    }
    let bytes = read_nofollow(&path)?;
    let mut state = ParseState::default();
    let mut offset = 0;
    let mut line_no = 1;
    while offset < bytes.len() {
        let rest = &bytes[offset..];
        let Some(newline) = rest.iter().position(|byte| *byte == b'\n') else {
            // A JSONL record is committed only after its newline and sync. Preserve no
            // matter how parseable these bytes look: this is a crash boundary.
            state.torn_tail = Some(rest.to_vec());
            break;
        };
        let line = &rest[..newline];
        let next = offset + newline + 1;
        if !line.is_empty() {
            let entry = serde_json::from_slice(line)
                .map_err(|error| ManifestError::Corrupt(format!("line {line_no}: {error}")))?;
            state.apply(entry)?;
        }
        offset = next;
        line_no += 1;
    }
    state.finish(bytes[..offset].to_vec())
}

fn read_nofollow(path: &Path) -> Result<Vec<u8>, ManifestError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|source| io(path, source))?;
    if file.metadata().map_err(|source| io(path, source))?.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestError::Invalid(
            "capture manifest exceeds its size limit".into(),
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| io(path, source))?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(ManifestError::Invalid(
            "capture manifest exceeds its size limit".into(),
        ));
    }
    Ok(bytes)
}

/// Quarantine exactly the unsynced final bytes then replace the journal with its
/// synced prefix plus a single sealed receipt. The journal receipt is authoritative;
/// no independent receipt file can disagree after a crash.
pub(crate) fn repair_torn(
    root: &Path,
    manifest: &CaptureManifest,
    receipt: &RecoveryReceipt,
) -> Result<std::path::PathBuf, ManifestError> {
    let manifest_path = root.join(MANIFEST_FILE);
    if !is_plain_dir(root)? || !is_plain_regular_file(&manifest_path)? {
        return Err(ManifestError::Invalid(
            "manifest root or file is not a local regular path".into(),
        ));
    }
    let tail = manifest
        .torn_tail_bytes
        .as_deref()
        .ok_or_else(|| ManifestError::Invalid("manifest has no torn tail to repair".into()))?;
    let quarantine = root.join("quarantine");
    fs::create_dir_all(&quarantine).map_err(|source| io(&quarantine, source))?;
    if !is_plain_dir(&quarantine)? {
        return Err(ManifestError::Invalid("unsafe quarantine directory".into()));
    }
    let tail_path = quarantine.join("capture.manifest.torn-tail.jsonl");
    crate::atomic::replace_synced(&tail_path, tail).map_err(|source| io(&tail_path, source))?;
    let mut repaired = manifest.valid_prefix.clone();
    repaired.extend(serde_json::to_vec(&Entry::Receipt(receipt.clone()))?);
    repaired.push(b'\n');
    crate::atomic::replace_synced(&manifest_path, &repaired)
        .map_err(|source| io(&manifest_path, source))?;
    Ok(tail_path)
}

#[derive(Default)]
struct ParseState {
    start: Option<CaptureStart>,
    checkpoints: Vec<Checkpoint>,
    receipt: Option<RecoveryReceipt>,
    openings: Vec<OpenCheckpoint>,
    torn_tail: Option<Vec<u8>>,
}

impl ParseState {
    fn apply(&mut self, entry: Entry) -> Result<(), ManifestError> {
        match entry {
            Entry::Start(start)
                if self.start.is_none()
                    && start.schema == SCHEMA
                    && valid_capture_id(&start.capture_id) =>
            {
                self.start = Some(start)
            }
            Entry::Start(_) => {
                return Err(ManifestError::Corrupt("invalid or duplicate start".into()))
            }
            Entry::Open(open) => {
                if self.start.is_none()
                    || self.receipt.is_some()
                    || open.staging != staging_name(open.sequence)
                {
                    return Err(ManifestError::Corrupt("invalid open checkpoint".into()));
                }
                if open.sequence != self.checkpoints.len() as u64 || !self.openings.is_empty() {
                    return Err(ManifestError::Corrupt(
                        "stale or duplicate open checkpoint".into(),
                    ));
                }
                self.openings.push(open);
            }
            Entry::Checkpoint(checkpoint) => {
                if self.start.is_none()
                    || self.receipt.is_some()
                    || checkpoint.sequence != self.checkpoints.len() as u64
                    || checkpoint.file != checkpoint_name(checkpoint.sequence)
                    || checkpoint.held_last_frame_ms.is_some_and(|held_ms| {
                        checkpoint.sequence == 0
                            || held_ms == 0
                            || checkpoint
                                .facts
                                .end_ms
                                .checked_sub(checkpoint.facts.start_ms)
                                != Some(held_ms)
                            || checkpoint.media.is_none()
                    })
                    || !self
                        .openings
                        .iter()
                        .any(|open| open.sequence == checkpoint.sequence)
                {
                    return Err(ManifestError::Corrupt(
                        "invalid checkpoint sequence/path".into(),
                    ));
                }
                self.openings
                    .retain(|open| open.sequence != checkpoint.sequence);
                self.checkpoints.push(checkpoint);
            }
            Entry::Receipt(receipt)
                if self.start.is_some()
                    && self.receipt.is_none()
                    && valid_receipt(&receipt, &self.checkpoints, !self.openings.is_empty()) =>
            {
                self.receipt = Some(receipt)
            }
            Entry::Receipt(_) => {
                return Err(ManifestError::Corrupt(
                    "duplicate or premature receipt".into(),
                ))
            }
        }
        Ok(())
    }

    fn finish(self, valid_prefix: Vec<u8>) -> Result<CaptureManifest, ManifestError> {
        let start = self
            .start
            .ok_or_else(|| ManifestError::Corrupt("missing start entry".into()))?;
        Ok(CaptureManifest {
            start,
            checkpoints: self.checkpoints,
            receipt: self.receipt,
            openings: self.openings,
            torn_tail: self.torn_tail.is_some(),
            valid_prefix,
            torn_tail_bytes: self.torn_tail,
        })
    }
}

#[cfg(test)]
mod tests;

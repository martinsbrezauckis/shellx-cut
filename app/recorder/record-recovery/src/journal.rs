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
    let mut file = options.open(path).map_err(|source| io(path, source))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|source| io(path, source))?;
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
mod tests {
    use super::{read, Entry, OpenCheckpoint};
    use crate::{
        recover_interrupted, CaptureStart, Checkpoint, CheckpointFacts, ManifestError,
        ManifestOwner, OwnerState, MANIFEST_FILE,
    };
    use std::fs;
    use tempfile::tempdir;

    fn checkpoint(file: String) -> Checkpoint {
        Checkpoint {
            sequence: 0,
            file,
            bytes: 1,
            sha256: "not-read".into(),
            media: None,
            facts: CheckpointFacts {
                start_ms: 0,
                end_ms: 1,
                event_offset_ms: 0,
                audio_offset_ms: None,
            },
        }
    }

    fn write_journal(root: &std::path::Path, entries: Vec<Entry>) {
        let body = entries
            .into_iter()
            .map(|entry| serde_json::to_string(&entry).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(root.join(MANIFEST_FILE), format!("{body}\n")).unwrap();
    }

    fn start() -> Entry {
        Entry::Start(CaptureStart::new("cap", 100))
    }

    fn open(staging: String) -> Entry {
        Entry::Open(OpenCheckpoint {
            sequence: 0,
            staging,
            start_ms: 0,
        })
    }

    fn assert_rejected_before_recovery(root: &std::path::Path, expected: &str) {
        for error in [
            read(root).unwrap_err(),
            recover_interrupted(root, "not-invoked", "not-invoked", OwnerState::Dead).unwrap_err(),
        ] {
            assert!(matches!(error, ManifestError::Corrupt(ref detail) if detail == expected));
        }
        assert!(!root.join("quarantine").exists());
    }

    #[test]
    fn rejects_absolute_traversal_and_nested_checkpoint_paths_before_recovery() {
        let outside = tempdir().unwrap();
        let outside_file = outside.path().join("outside.mp4");
        fs::write(&outside_file, "outside remains untouched").unwrap();
        for file in [
            outside_file.to_string_lossy().into_owned(),
            "checkpoints/../outside.mp4".into(),
            "other/segment-000000.mp4".into(),
        ] {
            let root = tempdir().unwrap();
            write_journal(
                root.path(),
                vec![
                    start(),
                    open(".checkpoint-000000.open.mp4".into()),
                    Entry::Checkpoint(checkpoint(file)),
                ],
            );
            assert_rejected_before_recovery(root.path(), "invalid checkpoint sequence/path");
            assert_eq!(
                fs::read_to_string(&outside_file).unwrap(),
                "outside remains untouched"
            );
        }
    }

    #[test]
    fn rejects_absolute_traversal_and_nested_open_paths_before_recovery() {
        let outside = tempdir().unwrap();
        let outside_file = outside.path().join("outside.mp4");
        fs::write(&outside_file, "outside remains untouched").unwrap();
        for staging in [
            outside_file.to_string_lossy().into_owned(),
            "../outside.open.mp4".into(),
            "nested/.checkpoint-000000.open.mp4".into(),
        ] {
            let root = tempdir().unwrap();
            write_journal(root.path(), vec![start(), open(staging)]);
            assert_rejected_before_recovery(root.path(), "invalid open checkpoint");
            assert_eq!(
                fs::read_to_string(&outside_file).unwrap(),
                "outside remains untouched"
            );
        }
    }

    #[test]
    fn rejects_unsafe_start_capture_id() {
        let root = tempdir().unwrap();
        assert!(ManifestOwner::begin(root.path(), CaptureStart::new("../cap", 100)).is_err());
        let mut start = CaptureStart::new("../cap", 100);
        start.schema = crate::contract::SCHEMA.into();
        write_journal(root.path(), vec![Entry::Start(start)]);
        assert_rejected_before_recovery(root.path(), "invalid or duplicate start");
    }

    #[cfg(unix)]
    #[test]
    fn checkpoint_symlink_is_rejected_before_probe_or_quarantine() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let outside_file = outside.path().join("outside.mp4");
        fs::write(&outside_file, "outside remains untouched").unwrap();
        fs::create_dir(root.path().join("checkpoints")).unwrap();
        let link = root.path().join("checkpoints/segment-000000.mp4");
        symlink(&outside_file, &link).unwrap();
        write_journal(
            root.path(),
            vec![
                start(),
                open(".checkpoint-000000.open.mp4".into()),
                Entry::Checkpoint(checkpoint("checkpoints/segment-000000.mp4".into())),
            ],
        );
        let result =
            recover_interrupted(root.path(), "not-invoked", "not-invoked", OwnerState::Dead)
                .unwrap()
                .unwrap();
        assert_eq!(result.receipt.state, crate::RecoveryState::Interrupted);
        assert!(result.quarantined.is_none());
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::read_to_string(&outside_file).unwrap(),
            "outside remains untouched"
        );
        assert!(!root.path().join("quarantine").exists());
    }
}

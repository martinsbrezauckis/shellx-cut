//! Operation-journal validation and crash-tail recovery.
//!
//! `ops.jsonl` remains the source of truth. This module performs the one full
//! scan at open, refuses malformed middle records, and preserves a malformed
//! final record in a sidecar before truncating back to the valid prefix.

use crate::error::{codes, CutError};
use crate::mutation_request::RequestIndex;
use crate::ops::{JournalRecovery, OpRecord};
use serde::Serialize;
use std::fs::{File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

pub(crate) struct JournalScan {
    pub next_seq: u64,
    pub recovery: Option<JournalRecovery>,
    pub requests: RequestIndex,
    /// Strictly parsed records retained by the open-time scan. The in-memory
    /// journal index adopts these as its immutable replay view.
    pub records: Vec<OpRecord>,
}

#[derive(Serialize)]
struct RecoveryNote<'a> {
    schema: &'static str,
    journal: &'a str,
    discarded_start: u64,
    discarded_end: u64,
    quarantine_file: &'a str,
    cause: &'a str,
}

struct ParsedJournal {
    records: Vec<OpRecord>,
    torn_tail: Option<(usize, String)>,
}

fn trimmed(line: &[u8]) -> &[u8] {
    let Some(first) = line.iter().position(|byte| !byte.is_ascii_whitespace()) else {
        return &[];
    };
    let last = line
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .expect("a first non-whitespace byte has a last byte");
    &line[first..=last]
}

fn has_nonempty_line(bytes: &[u8]) -> bool {
    bytes
        .split(|byte| *byte == b'\n')
        .any(|line| !trimmed(line).is_empty())
}

fn corrupt_middle(offset: usize, cause: &serde_json::Error) -> CutError {
    CutError::new(
        codes::INVALID_ARGS,
        "operation journal contains a malformed middle record",
        format!("ops.jsonl byte {offset}: {cause}"),
    )
    .with_suggested_action(
        "do not append or truncate the journal; restore a known-good project copy or inspect it with project repair tooling",
    )
}

fn parse(bytes: &[u8]) -> Result<ParsedJournal, CutError> {
    let mut records = Vec::new();
    let mut start = 0usize;
    while start < bytes.len() {
        let newline = bytes[start..].iter().position(|byte| *byte == b'\n');
        let end = newline.map_or(bytes.len(), |relative| start + relative);
        let next = newline.map_or(bytes.len(), |_| end + 1);
        let line = trimmed(&bytes[start..end]);
        if !line.is_empty() {
            match serde_json::from_slice::<OpRecord>(line) {
                Ok(record) => records.push(record),
                Err(cause) if has_nonempty_line(&bytes[next..]) => {
                    return Err(corrupt_middle(start, &cause));
                }
                Err(cause) => {
                    return Ok(ParsedJournal {
                        records,
                        torn_tail: Some((start, cause.to_string())),
                    });
                }
            }
        }
        start = next;
    }
    Ok(ParsedJournal {
        records,
        torn_tail: None,
    })
}

fn sync_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
}

fn same_file(left: &Metadata, right: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev() && left.ino() == right.ino()
    }
    #[cfg(not(unix))]
    {
        left.len() == right.len() && left.modified().ok() == right.modified().ok()
    }
}

/// Check the literal journal leaf before opening it, and keep recovery tied to
/// the same file that was parsed. A project-supplied symlink must never turn a
/// crash-tail repair into a write to an unrelated local file.
pub(crate) fn open_plain_journal(
    path: &Path,
    write: bool,
    append: bool,
    expected: Option<&Metadata>,
) -> Result<File, CutError> {
    let leaf = std::fs::symlink_metadata(path)?;
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        leaf.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    if !leaf.file_type().is_file() || leaf.file_type().is_symlink() || reparse {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "operation journal is not a plain file",
            format!("refusing linked or non-file journal {}", path.display()),
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true).write(write).append(append);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        #[cfg(target_os = "linux")]
        const O_NOFOLLOW: i32 = 0o400000;
        #[cfg(target_os = "macos")]
        const O_NOFOLLOW: i32 = 0x100;
        options.custom_flags(O_NOFOLLOW);
    }
    let file = options.open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file()
        || !same_file(&leaf, &opened)
        || expected.is_some_and(|original| !same_file(original, &opened))
    {
        return Err(CutError::new(
            codes::CONFLICT,
            "operation journal changed while opening",
            format!("journal identity changed at {}", path.display()),
        ));
    }
    Ok(file)
}

fn unique_recovery_names(path: &Path) -> (String, String) {
    let base = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("ops.jsonl");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let stem = format!("{base}.recovery-{}-{nonce}", std::process::id());
    (format!("{stem}.tail"), format!("{stem}.json"))
}

fn recover_tail(
    path: &Path,
    journal_file: &File,
    bytes: &[u8],
    start: usize,
    cause: &str,
) -> Result<JournalRecovery, CutError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let (quarantine_file, note_file) = unique_recovery_names(path);
    let quarantine_path = parent.join(&quarantine_file);
    let note_path = parent.join(&note_file);

    let mut quarantine = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&quarantine_path)?;
    quarantine.write_all(&bytes[start..])?;
    quarantine.sync_all()?;

    let journal = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("ops.jsonl");
    let note = RecoveryNote {
        schema: "shellx-cut/journal-recovery/1",
        journal,
        discarded_start: start as u64,
        discarded_end: bytes.len() as u64,
        quarantine_file: &quarantine_file,
        cause,
    };
    let mut note_out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&note_path)?;
    note_out.write_all(serde_json::to_string_pretty(&note)?.as_bytes())?;
    note_out.write_all(b"\n")?;
    note_out.sync_all()?;

    journal_file.set_len(start as u64)?;
    journal_file.sync_all()?;
    sync_parent(path);

    Ok(JournalRecovery {
        discarded_start: start as u64,
        discarded_end: bytes.len() as u64,
        quarantine_file,
        note_file,
    })
}

fn next_sequence(records: &[OpRecord]) -> u64 {
    records
        .iter()
        .filter_map(|record| record.op_id.strip_prefix("op_")?.parse::<u64>().ok())
        .max()
        .unwrap_or(records.len() as u64)
}

pub(crate) fn open_and_recover(path: &Path) -> Result<JournalScan, CutError> {
    let mut read_file = open_plain_journal(path, false, false, None)?;
    let original = read_file.metadata()?;
    let mut bytes = Vec::new();
    read_file.read_to_end(&mut bytes)?;
    let parsed = parse(&bytes)?;
    // Acquire the write handle before creating recovery evidence. If the
    // journal changed after the read, fail without creating sidecars or editing.
    let mut write_file =
        if parsed.torn_tail.is_some() || (!bytes.is_empty() && bytes.last() != Some(&b'\n')) {
            let file = open_plain_journal(path, true, false, Some(&original))?;
            if file.metadata()?.len() != bytes.len() as u64 {
                return Err(CutError::new(
                    codes::CONFLICT,
                    "operation journal changed during recovery",
                    format!("journal length changed at {}", path.display()),
                ));
            }
            Some(file)
        } else {
            None
        };
    let recovery = match parsed.torn_tail {
        Some((start, cause)) => Some(recover_tail(
            path,
            write_file
                .as_ref()
                .expect("recovery has a checked write handle"),
            &bytes,
            start,
            &cause,
        )?),
        None => None,
    };
    let byte_len = recovery
        .as_ref()
        .map_or(bytes.len() as u64, |recovery| recovery.discarded_start);

    // A complete final JSON record without its newline is valid, but appending
    // directly after it would concatenate two objects. Canonicalize that
    // boundary once at open.
    if recovery.is_none() && byte_len > 0 && bytes.last() != Some(&b'\n') {
        let file = write_file
            .as_mut()
            .expect("newline has a checked write handle");
        use std::io::{Seek, SeekFrom};
        file.seek(SeekFrom::End(0))?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }

    let requests = RequestIndex::from_records(&parsed.records)?;
    Ok(JournalScan {
        next_seq: next_sequence(&parsed.records),
        recovery,
        requests,
        records: parsed.records,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{Actor, ActorKind, OpLog, OpStatus};

    fn op(sequence: u64) -> OpRecord {
        OpRecord {
            op_id: OpRecord::format_id(sequence),
            ts: "2026-08-08T00:00:00.000Z".into(),
            actor: Actor {
                kind: ActorKind::Agent,
                name: "journal-test".into(),
                via: "test".into(),
                request: None,
            },
            verb: "edit.add_marker".into(),
            args: serde_json::json!({"at_ms": sequence}),
            rationale: None,
            effects: Vec::new(),
            inverse: None,
            status: OpStatus::Applied,
        }
    }

    fn line(record: &OpRecord, newline: bool) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(record).unwrap();
        if newline {
            bytes.push(b'\n');
        }
        bytes
    }

    #[test]
    fn torn_final_record_is_quarantined_and_next_id_does_not_collide() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ops.jsonl");
        let mut bytes = line(&op(0), true);
        let valid_end = bytes.len() as u64;
        let torn = br#"{"op_id":"op_000002","ts":"2026"#;
        bytes.extend_from_slice(torn);
        std::fs::write(&path, &bytes).unwrap();

        let log = OpLog::open(&path).expect("a malformed final record is recoverable");
        let recovery = log.recovery().expect("recovery must be disclosed");
        assert_eq!(recovery.discarded_start, valid_end);
        assert_eq!(recovery.discarded_end, bytes.len() as u64);
        assert_eq!(
            std::fs::read(dir.path().join(&recovery.quarantine_file)).unwrap(),
            torn
        );
        let note: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join(&recovery.note_file)).unwrap())
                .unwrap();
        assert_eq!(note["discarded_start"], valid_end);
        assert_eq!(note["discarded_end"], bytes.len() as u64);
        assert_eq!(log.read_all().unwrap(), vec![op(0)]);
        assert_eq!(log.next_id().unwrap(), "op_000002");

        log.append(&op(1)).unwrap();
        assert_eq!(
            log.read_all()
                .unwrap()
                .iter()
                .map(|record| record.op_id.as_str())
                .collect::<Vec<_>>(),
            vec!["op_000001", "op_000002"]
        );
    }

    #[test]
    fn malformed_middle_record_fails_closed_without_changing_the_journal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ops.jsonl");
        let mut bytes = line(&op(0), true);
        bytes.extend_from_slice(b"{not-json}\n");
        bytes.extend_from_slice(&line(&op(1), true));
        std::fs::write(&path, &bytes).unwrap();

        let error = OpLog::open(&path).unwrap_err();
        assert!(error.message.contains("malformed middle"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "fail-closed open must not create recovery sidecars"
        );
    }

    #[test]
    fn complete_final_record_without_newline_is_preserved_before_append() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ops.jsonl");
        std::fs::write(&path, line(&op(0), false)).unwrap();

        let log = OpLog::open(&path).unwrap();
        assert!(log.recovery().is_none());
        log.append(&op(1)).unwrap();
        assert_eq!(log.read_all().unwrap(), vec![op(0), op(1)]);
    }

    #[test]
    fn append_refuses_an_external_journal_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ops.jsonl");
        let log = OpLog::open(&path).unwrap();
        std::fs::write(&path, b" \n").unwrap();

        let error = log.append(&op(0)).unwrap_err();
        assert_eq!(error.code, codes::CONFLICT);
        assert!(error.message.contains("changed outside"));
    }

    #[cfg(unix)]
    #[test]
    fn linked_journal_is_rejected_before_tail_recovery_touches_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("unrelated.jsonl");
        let torn = b"{not-json";
        std::fs::write(&outside, torn).unwrap();
        let project = dir.path().join("project.cutproj");
        std::fs::create_dir(&project).unwrap();
        std::os::unix::fs::symlink(&outside, project.join("ops.jsonl")).unwrap();

        assert!(OpLog::open(&project.join("ops.jsonl")).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), torn);
        assert_eq!(std::fs::read_dir(&project).unwrap().count(), 1);
    }
}

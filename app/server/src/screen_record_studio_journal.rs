//! Recorder-owned append-only persistence for live Studio events.
//!
//! Each fully synced line is one accepted event. An interrupted final line is
//! deliberately ignored and removed before the next append, so a power loss
//! cannot turn earlier, durable Studio decisions into an unreadable log. The
//! first append also syncs its newly-created parent directory on Unix.

use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use cut_core::{error_codes, CutError};
use serde::{Deserialize, Serialize};

use crate::screen_record_studio::{validate_studio_event, StudioEvent, StudioEventLog};

pub(crate) const STUDIO_EVENTS_FILENAME: &str = "studio-events.jsonl";
const STUDIO_EVENTS_VERSION: u32 = 1;
const MAX_STUDIO_EVENTS_JOURNAL_BYTES: u64 = 4 * 1024 * 1024;
const MAX_STUDIO_EVENTS: usize = 50_000;

#[derive(Debug, Serialize, Deserialize)]
struct StudioJournalRecord {
    version: u32,
    logical_ts: u64,
    event: StudioEvent,
}

struct RecoveredJournal {
    log: StudioEventLog,
    durable_len: u64,
}

/// The one daemon-local owner for live Studio journal writes.
///
/// Dispatches can arrive concurrently from marker hotkeys and Studio controls.
/// The AppState mutex serializes them through this owner, which assigns a
/// strictly increasing logical timestamp after it has recovered the durable
/// tail. `logical_ts` therefore reflects server acceptance order, not timing
/// jitter between browser requests.
#[derive(Default)]
pub(crate) struct StudioJournalOwner {
    latest_logical_ts: u64,
}

impl StudioJournalOwner {
    pub(crate) fn append(
        &mut self,
        path: &Path,
        mut event: StudioEvent,
    ) -> Result<StudioEventLog, CutError> {
        let mut recovered = recover_journal(path)?;
        if recovered.log.events.len() >= MAX_STUDIO_EVENTS {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("Studio event limit reached ({MAX_STUDIO_EVENTS})"),
                "Recording Studio event metadata must be bounded",
            ));
        }

        let durable_clock = recovered
            .log
            .events
            .iter()
            .filter_map(|item| item.logical_ts)
            .max()
            .unwrap_or(0);
        self.latest_logical_ts = self.latest_logical_ts.max(durable_clock);
        self.latest_logical_ts = self.latest_logical_ts.checked_add(1).ok_or_else(|| {
            CutError::new(
                error_codes::IO,
                "Studio event logical timestamp overflowed",
                "start a new recording before adding more Studio events",
            )
        })?;
        event.logical_ts = Some(self.latest_logical_ts);
        append_record(
            path,
            &StudioJournalRecord {
                version: STUDIO_EVENTS_VERSION,
                logical_ts: self.latest_logical_ts,
                event: event.clone(),
            },
            recovered.durable_len,
        )?;
        recovered.log.events.push(event);
        Ok(recovered.log)
    }
}

/// Read all fully committed records. A final unterminated record is a possible
/// crash tail, not a malformed history; it is excluded until the next append
/// truncates it under the same recorder owner.
pub(crate) fn read_studio_journal(path: &Path) -> Result<StudioEventLog, CutError> {
    Ok(recover_journal(path)?.log)
}

fn recover_journal(path: &Path) -> Result<RecoveredJournal, CutError> {
    if !path.exists() {
        return Ok(RecoveredJournal {
            log: StudioEventLog::default(),
            durable_len: 0,
        });
    }
    let meta = path
        .metadata()
        .map_err(|error| journal_io_error(path, "stat", error))?;
    if meta.len() > MAX_STUDIO_EVENTS_JOURNAL_BYTES {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "Studio event journal is too large: {} bytes (limit: {} bytes)",
                meta.len(),
                MAX_STUDIO_EVENTS_JOURNAL_BYTES
            ),
            "Recording Studio event metadata must be bounded",
        ));
    }
    let bytes = std::fs::read(path).map_err(|error| journal_io_error(path, "read", error))?;
    let mut events = Vec::new();
    let mut durable_len = 0usize;
    let mut last_logical_ts = 0u64;
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        if !line.ends_with(b"\n") {
            break;
        }
        durable_len += line.len();
        let line = &line[..line.len() - 1];
        if line.is_empty() {
            return Err(journal_invalid("contains an empty committed record"));
        }
        let record: StudioJournalRecord = serde_json::from_slice(line).map_err(|error| {
            journal_invalid(&format!("contains malformed committed JSON: {error}"))
        })?;
        if record.version != STUDIO_EVENTS_VERSION {
            return Err(journal_invalid(&format!(
                "has unsupported record version {}",
                record.version
            )));
        }
        if record.logical_ts <= last_logical_ts {
            return Err(journal_invalid(
                "has non-increasing recorder logical timestamps",
            ));
        }
        validate_studio_event(&record.event)?;
        let mut event = record.event;
        event.logical_ts = Some(record.logical_ts);
        events.push(event);
        if events.len() > MAX_STUDIO_EVENTS {
            return Err(journal_invalid("has too many events"));
        }
        last_logical_ts = record.logical_ts;
    }
    Ok(RecoveredJournal {
        log: StudioEventLog {
            version: STUDIO_EVENTS_VERSION,
            events,
        },
        durable_len: durable_len as u64,
    })
}

fn append_record(
    path: &Path,
    record: &StudioJournalRecord,
    durable_len: u64,
) -> Result<(), CutError> {
    let bytes = serde_json::to_vec(record).map_err(|error| {
        CutError::new(
            error_codes::IO,
            format!("could not serialize Studio journal record: {error}"),
            "Studio event metadata serialization failed",
        )
    })?;
    let record_len = u64::try_from(bytes.len())
        .ok()
        .and_then(|len| len.checked_add(1))
        .ok_or_else(|| journal_invalid("record length overflowed"))?;
    if durable_len > MAX_STUDIO_EVENTS_JOURNAL_BYTES
        || record_len > MAX_STUDIO_EVENTS_JOURNAL_BYTES - durable_len
    {
        return Err(journal_invalid("would exceed the 4 MiB size limit"));
    }
    let created = !path.exists();
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| journal_io_error(path, "open", error))?;
    let actual_len = file
        .metadata()
        .map_err(|error| journal_io_error(path, "stat", error))?
        .len();
    if actual_len < durable_len {
        return Err(journal_invalid(
            "changed while the recorder was preparing an append",
        ));
    }
    if actual_len > durable_len {
        file.set_len(durable_len)
            .map_err(|error| journal_io_error(path, "recover", error))?;
        file.sync_data()
            .map_err(|error| journal_io_error(path, "sync recovered tail", error))?;
    }
    file.seek(SeekFrom::Start(durable_len))
        .map_err(|error| journal_io_error(path, "seek to durable tail", error))?;
    file.write_all(&bytes)
        .and_then(|()| file.write_all(b"\n"))
        .map_err(|error| journal_io_error(path, "append", error))?;
    file.sync_data()
        .map_err(|error| journal_io_error(path, "sync", error))?;
    if created {
        sync_created_journal_parent(path)?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_created_journal_parent(path: &Path) -> Result<(), CutError> {
    let parent = path
        .parent()
        .ok_or_else(|| journal_invalid("has no parent directory"))?;
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| journal_io_error(path, "sync parent directory", error))
}

#[cfg(not(unix))]
fn sync_created_journal_parent(_path: &Path) -> Result<(), CutError> {
    Ok(())
}

fn journal_io_error(path: &Path, action: &str, error: std::io::Error) -> CutError {
    CutError::new(
        error_codes::IO,
        format!(
            "could not {action} Studio event journal {}: {error}",
            path.display()
        ),
        "writing live Studio metadata failed",
    )
}

fn journal_invalid(reason: &str) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        format!("Studio event journal {reason}"),
        "discard the incomplete capture or retry recording before polish",
    )
}

#[cfg(test)]
#[path = "screen_record_studio_journal_tests.rs"]
mod tests;

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
mod tests {
    use super::*;

    fn marker(t_ms: u64) -> StudioEvent {
        StudioEvent {
            logical_ts: None,
            t_ms,
            source: "recording".into(),
            kind: "marker".into(),
            visible: None,
            x: None,
            y: None,
            size: None,
            shape: None,
            radius: None,
            label: Some("Mark".into()),
            background: None,
        }
    }

    #[tokio::test]
    async fn owner_serializes_overlapping_event_requests() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(STUDIO_EVENTS_FILENAME);
        let owner = std::sync::Arc::new(tokio::sync::Mutex::new(StudioJournalOwner::default()));
        let mut background = marker(90);
        background.source = "background".into();
        background.kind = "style".into();
        background.label = None;
        background.background = Some("solid".into());
        let first_owner = owner.clone();
        let first_path = path.clone();
        let first = async move { first_owner.lock().await.append(&first_path, marker(120)) };
        let second_owner = owner.clone();
        let second_path = path.clone();
        let second = async move { second_owner.lock().await.append(&second_path, background) };
        let (first, second) = tokio::join!(first, second);
        first.unwrap();
        second.unwrap();
        let log = read_studio_journal(&path).unwrap();

        assert_eq!(log.events.len(), 2);
        assert_eq!(log.events[0].logical_ts, Some(1));
        assert_eq!(log.events[1].logical_ts, Some(2));
        assert_eq!(log.events[1].t_ms, 90, "client timing is preserved");
    }

    #[test]
    fn recovers_a_crash_truncated_tail_before_the_next_append() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(STUDIO_EVENTS_FILENAME);
        let mut owner = StudioJournalOwner::default();
        owner.append(&path, marker(20)).unwrap();
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(br#"{\"version\":1,\"logical_ts\":2,\"event\":"#)
            .unwrap();

        assert_eq!(read_studio_journal(&path).unwrap().events.len(), 1);
        let recovered = owner.append(&path, marker(40)).unwrap();
        assert_eq!(recovered.events.len(), 2);
        assert_eq!(recovered.events[1].logical_ts, Some(2));
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 2);
    }

    #[test]
    fn first_append_is_readable_after_a_new_journal_is_created() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(STUDIO_EVENTS_FILENAME);
        StudioJournalOwner::default()
            .append(&path, marker(20))
            .unwrap();
        assert!(std::fs::read(&path).unwrap().ends_with(b"\n"));
        assert_eq!(read_studio_journal(&path).unwrap().events.len(), 1);
    }

    #[test]
    fn append_stops_at_the_size_limit_without_mutating_the_journal() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(STUDIO_EVENTS_FILENAME);
        let record = StudioJournalRecord {
            version: 1,
            logical_ts: 1,
            event: marker(20),
        };
        let record_len = serde_json::to_vec(&record).unwrap().len() + 1;
        let initial = vec![b'x'; MAX_STUDIO_EVENTS_JOURNAL_BYTES as usize - record_len];
        std::fs::write(&path, &initial).unwrap();
        append_record(&path, &record, initial.len() as u64).unwrap();
        let at_limit = std::fs::read(&path).unwrap();
        assert_eq!(at_limit.len(), MAX_STUDIO_EVENTS_JOURNAL_BYTES as usize);
        assert!(append_record(&path, &record, at_limit.len() as u64).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), at_limit);
    }
}

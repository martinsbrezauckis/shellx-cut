//! Recorder-owned append-only persistence for live Studio events.
//!
//! Each fully synced line is one accepted event. An interrupted final line is
//! deliberately ignored and removed before the next append, so a power loss
//! cannot turn earlier, durable Studio decisions into an unreadable log. The
//! first append also syncs its newly-created parent directory on Unix.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
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
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) if plain_regular(&meta) => meta,
        Ok(_) => return Err(journal_invalid("is not a local regular file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RecoveredJournal {
                log: StudioEventLog::default(),
                durable_len: 0,
            });
        }
        Err(error) => return Err(journal_io_error(path, "stat", error)),
    };
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
    let mut file = open_nofollow(path, false)?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_STUDIO_EVENTS_JOURNAL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| journal_io_error(path, "read", error))?;
    if bytes.len() as u64 > MAX_STUDIO_EVENTS_JOURNAL_BYTES {
        return Err(journal_invalid("exceeds its byte limit"));
    }
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
    let created = match fs::symlink_metadata(path) {
        Ok(metadata) if plain_regular(&metadata) => false,
        Ok(_) => return Err(journal_invalid("is not a local regular file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => return Err(journal_io_error(path, "stat", error)),
    };
    let mut file = open_nofollow(path, true)?;
    ensure_same_leaf(&file, path)?;
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
        ensure_same_leaf(&file, path)?;
        file.set_len(durable_len)
            .map_err(|error| journal_io_error(path, "recover", error))?;
        file.sync_data()
            .map_err(|error| journal_io_error(path, "sync recovered tail", error))?;
    }
    ensure_same_leaf(&file, path)?;
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

fn open_nofollow(path: &Path, write: bool) -> Result<File, CutError> {
    let mut options = OpenOptions::new();
    options.read(true);
    if write {
        options.write(true).create(true);
    }
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
        .map_err(|error| journal_io_error(path, "open", error))?;
    let metadata = file
        .metadata()
        .map_err(|error| journal_io_error(path, "stat", error))?;
    if !plain_regular(&metadata) {
        return Err(journal_invalid("is not a local regular file"));
    }
    Ok(file)
}

fn ensure_same_leaf(file: &File, path: &Path) -> Result<(), CutError> {
    let current = open_nofollow(path, false)?;
    if same_open_file(file, &current)? {
        Ok(())
    } else {
        Err(journal_invalid("was replaced while open"))
    }
}

#[cfg(unix)]
fn same_open_file(left: &File, right: &File) -> Result<bool, CutError> {
    use std::os::unix::fs::MetadataExt;
    let left = left.metadata()?;
    let right = right.metadata()?;
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
}

#[cfg(windows)]
fn same_open_file(left: &File, right: &File) -> Result<bool, CutError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
    };
    fn identity(file: &File) -> Result<(u32, u32, u32), CutError> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok((
            info.dwVolumeSerialNumber,
            info.nFileIndexHigh,
            info.nFileIndexLow,
        ))
    }
    Ok(identity(left)? == identity(right)?)
}

#[cfg(not(any(unix, windows)))]
fn same_open_file(_left: &File, _right: &File) -> Result<bool, CutError> {
    Err(journal_invalid("file identity is unsupported"))
}

fn plain_regular(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_file() && !metadata.file_type().is_symlink() && !is_reparse(metadata)
}

#[cfg(windows)]
fn is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

#[cfg(not(windows))]
fn is_reparse(_metadata: &fs::Metadata) -> bool {
    false
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

//! Durable, fail-closed JSONL storage for one pause-aware recording session.
//!
//! This is deliberately separate from the v1 checkpoint manifest. A reader never
//! repairs, quarantines, or otherwise mutates this journal: an unterminated or
//! malformed line is not a completed recording transition.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::{
    is_plain_regular_file, CaptureRoot, DurableStateTransition, ManifestError,
    RecordingSessionIntent, RecordingSessionJournal, RecordingSessionJournalEntry, SealedRun,
    SessionJournalError, SessionTerminal, RECORDING_SESSION_JOURNAL_SCHEMA,
};

/// Fixed literal leaf used for the v1 pause-aware session journal. It is always
/// derived from a validated [`CaptureRoot`] and capture-id, never accepted from
/// a caller as a path.
pub const RECORDING_SESSION_JOURNAL_FILE: &str = "recording-session.journal.jsonl";

/// A durable writer plus its fully replayed recording-session state.
///
/// All mutation first validates a cloned state machine, then appends exactly one
/// canonical JSONL entry and syncs it. If any write or sync fails, the in-memory
/// state is left unchanged; a later reopen replays the file fail-closed.
#[derive(Debug)]
pub struct RecordingSessionJournalFile {
    root: CaptureRoot,
    capture_id: String,
    path: PathBuf,
    file: File,
    journal: RecordingSessionJournal,
}

impl RecordingSessionJournalFile {
    /// Create a new session journal in an already-created capture directory.
    /// Existing leaves, including links and non-regular files, are never
    /// replaced.
    pub fn create_new(
        root: &CaptureRoot,
        capture_id: &str,
        intent: RecordingSessionIntent,
    ) -> Result<Self, SessionJournalError> {
        let journal = RecordingSessionJournal::new(intent)?;
        let path = capture_path(root, capture_id)?;
        reject_existing_leaf(&path)?;
        let mut file = create_new_nofollow(&path)?;
        // `create_new` prevents replacement at open time; re-prove that its
        // fixed leaf still names this handle before the first durable append.
        ensure_same_leaf(&file, &path)?;
        append_canonical_entry(
            &mut file,
            &path,
            &RecordingSessionJournalEntry::Intent(Box::new(journal.intent().clone())),
        )?;

        Ok(Self {
            root: root.clone(),
            capture_id: capture_id.into(),
            path,
            file,
            journal,
        })
    }

    /// Replay a journal through a read-only handle without modifying its file or
    /// capture directory. Every line must be a canonical JSON record terminated
    /// by `\n`, except that a v1 intent may retain an originally written integer
    /// `fps`; a torn final line is an error even if its JSON payload would
    /// otherwise parse.
    pub fn replay(
        root: &CaptureRoot,
        capture_id: &str,
    ) -> Result<RecordingSessionJournal, SessionJournalError> {
        let path = capture_path(root, capture_id)?;
        let mut file = open_read_nofollow(&path)?;
        ensure_open_regular(&file, &path)?;
        RecordingSessionJournal::replay(read_exact_entries(&mut file, &path)?)
    }

    /// Reopen a completed-or-live journal for later appends. Opening itself does
    /// not write; callers that need a read-only replay should use [`Self::replay`]
    /// so the journal is never opened with append capability. Every line must be
    /// a canonical JSON record terminated by `\n`, except that a v1 intent may
    /// retain an originally written integer `fps`; a torn final line is an error
    /// even if its JSON payload would otherwise parse.
    pub fn open(root: &CaptureRoot, capture_id: &str) -> Result<Self, SessionJournalError> {
        let path = capture_path(root, capture_id)?;
        let mut file = open_read_append_nofollow(&path)?;
        ensure_open_regular(&file, &path)?;
        let journal = RecordingSessionJournal::replay(read_exact_entries(&mut file, &path)?)?;

        Ok(Self {
            root: root.clone(),
            capture_id: capture_id.into(),
            path,
            file,
            journal,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn journal(&self) -> &RecordingSessionJournal {
        &self.journal
    }

    pub fn append_transition(
        &mut self,
        transition: DurableStateTransition,
    ) -> Result<(), SessionJournalError> {
        self.append_entry(RecordingSessionJournalEntry::Transition(transition))
    }

    pub fn seal_run(&mut self, run: SealedRun) -> Result<(), SessionJournalError> {
        self.append_entry(RecordingSessionJournalEntry::Run(run))
    }

    pub fn seal_terminal(&mut self, terminal: SessionTerminal) -> Result<(), SessionJournalError> {
        self.append_entry(RecordingSessionJournalEntry::Terminal(terminal))
    }

    /// Validate and durably append exactly one non-intent journal entry. This
    /// lower-level form keeps a future recorder coordinator from bypassing the
    /// immutable-intent, single-terminal, and state-transition checks.
    pub fn append_entry(
        &mut self,
        entry: RecordingSessionJournalEntry,
    ) -> Result<(), SessionJournalError> {
        let mut next = self.journal.clone();
        match &entry {
            RecordingSessionJournalEntry::Intent(_) => {
                return Err(invalid("cannot append a duplicate immutable intent"))
            }
            RecordingSessionJournalEntry::Transition(transition) => {
                next.append_transition(transition.clone())?
            }
            RecordingSessionJournalEntry::InputSidecar(pin) => {
                next.append_input_sidecar(pin.clone())?
            }
            RecordingSessionJournalEntry::Run(run) => next.seal_run(run.clone())?,
            RecordingSessionJournalEntry::Terminal(terminal) => {
                next.seal_terminal(terminal.clone())?
            }
        }
        self.append_validated(entry, next)
    }

    fn append_validated(
        &mut self,
        entry: RecordingSessionJournalEntry,
        next: RecordingSessionJournal,
    ) -> Result<(), SessionJournalError> {
        let path = capture_path(&self.root, &self.capture_id)?;
        if path != self.path || !is_plain_regular_file(&path).map_err(manifest_error)? {
            return Err(invalid("recording session journal leaf is no longer local"));
        }
        ensure_same_leaf(&self.file, &path)?;
        append_canonical_entry(&mut self.file, &path, &entry)?;
        self.journal = next;
        Ok(())
    }
}

fn capture_path(root: &CaptureRoot, capture_id: &str) -> Result<PathBuf, SessionJournalError> {
    root.capture_file(capture_id, RECORDING_SESSION_JOURNAL_FILE)
        .map_err(manifest_error)
}

fn reject_existing_leaf(path: &Path) -> Result<(), SessionJournalError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(invalid(
            "refusing to replace an existing recording session journal",
        )),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error(path, source)),
    }
}

fn create_new_nofollow(path: &Path) -> Result<File, SessionJournalError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
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
    options.open(path).map_err(|source| io_error(path, source))
}

fn open_read_nofollow(path: &Path) -> Result<File, SessionJournalError> {
    open_existing_nofollow(path, false)
}

fn open_read_append_nofollow(path: &Path) -> Result<File, SessionJournalError> {
    open_existing_nofollow(path, true)
}

fn open_existing_nofollow(path: &Path, append: bool) -> Result<File, SessionJournalError> {
    if !is_plain_regular_file(path).map_err(manifest_error)? {
        return Err(invalid(
            "recording session journal is not a local regular file",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true).append(append);
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
    options.open(path).map_err(|source| io_error(path, source))
}

fn ensure_open_regular(file: &File, path: &Path) -> Result<(), SessionJournalError> {
    let metadata = file.metadata().map_err(|source| io_error(path, source))?;
    if metadata.file_type().is_file() && !is_reparse(&metadata) {
        Ok(())
    } else {
        Err(invalid(
            "opened recording session journal is not a local regular file",
        ))
    }
}

/// Prove the append handle still names the same local file as the fixed journal
/// leaf. Without this check, a same-user replacement by another regular file
/// could leave a writer appending to an unlinked old inode while reporting
/// success. CaptureRoot deliberately does not claim to defeat a replacement in
/// the small interval after this check; it rejects static redirection and this
/// check fails closed before every normal append.
fn ensure_same_leaf(file: &File, path: &Path) -> Result<(), SessionJournalError> {
    ensure_open_regular(file, path)?;
    let current = open_read_nofollow(path)?;
    ensure_open_regular(&current, path)?;
    if same_open_file(file, &current, path)? {
        Ok(())
    } else {
        Err(invalid(
            "recording session journal leaf was replaced since it was opened",
        ))
    }
}

#[cfg(unix)]
fn same_open_file(left: &File, right: &File, path: &Path) -> Result<bool, SessionJournalError> {
    use std::os::unix::fs::MetadataExt;

    let left = left.metadata().map_err(|source| io_error(path, source))?;
    let right = right.metadata().map_err(|source| io_error(path, source))?;
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
}

#[cfg(windows)]
fn same_open_file(left: &File, right: &File, path: &Path) -> Result<bool, SessionJournalError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
    };

    fn identity(file: &File, path: &Path) -> Result<(u32, u32, u32), SessionJournalError> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: `File::as_raw_handle` is a live Windows file handle for the
        // duration of this call and `info` is valid writable storage.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) } == 0 {
            return Err(io_error(path, std::io::Error::last_os_error()));
        }
        Ok((
            info.dwVolumeSerialNumber,
            info.nFileIndexHigh,
            info.nFileIndexLow,
        ))
    }

    Ok(identity(left, path)? == identity(right, path)?)
}

#[cfg(not(any(unix, windows)))]
fn same_open_file(_left: &File, _right: &File, _path: &Path) -> Result<bool, SessionJournalError> {
    Err(invalid(
        "recording session journal file identity is unsupported on this platform",
    ))
}

fn read_exact_entries(
    file: &mut File,
    path: &Path,
) -> Result<Vec<RecordingSessionJournalEntry>, SessionJournalError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|source| io_error(path, source))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|source| io_error(path, source))?;
    if !bytes.ends_with(b"\n") {
        return Err(invalid(
            "recording session journal has a torn final entry; read does not repair it",
        ));
    }

    let mut entries = Vec::new();
    for (index, line) in bytes[..bytes.len().saturating_sub(1)]
        .split(|byte| *byte == b'\n')
        .enumerate()
    {
        let line_no = index + 1;
        if line.is_empty() {
            return Err(invalid(format!(
                "recording session journal line {line_no} is empty"
            )));
        }
        let entry = serde_json::from_slice(line).map_err(|source| {
            invalid(format!(
                "recording session journal line {line_no} is malformed: {source}"
            ))
        })?;
        let canonical = serde_json::to_vec(&entry).map_err(|source| {
            invalid(format!(
                "recording session journal line {line_no} cannot be canonicalized: {source}"
            ))
        })?;
        if canonical != line
            && legacy_integer_fps_intent_canonical_bytes(&entry, &canonical).as_deref()
                != Some(line)
        {
            return Err(invalid(format!(
                "recording session journal line {line_no} is not canonical"
            )));
        }
        entries.push(entry);
    }
    Ok(entries)
}

/// Journal v1 originally wrote `fps` as an integer. Serde now canonicalizes its
/// exact `f64` equivalent with `.0`, so accept only that byte-for-byte legacy
/// spelling for a valid v1 intent. All other JSON remains canonical.
fn legacy_integer_fps_intent_canonical_bytes(
    entry: &RecordingSessionJournalEntry,
    canonical: &[u8],
) -> Option<Vec<u8>> {
    let RecordingSessionJournalEntry::Intent(intent) = entry else {
        return None;
    };
    if intent.schema != RECORDING_SESSION_JOURNAL_SCHEMA
        || !intent.fps.is_finite()
        || !(1.0..=240.0).contains(&intent.fps)
    {
        return None;
    }
    let integer_fps = intent.fps as u32;
    if f64::from(integer_fps) != intent.fps {
        return None;
    }

    let canonical_fps = serde_json::to_vec(&intent.fps).ok()?;
    let mut canonical_field = b"\"fps\":".to_vec();
    canonical_field.extend(canonical_fps);
    let field_start = canonical
        .windows(canonical_field.len())
        .position(|window| window == canonical_field)?;
    let mut legacy_field = b"\"fps\":".to_vec();
    legacy_field.extend(serde_json::to_vec(&integer_fps).ok()?);

    let mut legacy =
        Vec::with_capacity(canonical.len() - canonical_field.len() + legacy_field.len());
    legacy.extend_from_slice(&canonical[..field_start]);
    legacy.extend(legacy_field);
    legacy.extend_from_slice(&canonical[field_start + canonical_field.len()..]);
    Some(legacy)
}

fn append_canonical_entry(
    file: &mut File,
    path: &Path,
    entry: &RecordingSessionJournalEntry,
) -> Result<(), SessionJournalError> {
    let bytes = serde_json::to_vec(entry).map_err(|source| {
        invalid(format!(
            "could not serialize recording session entry: {source}"
        ))
    })?;
    file.write_all(&bytes)
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all())
        .map_err(|source| io_error(path, source))?;
    sync_parent(path)
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<(), SessionJournalError> {
    File::open(path.parent().unwrap_or(Path::new(".")))
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error(path, source))
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> Result<(), SessionJournalError> {
    Ok(())
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

fn manifest_error(error: ManifestError) -> SessionJournalError {
    invalid(format!(
        "unsafe recording session journal location: {error}"
    ))
}

fn io_error(path: &Path, source: std::io::Error) -> SessionJournalError {
    invalid(format!(
        "recording session journal I/O at {}: {source}",
        path.display()
    ))
}

fn invalid(detail: impl Into<String>) -> SessionJournalError {
    SessionJournalError::Invalid(detail.into())
}

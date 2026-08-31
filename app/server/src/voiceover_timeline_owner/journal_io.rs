//! Canonical, synced, no-follow storage for private voiceover take journals.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use cut_core::{error_codes, CutError};
use record_recovery::CaptureRoot;

use super::journal::{VoiceoverTakeJournalEntry, VoiceoverTakeJournalState};

pub(super) const VOICEOVER_TAKE_JOURNAL_FILE: &str = ".voiceover-take.jsonl";

/// One fixed, private journal leaf under a validated project capture directory.
/// Each append validates a cloned state before it reaches disk, writes canonical
/// JSONL, and synchronizes both the leaf and its parent directory.
#[derive(Debug)]
pub(super) struct VoiceoverTakeJournalFile {
    root: CaptureRoot,
    take_id: String,
    path: PathBuf,
    file: File,
    state: VoiceoverTakeJournalState,
    #[cfg(test)]
    fail_next_append: bool,
}

impl VoiceoverTakeJournalFile {
    pub(super) fn create_new(
        root: &CaptureRoot,
        take_id: &str,
        state: VoiceoverTakeJournalState,
    ) -> Result<Self, CutError> {
        let path = journal_path(root, take_id)?;
        reject_existing_leaf(&path)?;
        let mut file = create_new_nofollow(&path)?;
        ensure_same_leaf(&file, &path)?;
        append_canonical_entry(
            &mut file,
            &path,
            &VoiceoverTakeJournalEntry::Intent {
                intent: state.intent.clone(),
            },
        )?;
        Ok(Self {
            root: root.clone(),
            take_id: take_id.into(),
            path,
            file,
            state,
            #[cfg(test)]
            fail_next_append: false,
        })
    }

    pub(super) fn open(root: &CaptureRoot, take_id: &str) -> Result<Self, CutError> {
        let path = journal_path(root, take_id)?;
        let mut file = open_existing_nofollow(&path, true)?;
        ensure_open_regular(&file)?;
        let state = VoiceoverTakeJournalState::replay(read_exact_entries(&mut file)?)?;
        Ok(Self {
            root: root.clone(),
            take_id: take_id.into(),
            path,
            file,
            state,
            #[cfg(test)]
            fail_next_append: false,
        })
    }

    pub(super) fn state(&self) -> &VoiceoverTakeJournalState {
        &self.state
    }

    pub(super) fn append(&mut self, entry: VoiceoverTakeJournalEntry) -> Result<(), CutError> {
        let mut next = self.state.clone();
        next.apply(entry.clone())?;
        let path = journal_path(&self.root, &self.take_id)?;
        if path != self.path || !is_plain_regular_file(&path)? {
            return Err(io_error("voiceover journal leaf is no longer local"));
        }
        ensure_same_leaf(&self.file, &path)?;
        #[cfg(test)]
        if std::mem::replace(&mut self.fail_next_append, false) {
            return Err(io_error("injected voiceover journal append failure"));
        }
        append_canonical_entry(&mut self.file, &path, &entry)?;
        self.state = next;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn fail_next_append_for_test(&mut self) {
        self.fail_next_append = true;
    }
}

fn journal_path(root: &CaptureRoot, take_id: &str) -> Result<PathBuf, CutError> {
    root.capture_file(take_id, VOICEOVER_TAKE_JOURNAL_FILE)
        .map_err(|_| io_error("derive private voiceover journal path"))
}

fn reject_existing_leaf(path: &Path) -> Result<(), CutError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(io_error(
            "refusing to replace an existing voiceover journal",
        )),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(io_error("inspect private voiceover journal leaf")),
    }
}

fn create_new_nofollow(path: &Path) -> Result<File, CutError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    nofollow(&mut options);
    options
        .open(path)
        .map_err(|_| io_error("create private voiceover journal"))
}

fn open_existing_nofollow(path: &Path, append: bool) -> Result<File, CutError> {
    if !is_plain_regular_file(path)? {
        return Err(io_error(
            "private voiceover journal is not a regular local file",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true).append(append);
    nofollow(&mut options);
    options
        .open(path)
        .map_err(|_| io_error("open private voiceover journal"))
}

#[cfg(unix)]
fn nofollow(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options.custom_flags(libc::O_NOFOLLOW);
}

#[cfg(windows)]
fn nofollow(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;

    options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
}

#[cfg(not(any(unix, windows)))]
fn nofollow(_options: &mut OpenOptions) {}

fn ensure_same_leaf(file: &File, path: &Path) -> Result<(), CutError> {
    ensure_open_regular(file)?;
    let current = open_existing_nofollow(path, false)?;
    ensure_open_regular(&current)?;
    if same_open_file(file, &current)? {
        Ok(())
    } else {
        Err(io_error(
            "private voiceover journal was replaced while open",
        ))
    }
}

fn ensure_open_regular(file: &File) -> Result<(), CutError> {
    let metadata = file
        .metadata()
        .map_err(|_| io_error("inspect private voiceover journal"))?;
    if metadata.file_type().is_file() && !is_reparse(&metadata) {
        Ok(())
    } else {
        Err(io_error(
            "private voiceover journal is not a local regular file",
        ))
    }
}

#[cfg(unix)]
fn same_open_file(left: &File, right: &File) -> Result<bool, CutError> {
    use std::os::unix::fs::MetadataExt;

    let left = left
        .metadata()
        .map_err(|_| io_error("inspect private voiceover journal"))?;
    let right = right
        .metadata()
        .map_err(|_| io_error("inspect private voiceover journal"))?;
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
        // SAFETY: the handle is live for this call and `info` is writable.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) } == 0 {
            return Err(io_error("inspect private voiceover journal"));
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
    Err(io_error(
        "private voiceover journal identity is unsupported",
    ))
}

fn read_exact_entries(file: &mut File) -> Result<Vec<VoiceoverTakeJournalEntry>, CutError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| io_error("read private voiceover journal"))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| io_error("read private voiceover journal"))?;
    if !bytes.ends_with(b"\n") {
        return Err(io_error("private voiceover journal has a torn final entry"));
    }
    bytes[..bytes.len().saturating_sub(1)]
        .split(|byte| *byte == b'\n')
        .enumerate()
        .map(|(index, line)| {
            if line.is_empty() {
                return Err(io_error(
                    "private voiceover journal contains an empty entry",
                ));
            }
            let entry = serde_json::from_slice(line)
                .map_err(|_| io_error("private voiceover journal entry is malformed"))?;
            let canonical = serde_json::to_vec(&entry)
                .map_err(|_| io_error("canonicalize private voiceover journal entry"))?;
            (canonical == line).then_some(entry).ok_or_else(|| {
                io_error(format!(
                    "private voiceover journal entry {} is not canonical",
                    index + 1
                ))
            })
        })
        .collect()
}

fn append_canonical_entry(
    file: &mut File,
    path: &Path,
    entry: &VoiceoverTakeJournalEntry,
) -> Result<(), CutError> {
    let bytes = serde_json::to_vec(entry)
        .map_err(|_| io_error("serialize private voiceover journal entry"))?;
    file.write_all(&bytes)
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all())
        .map_err(|_| io_error("sync private voiceover journal entry"))?;
    sync_parent(path)
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<(), CutError> {
    File::open(path.parent().unwrap_or(Path::new(".")))
        .and_then(|directory| directory.sync_all())
        .map_err(|_| io_error("sync private voiceover journal directory"))
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> Result<(), CutError> {
    Ok(())
}

fn is_plain_regular_file(path: &Path) -> Result<bool, CutError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| io_error("inspect private voiceover journal leaf"))?;
    Ok(metadata.file_type().is_file()
        && !metadata.file_type().is_symlink()
        && !is_reparse(&metadata))
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

fn io_error(detail: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::IO,
        "voiceover private journal I/O failed",
        detail.into(),
    )
}

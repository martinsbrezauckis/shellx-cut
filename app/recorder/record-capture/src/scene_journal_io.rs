//! Contained file operations for the durable generic scene-journal owner.

#[cfg(not(windows))]
use std::fs::OpenOptions;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
#[cfg(not(windows))]
use record_recovery::is_plain_regular_file;
use record_recovery::CaptureRoot;

use crate::scene_journal::{
    SceneJournalEntry, SceneJournalError, SceneJournalReplay, SceneJournalResult,
    MAX_SCENE_JOURNAL_BYTES, SCENE_JOURNAL_FILE,
};
use crate::scene_journal_parse::{parse_entries, torn_tail_is_unambiguously_incomplete};
use crate::scene_journal_test_hooks::{trigger, SceneJournalIoHook};

pub(super) fn capture_path(root: &CaptureRoot, capture_id: &str) -> SceneJournalResult<PathBuf> {
    root.capture_file(capture_id, SCENE_JOURNAL_FILE)
        .map_err(containment_error)
}

pub(super) fn reject_existing_leaf(path: &Path) -> SceneJournalResult<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(invalid("refusing to replace an existing scene journal")),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error(path, source)),
    }
}

pub(super) fn create_new_nofollow(path: &Path) -> SceneJournalResult<File> {
    #[cfg(windows)]
    {
        return crate::scene_journal_windows::open_windows_anchored(path, true, true);
    }
    #[cfg(not(windows))]
    {
        let mut options = OpenOptions::new();
        options.read(true).append(true).create_new(true);
        nofollow(&mut options);
        options.open(path).map_err(|source| io_error(path, source))
    }
}

/// Unix owns parent-directory fsync. Windows owns a non-reparse parent handle,
/// no-replace/write-through leaf, and FlushFileBuffers acknowledgement.
pub(super) fn require_parent_durability() -> SceneJournalResult<()> {
    #[cfg(any(unix, windows))]
    {
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(invalid(
            "scene journal is unavailable without a durable namespace barrier",
        ))
    }
}

pub(super) fn open_existing_nofollow(path: &Path, append: bool) -> SceneJournalResult<File> {
    #[cfg(windows)]
    {
        return crate::scene_journal_windows::open_windows_anchored(path, false, append);
    }
    #[cfg(not(windows))]
    {
        if !is_plain_regular_file(path).map_err(containment_error)? {
            return Err(invalid("scene journal is not a local regular file"));
        }
        let mut options = OpenOptions::new();
        options.read(true).append(append);
        nofollow(&mut options);
        options.open(path).map_err(|source| io_error(path, source))
    }
}

pub(super) fn lock_owner(file: &File, path: &Path) -> SceneJournalResult<()> {
    file.try_lock_exclusive()
        .map_err(|source| io_error(path, source))
}

pub(super) fn read_replay(
    file: &mut File,
    path: &Path,
    repair_torn_tail: bool,
) -> SceneJournalResult<SceneJournalReplay> {
    let len = file
        .metadata()
        .map_err(|source| io_error(path, source))?
        .len();
    if len > MAX_SCENE_JOURNAL_BYTES {
        return Err(invalid("scene journal exceeds its byte limit"));
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|source| io_error(path, source))?;
    let capacity = usize::try_from(len).map_err(|_| invalid("scene journal length overflowed"))?;
    let mut bytes = Vec::with_capacity(capacity);
    Read::by_ref(file)
        .take(MAX_SCENE_JOURNAL_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| io_error(path, source))?;
    trigger(SceneJournalIoHook::GrowDuringRead, path)?;
    trigger(SceneJournalIoHook::SwapLeafDuringRead, path)?;
    let final_len = file
        .metadata()
        .map_err(|source| io_error(path, source))?
        .len();
    if bytes.len() != capacity || final_len != len {
        return Err(invalid("scene journal changed while it was being read"));
    }
    ensure_same_leaf(file, path)?;
    if bytes.ends_with(b"\n") {
        return parse_entries(&bytes);
    }
    let Some(last_newline) = bytes.iter().rposition(|byte| *byte == b'\n') else {
        return Err(invalid(
            "scene journal has no complete header before its torn tail",
        ));
    };
    let retained = last_newline + 1;
    let replay = parse_entries(&bytes[..retained])?;
    if !torn_tail_is_unambiguously_incomplete(&bytes[retained..]) {
        return Err(invalid(
            "scene journal final bytes are not an unambiguously incomplete record",
        ));
    }
    if !repair_torn_tail {
        return Err(invalid(
            "scene journal has a recoverable torn tail but no exclusive repair owner",
        ));
    }
    require_parent_durability()?;
    ensure_same_leaf(file, path)?;
    file.set_len(
        u64::try_from(retained).map_err(|_| invalid("scene journal recovery length overflowed"))?,
    )
    .map_err(|source| io_error(path, source))?;
    verify_durable(file, path)?;
    ensure_same_leaf(file, path)?;
    Ok(replay)
}

pub(crate) fn canonical_bytes(entry: &SceneJournalEntry) -> SceneJournalResult<Vec<u8>> {
    serde_json::to_vec(entry)
        .map_err(|source| invalid(format!("serialize scene journal entry: {source}")))
}

pub(super) fn append_canonical(
    file: &mut File,
    path: &Path,
    entry: &SceneJournalEntry,
) -> SceneJournalResult<()> {
    append_bytes(file, path, &canonical_bytes(entry)?)
}

pub(super) fn append_bytes(file: &mut File, path: &Path, bytes: &[u8]) -> SceneJournalResult<()> {
    // Under the exclusive owner lock, replay/torn-tail repair may have moved
    // the cursor. Seek canonical EOF before every append, including Windows
    // GENERIC_WRITE handles, so no event can overwrite the header or gap.
    file.seek(SeekFrom::End(0))
        .map_err(|source| io_error(path, source))?;
    file.write_all(bytes)
        .and_then(|()| file.write_all(b"\n"))
        .map_err(|source| io_error(path, source))?;
    trigger(SceneJournalIoHook::SwapLeafAfterWrite, path)?;
    trigger(SceneJournalIoHook::PostWriteBeforeFileSyncFailure, path)?;
    verify_durable(file, path)
}

pub(super) fn verify_durable(file: &File, path: &Path) -> SceneJournalResult<()> {
    require_parent_durability()?;
    trigger(SceneJournalIoHook::FileSyncFailure, path)?;
    sync_journal_file(file, path)?;
    trigger(SceneJournalIoHook::ParentSyncFailure, path)?;
    sync_parent(path)
}

pub(super) fn ensure_same_leaf(file: &File, path: &Path) -> SceneJournalResult<()> {
    ensure_open_regular(file, path)?;
    let current = open_existing_nofollow(path, false)?;
    ensure_open_regular(&current, path)?;
    same_open_file(file, &current, path)?
        .then_some(())
        .ok_or_else(|| invalid("scene journal leaf was replaced since its owner opened it"))
}

/// Retain a failed new leaf: a path unlink cannot atomically prove it still
/// names this owned inode/handle, so removal could erase a replacement.
pub(super) fn preserve_failed_new_leaf(file: &File, path: &Path) -> SceneJournalResult<()> {
    let _ = ensure_same_leaf(file, path);
    trigger(SceneJournalIoHook::SwapLeafAfterCleanupProof, path)
}

pub(crate) fn ensure_open_regular(file: &File, path: &Path) -> SceneJournalResult<()> {
    let metadata = file.metadata().map_err(|source| io_error(path, source))?;
    (metadata.file_type().is_file() && !is_reparse(&metadata))
        .then_some(())
        .ok_or_else(|| invalid("opened scene journal is not a local regular file"))
}

pub(crate) fn io_error(path: &Path, source: std::io::Error) -> SceneJournalError {
    invalid(format!("scene journal I/O at {}: {source}", path.display()))
}

pub(crate) fn invalid(detail: impl Into<String>) -> SceneJournalError {
    SceneJournalError::Invalid(detail.into())
}

pub(super) fn containment_error(error: record_recovery::ManifestError) -> SceneJournalError {
    invalid(format!("unsafe scene journal location: {error}"))
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> SceneJournalResult<()> {
    File::open(path.parent().unwrap_or(Path::new(".")))
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error(path, source))
}

#[cfg(windows)]
fn sync_parent(path: &Path) -> SceneJournalResult<()> {
    crate::scene_journal_windows::sync_parent(path)
}

#[cfg(not(any(unix, windows)))]
fn sync_parent(_path: &Path) -> SceneJournalResult<()> {
    Err(invalid(
        "scene journal is unavailable without a durable namespace barrier",
    ))
}

#[cfg(unix)]
fn nofollow(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(libc::O_NOFOLLOW);
}

#[cfg(not(any(unix, windows)))]
fn nofollow(_options: &mut OpenOptions) {}

#[cfg(not(windows))]
fn sync_journal_file(file: &File, path: &Path) -> SceneJournalResult<()> {
    file.sync_all().map_err(|source| io_error(path, source))
}

#[cfg(windows)]
fn sync_journal_file(file: &File, path: &Path) -> SceneJournalResult<()> {
    crate::scene_journal_windows::sync_journal_file(file, path)
}

#[cfg(unix)]
fn same_open_file(left: &File, right: &File, path: &Path) -> SceneJournalResult<bool> {
    use std::os::unix::fs::MetadataExt;
    let left = left.metadata().map_err(|source| io_error(path, source))?;
    let right = right.metadata().map_err(|source| io_error(path, source))?;
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
}

#[cfg(windows)]
fn same_open_file(left: &File, right: &File, path: &Path) -> SceneJournalResult<bool> {
    crate::scene_journal_windows::same_open_file(left, right, path)
}

#[cfg(not(any(unix, windows)))]
fn same_open_file(_left: &File, _right: &File, _path: &Path) -> SceneJournalResult<bool> {
    Err(invalid(
        "scene journal file identity is unsupported on this platform",
    ))
}

#[cfg(windows)]
fn is_reparse(metadata: &fs::Metadata) -> bool {
    crate::scene_journal_windows::is_reparse(metadata)
}

#[cfg(not(windows))]
fn is_reparse(_metadata: &fs::Metadata) -> bool {
    false
}

//! Private crash-safe JSONL ownership for accepted recorder scenes.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use record_core::{reduce_scene_event, AcceptedStartSnapshot, SceneError, SceneEvent, SceneState};
use record_recovery::{is_plain_regular_file, CaptureRoot};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::scene_journal_io::{
    append_bytes, append_canonical, canonical_bytes, capture_path, containment_error,
    create_new_nofollow, ensure_open_regular, ensure_same_leaf, invalid, io_error, lock_owner,
    open_existing_nofollow, preserve_failed_new_leaf, read_replay, reject_existing_leaf,
    require_parent_durability, verify_durable,
};
use crate::scene_journal_test_hooks::{trigger, SceneJournalIoHook};

pub(crate) const SCENE_JOURNAL_FILE: &str = "recorder-scenes.journal.jsonl";
pub(crate) const SCENE_JOURNAL_SCHEMA: &str = "shellx-record/recorder-scenes-journal@1";
pub(crate) const MAX_SCENE_JOURNAL_BYTES: u64 = 128 * 1024;
pub(crate) const MAX_SCENE_JOURNAL_EVENTS: usize = 128;

pub(super) type SceneJournalResult<T> = Result<T, SceneJournalError>;

#[derive(Debug)]
pub(super) enum SceneJournalError {
    Scene(SceneError),
    Invalid(String),
}

impl From<SceneError> for SceneJournalError {
    fn from(error: SceneError) -> Self {
        Self::Scene(error)
    }
}

impl std::fmt::Display for SceneJournalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Scene(error) => error.fmt(formatter),
            Self::Invalid(detail) => formatter.write_str(detail),
        }
    }
}

impl std::error::Error for SceneJournalError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SceneJournalReplay {
    pub(super) snapshot: AcceptedStartSnapshot,
    pub(super) events: Vec<SceneEvent>,
    pub(super) state: SceneState,
}

impl SceneJournalReplay {
    pub(super) fn initial(snapshot: AcceptedStartSnapshot) -> SceneJournalResult<Self> {
        Ok(Self {
            state: SceneState::initial(&snapshot)?,
            snapshot,
            events: Vec::new(),
        })
    }

    pub(crate) fn snapshot(&self) -> &AcceptedStartSnapshot {
        &self.snapshot
    }

    pub(crate) fn events(&self) -> &[SceneEvent] {
        &self.events
    }

    pub(crate) fn state(&self) -> &SceneState {
        &self.state
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "entry", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum SceneJournalEntry {
    Header {
        schema: String,
        snapshot: AcceptedStartSnapshot,
    },
    Event {
        event: SceneEvent,
    },
}

/// The only append owner for a capture's fixed private scene-journal leaf.
#[derive(Debug)]
pub(crate) struct SceneJournalOwner {
    root: CaptureRoot,
    capture_id: String,
    path: PathBuf,
    file: File,
    replay: SceneJournalReplay,
}

impl SceneJournalOwner {
    pub(crate) fn create_new(
        root: &CaptureRoot,
        capture_id: &str,
        snapshot: AcceptedStartSnapshot,
    ) -> SceneJournalResult<Self> {
        require_parent_durability()?;
        let replay = SceneJournalReplay::initial(snapshot)?;
        let path = capture_path(root, capture_id)?;
        reject_existing_leaf(&path)?;
        let mut file = create_new_nofollow(&path)?;
        let initialized = (|| {
            trigger(SceneJournalIoHook::CreateLockFailure, &path)?;
            lock_owner(&file, &path)?;
            ensure_same_leaf(&file, &path)?;
            append_canonical(
                &mut file,
                &path,
                &SceneJournalEntry::Header {
                    schema: SCENE_JOURNAL_SCHEMA.into(),
                    snapshot: replay.snapshot.clone(),
                },
            )?;
            verify_capture_leaf(root, capture_id, &path, &file)?;
            verify_durable(&file, &path)?;
            verify_capture_leaf(root, capture_id, &path, &file)
        })();
        if let Err(error) = initialized {
            preserve_failed_new_leaf(&file, &path)?;
            return Err(error);
        }
        Ok(Self {
            root: root.clone(),
            capture_id: capture_id.into(),
            path,
            file,
            replay,
        })
    }

    /// Read only: a torn tail remains an error because this call owns no repair lock.
    pub(crate) fn replay(
        root: &CaptureRoot,
        capture_id: &str,
    ) -> SceneJournalResult<SceneJournalReplay> {
        let path = capture_path(root, capture_id)?;
        let mut file = open_existing_nofollow(&path, false)?;
        ensure_open_regular(&file, &path)?;
        verify_capture_leaf(root, capture_id, &path, &file)?;
        let replay = read_replay(&mut file, &path, false)?;
        verify_capture_leaf(root, capture_id, &path, &file)?;
        Ok(replay)
    }

    /// Reopen and re-acknowledge the leaf as its exclusive durable owner. Only
    /// a final unterminated record after a fully valid prefix is discarded; any
    /// malformed committed line stays untouched and fails closed.
    pub(crate) fn open(root: &CaptureRoot, capture_id: &str) -> SceneJournalResult<Self> {
        require_parent_durability()?;
        let path = capture_path(root, capture_id)?;
        let mut file = open_existing_nofollow(&path, true)?;
        lock_owner(&file, &path)?;
        let replay = (|| {
            verify_capture_leaf(root, capture_id, &path, &file)?;
            let replay = read_replay(&mut file, &path, true)?;
            verify_capture_leaf(root, capture_id, &path, &file)?;
            trigger(SceneJournalIoHook::PostWriteBeforeFileSyncFailure, &path)?;
            verify_durable(&file, &path)?;
            verify_capture_leaf(root, capture_id, &path, &file)?;
            Ok(replay)
        })();
        match replay {
            Ok(replay) => Ok(Self {
                root: root.clone(),
                capture_id: capture_id.into(),
                path,
                file,
                replay,
            }),
            Err(error) => {
                // The owner does not exist yet, so its Drop cannot release a
                // lock acquired before replay/durability validation failed.
                let _ = fs2::FileExt::unlock(&file);
                Err(error)
            }
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn replayed(&self) -> &SceneJournalReplay {
        &self.replay
    }

    /// Return the digest of the exact durable journal leaf currently owned by
    /// this coordinator. A separate no-follow handle is rechecked against the
    /// owner before and after its bounded read, so a path replacement cannot
    /// silently substitute bytes into the terminal projection.
    pub(crate) fn durable_sha256(&self) -> SceneJournalResult<String> {
        self.confirm_durability()?;
        // Windows' exclusive owner lock deliberately prevents a separately
        // opened handle from reading the journal, including another handle in
        // this process. Digest through a duplicated owner handle instead; the
        // path identity checks immediately before and after the bounded read
        // still reject a replaced leaf.
        let mut digest_file = self
            .file
            .try_clone()
            .map_err(|source| io_error(&self.path, source))?;
        ensure_open_regular(&digest_file, &self.path)?;
        ensure_same_leaf(&self.file, &self.path)?;
        let len = digest_file
            .metadata()
            .map_err(|source| io_error(&self.path, source))?
            .len();
        if len > MAX_SCENE_JOURNAL_BYTES {
            return Err(invalid("scene journal exceeds its byte limit"));
        }
        let mut bytes = Vec::with_capacity(
            usize::try_from(len).map_err(|_| invalid("scene journal length overflowed"))?,
        );
        digest_file
            .seek(SeekFrom::Start(0))
            .map_err(|source| io_error(&self.path, source))?;
        digest_file
            .by_ref()
            .take(MAX_SCENE_JOURNAL_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|source| io_error(&self.path, source))?;
        let final_len = digest_file
            .metadata()
            .map_err(|source| io_error(&self.path, source))?
            .len();
        if u64::try_from(bytes.len()).ok() != Some(len) || final_len != len {
            return Err(invalid("scene journal changed while it was being digested"));
        }
        ensure_same_leaf(&self.file, &self.path)?;
        self.revalidate_leaf()?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    /// Append one reducer-validated event, then sync it before reporting success.
    /// Repeating an exact already-durable event is a no-op, which makes retry and
    /// reopen safe when a previous writer observed an ambiguous I/O failure.
    pub(crate) fn append_event(&mut self, event: SceneEvent) -> SceneJournalResult<()> {
        self.replay = self.refresh()?;
        if let Some(existing) = self
            .replay
            .events
            .iter()
            .find(|existing| existing.sequence == event.sequence)
        {
            if existing != &event {
                return Err(invalid(
                    "scene event retries must match the durable event exactly",
                ));
            }
            self.confirm_durability()?;
            return Ok(());
        }
        let next_state = reduce_scene_event(&self.replay.snapshot, &self.replay.state, &event)?;
        let entry = SceneJournalEntry::Event {
            event: event.clone(),
        };
        let bytes = canonical_bytes(&entry)?;
        let current_len = self
            .file
            .metadata()
            .map_err(|source| io_error(&self.path, source))?
            .len();
        let appended_len = u64::try_from(bytes.len() + 1)
            .map_err(|_| invalid("scene journal entry length overflowed"))?;
        if self.replay.events.len() >= MAX_SCENE_JOURNAL_EVENTS
            || current_len.saturating_add(appended_len) > MAX_SCENE_JOURNAL_BYTES
        {
            return Err(invalid("scene journal limit exceeded"));
        }
        self.revalidate_leaf()?;
        trigger(SceneJournalIoHook::SwapLeafAfterValidation, &self.path)?;
        append_bytes(&mut self.file, &self.path, &bytes)?;
        trigger(SceneJournalIoHook::SwapLeafAfterSync, &self.path)?;
        trigger(
            SceneJournalIoHook::SwapCaptureDirectoryAfterSync,
            &self.path,
        )?;
        self.confirm_durability()?;
        self.replay.events.push(event);
        self.replay.state = next_state;
        Ok(())
    }

    fn refresh(&mut self) -> SceneJournalResult<SceneJournalReplay> {
        self.revalidate_leaf()?;
        read_replay(&mut self.file, &self.path, true)
    }

    fn revalidate_leaf(&self) -> SceneJournalResult<()> {
        verify_capture_leaf(&self.root, &self.capture_id, &self.path, &self.file)
    }

    fn confirm_durability(&self) -> SceneJournalResult<()> {
        self.revalidate_leaf()?;
        verify_durable(&self.file, &self.path)?;
        self.revalidate_leaf()
    }
}

impl Drop for SceneJournalOwner {
    fn drop(&mut self) {
        // Closing releases the advisory lock too, but an explicit unlock makes
        // immediate same-process recovery deterministic under parallel tests.
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

fn verify_capture_leaf(
    root: &CaptureRoot,
    capture_id: &str,
    path: &Path,
    file: &File,
) -> SceneJournalResult<()> {
    let current = capture_path(root, capture_id)?;
    if current != path || !is_plain_regular_file(&current).map_err(containment_error)? {
        return Err(invalid(
            "scene journal leaf is no longer a local regular file",
        ));
    }
    ensure_same_leaf(file, &current)
}

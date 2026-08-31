//! Test-only fault and replacement seams for private scene-journal durability.

use std::path::Path;

#[cfg(test)]
use crate::scene_journal::SceneJournalError;
use crate::scene_journal::SceneJournalResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SceneJournalIoHook {
    CreateLockFailure,
    PostWriteBeforeFileSyncFailure,
    FileSyncFailure,
    ParentSyncFailure,
    SwapLeafAfterValidation,
    SwapLeafAfterWrite,
    SwapLeafAfterSync,
    SwapCaptureDirectoryAfterSync,
    GrowDuringRead,
    SwapLeafDuringRead,
    SwapLeafAfterCleanupProof,
}

pub(super) fn trigger(hook: SceneJournalIoHook, path: &Path) -> SceneJournalResult<()> {
    #[cfg(test)]
    {
        test_trigger(hook, path)
    }
    #[cfg(not(test))]
    {
        let _ = (hook, path);
        Ok(())
    }
}

#[cfg(test)]
thread_local! {
    static NEXT_HOOKS: std::cell::RefCell<Vec<SceneJournalIoHook>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(all(test, unix))]
pub(super) fn inject_once(hook: SceneJournalIoHook) {
    inject_sequence(&[hook]);
}

#[cfg(all(test, unix))]
pub(super) fn inject_sequence(hooks: &[SceneJournalIoHook]) {
    NEXT_HOOKS.with(|next| *next.borrow_mut() = hooks.to_vec());
}

#[cfg(test)]
fn test_trigger(hook: SceneJournalIoHook, path: &Path) -> SceneJournalResult<()> {
    let active = NEXT_HOOKS.with(|next| {
        let mut next = next.borrow_mut();
        if next.first() == Some(&hook) {
            Some(next.remove(0))
        } else {
            None
        }
    });
    if active.is_none() {
        return Ok(());
    }
    match hook {
        SceneJournalIoHook::CreateLockFailure
        | SceneJournalIoHook::PostWriteBeforeFileSyncFailure
        | SceneJournalIoHook::FileSyncFailure
        | SceneJournalIoHook::ParentSyncFailure => Err(SceneJournalError::Invalid(format!(
            "injected scene journal {hook:?}"
        ))),
        SceneJournalIoHook::SwapLeafAfterValidation
        | SceneJournalIoHook::SwapLeafAfterWrite
        | SceneJournalIoHook::SwapLeafAfterSync
        | SceneJournalIoHook::SwapLeafDuringRead
        | SceneJournalIoHook::SwapLeafAfterCleanupProof => replace_leaf(path),
        SceneJournalIoHook::SwapCaptureDirectoryAfterSync => replace_capture_directory(path),
        SceneJournalIoHook::GrowDuringRead => std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .and_then(|mut file| std::io::Write::write_all(&mut file, b"x"))
            .map_err(|error| {
                SceneJournalError::Invalid(format!("inject scene read growth: {error}"))
            }),
    }
}

#[cfg(test)]
fn replace_leaf(path: &Path) -> SceneJournalResult<()> {
    let replacement = path.with_extension("scene-journal-replacement");
    std::fs::write(&replacement, b"replacement remains authoritative")
        .and_then(|()| std::fs::rename(&replacement, path))
        .map_err(|error| {
            SceneJournalError::Invalid(format!("inject scene leaf replacement: {error}"))
        })
}

#[cfg(test)]
fn replace_capture_directory(path: &Path) -> SceneJournalResult<()> {
    let capture_dir = path.parent().ok_or_else(|| {
        SceneJournalError::Invalid("inject scene capture-directory replacement: no parent".into())
    })?;
    let parked = capture_dir.with_extension("scene-journal-old");
    std::fs::rename(capture_dir, &parked)
        .and_then(|()| std::fs::create_dir(capture_dir))
        .and_then(|()| std::fs::write(path, b"replacement remains authoritative"))
        .map_err(|error| {
            SceneJournalError::Invalid(format!(
                "inject scene capture-directory replacement: {error}"
            ))
        })
}

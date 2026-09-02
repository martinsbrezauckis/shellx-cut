//! State transitions, revocation, and bounded playback retention.

use super::media::media_relative_in_root;
use super::state::{
    self, ReadyTake, RehearsalOwner, TakeState, EXPIRED_CLEANUP_RETRY_DELAY,
    MAX_EXPIRED_CLEANUP_RETRIES, READY_TAKE_TTL,
};
use super::valid_handle;
use crate::screen_record::CaptureSessionControl;
use cut_core::{error_codes, CutError};
use std::path::{Path, PathBuf};
#[cfg(not(test))]
fn remove_owned_root(path: &Path) -> std::io::Result<()> {
    std::fs::remove_dir_all(path)
}
#[cfg(test)]
fn remove_owned_root(path: &Path) -> std::io::Result<()> {
    let hook = *remove_hook()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    hook.map_or_else(|| std::fs::remove_dir_all(path), |remove| remove(path))
}
#[cfg(test)]
fn remove_hook() -> &'static std::sync::Mutex<Option<fn(&Path) -> std::io::Result<()>>> {
    static HOOK: std::sync::OnceLock<std::sync::Mutex<Option<fn(&Path) -> std::io::Result<()>>>> =
        std::sync::OnceLock::new();
    HOOK.get_or_init(|| std::sync::Mutex::new(None))
}
#[cfg(test)]
pub(super) fn set_remove_hook_for_test(hook: Option<fn(&Path) -> std::io::Result<()>>) {
    *remove_hook()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = hook;
}
fn remove_ready_take(take: ReadyTake) -> Result<(), (ReadyTake, CutError)> {
    remove_owned_root(take.root.path()).map_err(|error| {
        (
            take,
            CutError::new(
                error_codes::IO,
                "could not discard the rehearsal media",
                error.to_string(),
            )
            .with_suggested_action("close playback and try discarding the rehearsal again"),
        )
    })
}
fn clear_running(owner: &mut RehearsalOwner, handle: &str) {
    if matches!(&owner.state, TakeState::Running { handle: current, .. } if current == handle) {
        owner.state = TakeState::Idle;
    }
}
/// In-flight cleanup guard. The detached worker owns this guard, so a dropped
/// HTTP future, backend panic, or channel receiver cancellation cannot strand
/// `TakeState::Running` after its native reservation unwinds.
pub(crate) struct RunningTakeGuard {
    handle: String,
}
impl RunningTakeGuard {
    pub(crate) fn new(handle: String) -> Self {
        Self { handle }
    }
}

impl Drop for RunningTakeGuard {
    fn drop(&mut self) {
        clear_running(&mut state::lock_owner(), &self.handle);
    }
}

/// Undo admission if detached worker construction failed. Its closure drops the
/// TempDir; this only clears the matching process state.
pub(crate) fn abandon_take(handle: &str) {
    clear_running(&mut state::lock_owner(), handle);
}

/// Release a completed take before new capture. A running take is signalled and
/// revoked, yet keeps its reservation until its worker exits.
pub(super) fn discard_current(owner: &mut RehearsalOwner) -> Result<Option<String>, CutError> {
    let current = std::mem::replace(&mut owner.state, TakeState::Idle);
    match current {
        TakeState::Idle => Ok(None),
        TakeState::Running { handle, control } => {
            let _ = control.terminalize();
            state::remember_revoked(owner, handle.clone());
            owner.state = TakeState::Running {
                handle: handle.clone(),
                control,
            };
            Err(CutError::new(
                error_codes::CONFLICT,
                "a rehearsal take is still stopping",
                "the bounded native capture still owns the recording devices",
            )
            .with_suggested_action(
                "wait for the 3- to 5-second rehearsal to finish, then start recording",
            ))
        }
        TakeState::Ready(take) => {
            let handle = take.handle.clone();
            match remove_ready_take(take) {
                Ok(()) => {
                    state::remember_revoked(owner, handle.clone());
                    Ok(Some(handle))
                }
                Err((take, error)) => {
                    owner.state = TakeState::Ready(take);
                    Err(error)
                }
            }
        }
    }
}

pub(crate) fn discard_for_recording() -> Result<(), CutError> {
    let mut owner = state::lock_owner();
    let _ = discard_current(&mut owner)?;
    Ok(())
}

pub(crate) fn begin_take(handle: String, control: CaptureSessionControl) -> Result<(), CutError> {
    let mut owner = state::lock_owner();
    let _ = discard_current(&mut owner)?;
    owner.state = TakeState::Running { handle, control };
    Ok(())
}

/// The worker moves `root` here after capture. It either retains it as Ready or
/// drops it after settling Idle/Revoked, never under a caller-owned future.
pub(crate) fn finish_take(
    handle: &str,
    root: tempfile::TempDir,
    capture_result: Result<String, CutError>,
) -> Result<(), CutError> {
    let source = match capture_result {
        Ok(source) => source,
        Err(error) => {
            clear_running(&mut state::lock_owner(), handle);
            return Err(error);
        }
    };
    let media_relative = match media_relative_in_root(root.path(), Path::new(&source)) {
        Ok(relative) => relative,
        Err(error) => {
            clear_running(&mut state::lock_owner(), handle);
            return Err(error);
        }
    };

    let mut owner = state::lock_owner();
    if owner.revoked.iter().any(|revoked| revoked == handle) {
        clear_running(&mut owner, handle);
        return Err(CutError::new(
            error_codes::CONFLICT,
            "the rehearsal was discarded before playback became ready",
            "the caller or a new recording request cancelled the disposable take",
        ));
    }
    if !matches!(&owner.state, TakeState::Running { handle: current, .. } if current == handle) {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "the rehearsal playback session was replaced",
            "a newer rehearsal session became active before this take completed",
        ));
    }
    owner.state = TakeState::Ready(ReadyTake {
        handle: handle.to_owned(),
        root,
        media_relative,
        expires_at: std::time::Instant::now() + READY_TAKE_TTL,
        cleanup_attempts: 0,
        cleanup_retry_scheduled: false,
    });
    drop(owner);
    schedule_expiry(handle.to_owned());
    Ok(())
}

fn schedule_expiry(handle: String) {
    let _ = std::thread::Builder::new()
        .name("cut-rehearsal-expiry".into())
        .spawn(move || {
            std::thread::sleep(READY_TAKE_TTL);
            expire_ready(&handle, false);
        });
}
/// Expired takes remain `Ready` while their root cannot be deleted, but their
/// old expiry still denies playback. Only the bounded retry marker is mutable.
fn retain_failed_expired_cleanup(
    owner: &mut RehearsalOwner,
    mut take: ReadyTake,
) -> Option<String> {
    if take.cleanup_attempts >= MAX_EXPIRED_CLEANUP_RETRIES || take.cleanup_retry_scheduled {
        owner.state = TakeState::Ready(take);
        return None;
    }
    take.cleanup_attempts += 1;
    take.cleanup_retry_scheduled = true;
    let handle = take.handle.clone();
    owner.state = TakeState::Ready(take);
    Some(handle)
}
/// Delete the owned root before revoking/dropping its capability. On a Windows
/// media-file lock, retain the take and arrange at most three delayed retries.
fn expire_take(
    owner: &mut RehearsalOwner,
    mut take: ReadyTake,
    scheduled_retry: bool,
) -> Option<String> {
    // Playback requests deny an already-expired take but never compete with a
    // queued retry. The detached retry alone clears and consumes this marker.
    if !scheduled_retry
        && (take.cleanup_retry_scheduled || take.cleanup_attempts >= MAX_EXPIRED_CLEANUP_RETRIES)
    {
        owner.state = TakeState::Ready(take);
        return None;
    }
    take.cleanup_retry_scheduled = false;
    let handle = take.handle.clone();
    match remove_ready_take(take) {
        Ok(()) => {
            state::remember_revoked(owner, handle);
            None
        }
        Err((take, _error)) => retain_failed_expired_cleanup(owner, take),
    }
}
fn schedule_expired_cleanup_retry(handle: String) -> bool {
    std::thread::Builder::new()
        .name("cut-rehearsal-cleanup-retry".into())
        .spawn(move || {
            std::thread::sleep(EXPIRED_CLEANUP_RETRY_DELAY);
            expire_ready(&handle, true);
        })
        .is_ok()
}
fn clear_failed_retry_schedule(handle: &str) {
    let mut owner = state::lock_owner();
    if let TakeState::Ready(take) = &mut owner.state {
        if take.handle == handle {
            take.cleanup_retry_scheduled = false;
        }
    }
}
fn schedule_retained_cleanup_retry(handle: Option<String>) {
    if let Some(handle) = handle {
        if !schedule_expired_cleanup_retry(handle.clone()) {
            clear_failed_retry_schedule(&handle);
        }
    }
}
fn expire_ready(handle: &str, scheduled_retry: bool) {
    let mut owner = state::lock_owner();
    let current = std::mem::replace(&mut owner.state, TakeState::Idle);
    let retry = match current {
        TakeState::Ready(take)
            if take.handle == handle && take.expires_at <= std::time::Instant::now() =>
        {
            expire_take(&mut owner, take, scheduled_retry)
        }
        current => {
            owner.state = current;
            None
        }
    };
    drop(owner);
    schedule_retained_cleanup_retry(retry);
}
pub(crate) fn playback_media(handle: &str) -> Option<PathBuf> {
    if !valid_handle(handle) {
        return None;
    }
    let mut owner = state::lock_owner();
    let expired = matches!(&owner.state, TakeState::Ready(take) if take.handle == handle && take.expires_at <= std::time::Instant::now());
    if expired {
        let current = std::mem::replace(&mut owner.state, TakeState::Idle);
        let retry = match current {
            TakeState::Ready(take) => expire_take(&mut owner, take, false),
            current => {
                owner.state = current;
                None
            }
        };
        drop(owner);
        schedule_retained_cleanup_retry(retry);
        return None;
    }
    let TakeState::Ready(take) = &owner.state else {
        return None;
    };
    if take.handle != handle {
        return None;
    }
    let canonical_root = take.root.path().canonicalize().ok()?;
    let candidate = take.root.path().join(&take.media_relative);
    let metadata = std::fs::symlink_metadata(&candidate).ok()?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() || metadata.len() == 0 {
        return None;
    }
    let canonical_media = candidate.canonicalize().ok()?;
    canonical_media
        .starts_with(&canonical_root)
        .then_some(canonical_media)
}

pub(crate) fn discard(
    handle: Option<String>,
) -> Result<(Option<String>, bool, &'static str), CutError> {
    if let Some(handle) = handle.as_deref() {
        if !valid_handle(handle) {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "rehearsal playback handle is invalid",
                "the handle is not a server-issued opaque rehearsal capability",
            ));
        }
    }
    let mut owner = state::lock_owner();
    let requested = handle.as_deref();
    let current = match &owner.state {
        TakeState::Idle => None,
        TakeState::Running { handle, .. } | TakeState::Ready(ReadyTake { handle, .. }) => {
            Some(handle.clone())
        }
    };
    if let Some(requested) = requested {
        if owner.revoked.iter().any(|revoked| revoked == requested) {
            return Ok((Some(requested.to_owned()), false, "discarded"));
        }
        if current.as_deref() != Some(requested) {
            return Err(CutError::new(
                error_codes::NOT_FOUND,
                "rehearsal playback is no longer available",
                "the opaque handle does not name the current disposable rehearsal",
            ));
        }
    }
    let handle = current.or(handle);
    let Some(handle) = handle else {
        return Ok((None, false, "discarded"));
    };
    match discard_current(&mut owner) {
        Ok(_) => Ok((Some(handle), true, "discarded")),
        Err(error) if error.code == error_codes::CONFLICT => {
            state::remember_revoked(&mut owner, handle.clone());
            Ok((Some(handle), true, "stopping"))
        }
        Err(error) => Err(error),
    }
}

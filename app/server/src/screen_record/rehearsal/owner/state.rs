//! Process-local state for the one disposable rehearsal capability.

use crate::screen_record::CaptureSessionControl;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub(super) const READY_TAKE_TTL: Duration = Duration::from_secs(90);
pub(super) const MAX_REVOKED_HANDLES: usize = 16;
pub(super) const MAX_EXPIRED_CLEANUP_RETRIES: u8 = 3;
#[cfg(not(test))]
pub(super) const EXPIRED_CLEANUP_RETRY_DELAY: Duration = Duration::from_secs(2);
#[cfg(test)]
pub(super) const EXPIRED_CLEANUP_RETRY_DELAY: Duration = Duration::from_millis(5);

#[derive(Debug)]
pub(super) struct ReadyTake {
    pub(super) handle: String,
    pub(super) root: tempfile::TempDir,
    /// Normalized, fixed file name under `root`, never a caller path.
    pub(super) media_relative: PathBuf,
    pub(super) expires_at: Instant,
    /// A failed deletion keeps the TempDir owned but playback-denied. At most
    /// three detached retries are scheduled for one expired take.
    pub(super) cleanup_attempts: u8,
    pub(super) cleanup_retry_scheduled: bool,
}

#[derive(Debug)]
pub(super) enum TakeState {
    Idle,
    Running {
        handle: String,
        control: CaptureSessionControl,
    },
    Ready(ReadyTake),
}

#[derive(Debug)]
pub(super) struct RehearsalOwner {
    pub(super) state: TakeState,
    /// Bounded memory only; never a recovery journal or durable inventory.
    pub(super) revoked: VecDeque<String>,
}

impl Default for RehearsalOwner {
    fn default() -> Self {
        Self {
            state: TakeState::Idle,
            revoked: VecDeque::new(),
        }
    }
}

fn owner() -> &'static Mutex<RehearsalOwner> {
    static OWNER: OnceLock<Mutex<RehearsalOwner>> = OnceLock::new();
    OWNER.get_or_init(|| Mutex::new(RehearsalOwner::default()))
}

pub(super) fn lock_owner() -> std::sync::MutexGuard<'static, RehearsalOwner> {
    owner()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(super) fn remember_revoked(owner: &mut RehearsalOwner, handle: String) {
    if owner.revoked.iter().any(|existing| existing == &handle) {
        return;
    }
    owner.revoked.push_back(handle);
    while owner.revoked.len() > MAX_REVOKED_HANDLES {
        owner.revoked.pop_front();
    }
}

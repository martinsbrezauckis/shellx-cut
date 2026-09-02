//! Small facade for the disposable rehearsal capability owner.
//!
//! Capability issuance stays here; media validation, process state, and
//! lifecycle transitions each live in focused siblings.

mod lifecycle;
mod media;
mod state;

use cut_core::{error_codes, CutError};

pub(super) use lifecycle::{
    abandon_take, begin_take, discard, discard_for_recording, finish_take, playback_media,
    RunningTakeGuard,
};
#[cfg(test)]
pub(super) use media::media_relative_in_root;

pub(super) fn mint_handle() -> Result<String, CutError> {
    let mut random = [0_u8; 24];
    getrandom::fill(&mut random).map_err(|error| {
        CutError::new(
            error_codes::IO,
            "could not issue a rehearsal playback handle",
            error.to_string(),
        )
    })?;
    Ok(format!("rehearsal_{}", hex::encode(random)))
}

pub(super) fn valid_handle(handle: &str) -> bool {
    let Some(hex_part) = handle.strip_prefix("rehearsal_") else {
        return false;
    };
    hex_part.len() == 48 && hex_part.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn owned_root() -> Result<tempfile::TempDir, CutError> {
    tempfile::Builder::new()
        .prefix("shellx-cut-rehearsal-")
        .tempdir()
        .map_err(|error| {
            CutError::new(
                error_codes::IO,
                "could not create the disposable rehearsal workspace",
                error.to_string(),
            )
        })
}

#[cfg(test)]
pub(super) fn rehearsal_test_lock() -> &'static tokio::sync::Mutex<()> {
    use std::sync::OnceLock;

    static TEST_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    TEST_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

#[cfg(test)]
pub(super) fn install_test_playback() -> String {
    install_test_playback_with_expiry(
        false,
        "rehearsal_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
}

#[cfg(test)]
pub(super) fn install_expired_test_playback() -> String {
    install_test_playback_with_expiry(
        true,
        "rehearsal_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
}

#[cfg(test)]
pub(super) fn install_locked_expired_test_playback() -> String {
    install_test_playback_with_expiry(
        true,
        "rehearsal_dddddddddddddddddddddddddddddddddddddddddddddddd",
    )
}

#[cfg(test)]
pub(super) fn set_remove_hook_for_test(hook: Option<fn(&std::path::Path) -> std::io::Result<()>>) {
    lifecycle::set_remove_hook_for_test(hook)
}

#[cfg(test)]
fn install_test_playback_with_expiry(expired: bool, handle: &str) -> String {
    use std::path::PathBuf;
    use std::time::Instant;

    let root = owned_root().unwrap();
    std::fs::write(
        root.path().join("source.mp4"),
        b"\x00\x00\x00\x18ftypmp42stub",
    )
    .unwrap();
    let handle = handle.to_string();
    let mut owner = state::lock_owner();
    let _ = lifecycle::discard_current(&mut owner);
    // Deterministic test capabilities may reuse a fixed opaque handle after a
    // prior test has revoked it; production handles are random and never do.
    owner.revoked.retain(|revoked| revoked != &handle);
    owner.state = state::TakeState::Ready(state::ReadyTake {
        handle: handle.clone(),
        root,
        media_relative: PathBuf::from("source.mp4"),
        expires_at: if expired {
            Instant::now() - std::time::Duration::from_secs(1)
        } else {
            Instant::now() + state::READY_TAKE_TTL
        },
        cleanup_attempts: 0,
        cleanup_retry_scheduled: false,
    });
    handle
}

//! Shared serialization and panic-safe reset for process-wide output fixtures.

use super::{set_session_output_dir, SESSION_OUTPUT_DIR_TEST_LOCK};

/// Give a test folder a private (owner-only) mode so it does not depend on the machine's umask;
/// the render no longer requires it (a group-writable folder is allowed). A no-op off Unix.
pub(crate) fn make_private(dir: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    let _ = dir;
}

pub(crate) struct SessionOutputDirFixture {
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl SessionOutputDirFixture {
    pub(crate) fn new() -> Self {
        let lock = SESSION_OUTPUT_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        set_session_output_dir(None);
        Self { _lock: lock }
    }
}

impl Drop for SessionOutputDirFixture {
    fn drop(&mut self) {
        // Reset before the mutex unlocks, including assertion unwinding.
        set_session_output_dir(None);
    }
}

/// For a runtime a test builds itself: `Builder::on_thread_start(hook)` gives
/// that runtime's worker and blocking threads THIS test's folder (and only this
/// test's). Call it on the test thread, before building the runtime.
pub(crate) fn thread_start_hook() -> impl Fn() + Send + Sync + 'static {
    let store = super::SESSION_OUTPUT_DIR.with(|store| store.borrow().clone());
    move || super::SESSION_OUTPUT_DIR.with(|slot| *slot.borrow_mut() = store.clone())
}

#[test]
fn session_output_fixture_resets_after_panic_and_recovers_poison() {
    let chosen = tempfile::tempdir().unwrap();
    let failed = std::panic::catch_unwind(|| {
        let _fixture = SessionOutputDirFixture::new();
        set_session_output_dir(Some(chosen.path().to_path_buf()));
        panic!("fixture assertion failure");
    });
    assert!(failed.is_err());
    let lock = SESSION_OUTPUT_DIR_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(super::unscoped_session_output_dir().is_none());
    drop(lock);
    let _fixture = SessionOutputDirFixture::new();
    assert!(super::unscoped_session_output_dir().is_none());
}

/// The folder one test chose must not be visible to any other test running in
/// the same process (a queued render in another test once wrote into a folder
/// this kind of leak had pointed it at after that test removed it).
#[test]
fn a_folder_chosen_in_one_test_is_not_seen_by_another_test() {
    let _fixture = SessionOutputDirFixture::new();
    let chosen = tempfile::tempdir().unwrap();
    set_session_output_dir(Some(chosen.path().to_path_buf()));
    assert!(super::unscoped_session_output_dir().is_some());
    let seen_elsewhere = std::thread::spawn(super::unscoped_session_output_dir)
        .join()
        .unwrap();
    assert!(
        seen_elsewhere.is_none(),
        "another test's thread saw {seen_elsewhere:?}"
    );
}

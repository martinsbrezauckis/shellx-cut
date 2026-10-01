//! Shared serialization and panic-safe reset for process-wide output fixtures.

use super::{set_session_output_dir, SESSION_OUTPUT_DIR_TEST_LOCK};

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

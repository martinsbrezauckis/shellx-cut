//! Focused deterministic tests for rehearsal admission and temporary ownership.

use super::{bounded_duration, owner, MAX_DURATION_MS, MIN_DURATION_MS};
use crate::screen_record::CaptureSessionControl;
use cut_core::error_codes;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

static FAILED_EXPIRY_REMOVALS: AtomicUsize = AtomicUsize::new(0);

fn refuse_expired_removal(_: &std::path::Path) -> std::io::Result<()> {
    FAILED_EXPIRY_REMOVALS.fetch_add(1, Ordering::SeqCst);
    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "simulated open playback file",
    ))
}

#[test]
fn duration_is_strictly_capped_to_a_short_native_take() {
    assert_eq!(bounded_duration(None).unwrap(), MIN_DURATION_MS);
    assert_eq!(
        bounded_duration(Some(MAX_DURATION_MS)).unwrap(),
        MAX_DURATION_MS
    );
    assert_eq!(
        bounded_duration(Some(MIN_DURATION_MS - 1))
            .unwrap_err()
            .code,
        error_codes::INVALID_ARGS
    );
    assert_eq!(
        bounded_duration(Some(MAX_DURATION_MS + 1))
            .unwrap_err()
            .code,
        error_codes::INVALID_ARGS
    );
}

#[test]
fn only_regular_source_mp4_inside_the_owned_root_is_playable() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.mp4");
    std::fs::write(&source, b"native test bytes").unwrap();
    assert_eq!(
        owner::media_relative_in_root(root.path(), &source).unwrap(),
        std::path::Path::new("source.mp4")
    );

    let other = tempfile::NamedTempFile::new().unwrap();
    assert_eq!(
        owner::media_relative_in_root(root.path(), other.path())
            .unwrap_err()
            .code,
        error_codes::IO
    );
    assert_eq!(
        owner::media_relative_in_root(root.path(), std::path::Path::new("source.mp4"))
            .unwrap_err()
            .code,
        error_codes::IO
    );
}

#[tokio::test]
async fn discard_is_idempotent_and_expiry_revokes_playback() {
    let _lock = super::rehearsal_test_lock().lock().await;

    let handle = owner::install_test_playback();
    assert!(owner::playback_media(&handle).is_some());
    assert!(owner::discard(Some(handle.clone())).unwrap().1);
    assert!(!owner::discard(Some(handle.clone())).unwrap().1);
    assert!(owner::playback_media(&handle).is_none());

    let expired = owner::install_expired_test_playback();
    assert!(owner::playback_media(&expired).is_none());
    assert!(!owner::discard(Some(expired)).unwrap().1);

    // A locked Windows media file is not forgotten after expiry. Playback is
    // denied, exactly three bounded retries run, and explicit cleanup still
    // owns and removes the retained TempDir after the lock clears.
    FAILED_EXPIRY_REMOVALS.store(0, Ordering::SeqCst);
    owner::set_remove_hook_for_test(Some(refuse_expired_removal));
    let locked = owner::install_locked_expired_test_playback();
    assert!(owner::playback_media(&locked).is_none());
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
    while FAILED_EXPIRY_REMOVALS.load(Ordering::SeqCst) < 4 && std::time::Instant::now() < deadline
    {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        FAILED_EXPIRY_REMOVALS.load(Ordering::SeqCst),
        4,
        "initial expiry plus exactly three bounded retry removals"
    );
    owner::set_remove_hook_for_test(None);
    assert!(owner::discard(Some(locked)).unwrap().1);
}

/// The request future owns only the response channel. The detached worker owns
/// both the state guard and TempDir; dropping the former cannot strand Running
/// or remove the root under a still-capturing worker.
#[test]
fn dropped_start_receiver_leaves_worker_owner_intact_until_completion() {
    let _lock = super::rehearsal_test_lock().blocking_lock();
    let _ = owner::discard(None);

    let handle = "rehearsal_cccccccccccccccccccccccccccccccccccccccccccccccc".to_string();
    let control = CaptureSessionControl::new(Some(MIN_DURATION_MS), false, false, false);
    owner::begin_take(handle.clone(), control).unwrap();
    let root = owner::owned_root().unwrap();
    let source = root.path().join("source.mp4");
    std::fs::write(&source, b"native test bytes").unwrap();
    let owned_path = root.path().to_path_buf();
    let (started_tx, started_rx) = mpsc::channel();
    let (complete_tx, complete_rx) = mpsc::channel();
    let (response_tx, response_rx) = mpsc::channel();

    let worker_handle = handle.clone();
    let worker = std::thread::spawn(move || {
        let _guard = owner::RunningTakeGuard::new(worker_handle.clone());
        started_tx.send(()).unwrap();
        complete_rx.recv().unwrap();
        let result = owner::finish_take(&worker_handle, root, Ok(source.to_string_lossy().into()));
        let _ = response_tx.send(result);
    });
    started_rx.recv().unwrap();
    drop(response_rx); // exactly the cancelled async caller case
    assert!(
        owned_path.exists(),
        "the worker retains its TempDir while active"
    );
    complete_tx.send(()).unwrap();
    worker.join().unwrap();

    assert!(owner::playback_media(&handle).is_some());
    assert!(owner::discard(Some(handle)).unwrap().1);
}

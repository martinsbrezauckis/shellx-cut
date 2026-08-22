use super::*;
use std::io;
use std::time::Duration;

#[test]
fn cleanup_retry_recovers_before_a_fresh_promotion() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("job_001.json");
    std::fs::write(&path, b"old record").unwrap();
    let delays = [Duration::from_millis(25)];
    let mut promotion_attempts = 0;
    let mut cleanup_attempts = 0;
    let mut slept = Vec::new();

    write_atomically_with_retry_and_cleanup(
        &path,
        b"new record",
        &delays,
        |_| Ok(()),
        |temporary, destination| {
            promotion_attempts += 1;
            if promotion_attempts == 1 {
                Err(PromotionFailure {
                    temporary,
                    error: io::Error::from_raw_os_error(32),
                })
            } else {
                promote_temporary(temporary, destination)
            }
        },
        |temporary| {
            cleanup_attempts += 1;
            if cleanup_attempts == 1 {
                Err(io::Error::from_raw_os_error(5))
            } else {
                std::fs::remove_file(temporary)
            }
        },
        |delay| slept.push(delay),
    )
    .unwrap();

    assert_eq!(promotion_attempts, 2);
    assert_eq!(cleanup_attempts, 2);
    assert_eq!(slept, vec![Duration::from_millis(25); 2]);
    assert_eq!(std::fs::read(&path).unwrap(), b"new record");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn cleanup_not_found_confirms_an_already_removed_temporary() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("job_001.json");
    std::fs::write(&path, b"old record").unwrap();
    let mut promotion_attempts = 0;
    let mut cleanup_attempts = 0;

    write_atomically_with_retry_and_cleanup(
        &path,
        b"new record",
        &[Duration::from_millis(25)],
        |_| Ok(()),
        |temporary, destination| {
            promotion_attempts += 1;
            if promotion_attempts == 1 {
                Err(PromotionFailure {
                    temporary,
                    error: io::Error::from_raw_os_error(32),
                })
            } else {
                promote_temporary(temporary, destination)
            }
        },
        |temporary| {
            cleanup_attempts += 1;
            std::fs::remove_file(temporary).unwrap();
            Err(io::Error::from(io::ErrorKind::NotFound))
        },
        |_| {},
    )
    .unwrap();

    assert_eq!(promotion_attempts, 2);
    assert_eq!(cleanup_attempts, 1);
    assert_eq!(std::fs::read(&path).unwrap(), b"new record");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn nontransient_cleanup_failure_blocks_promotion_retry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("job_001.json");
    std::fs::write(&path, b"old record").unwrap();
    let mut promotion_attempts = 0;
    let mut cleanup_attempts = 0;
    let mut temporary_paths = Vec::new();

    let error = write_atomically_with_retry_and_cleanup(
        &path,
        b"new record",
        &[Duration::from_millis(25)],
        |_| Ok(()),
        |temporary, _| {
            promotion_attempts += 1;
            temporary_paths.push(temporary.path().to_path_buf());
            Err(PromotionFailure {
                temporary,
                error: io::Error::from_raw_os_error(32),
            })
        },
        |_| {
            cleanup_attempts += 1;
            Err(io::Error::from_raw_os_error(2))
        },
        |_| panic!("failed cleanup must not start another promotion retry"),
    )
    .unwrap_err();

    assert_eq!(error.to_string(), "job record temporary cleanup failed");
    assert_eq!(promotion_attempts, 1);
    assert_eq!(cleanup_attempts, 1);
    assert_eq!(std::fs::read(&path).unwrap(), b"old record");
    assert!(temporary_paths[0].exists());
    std::fs::remove_file(&temporary_paths[0]).unwrap();
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn transient_cleanup_exhaustion_preserves_old_json_and_reports_the_remnant() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("job_001.json");
    std::fs::write(&path, b"old record").unwrap();
    let delays = [Duration::from_millis(25), Duration::from_millis(50)];
    let mut promotion_attempts = 0;
    let mut cleanup_attempts = 0;
    let mut temporary_paths = Vec::new();
    let mut slept = Vec::new();

    let error = write_atomically_with_retry_and_cleanup(
        &path,
        b"new record",
        &delays,
        |_| Ok(()),
        |temporary, _| {
            promotion_attempts += 1;
            temporary_paths.push(temporary.path().to_path_buf());
            Err(PromotionFailure {
                temporary,
                error: io::Error::from_raw_os_error(32),
            })
        },
        |_| {
            cleanup_attempts += 1;
            Err(io::Error::from_raw_os_error(5))
        },
        |delay| slept.push(delay),
    )
    .unwrap_err();

    assert_eq!(error.to_string(), "job record temporary cleanup failed");
    assert_eq!(promotion_attempts, 1);
    assert_eq!(cleanup_attempts, delays.len() + 1);
    assert_eq!(slept, delays);
    assert_eq!(std::fs::read(&path).unwrap(), b"old record");
    assert!(temporary_paths[0].exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    std::fs::remove_file(&temporary_paths[0]).unwrap();
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[cfg(windows)]
#[test]
fn windows_temp_side_no_delete_share_is_cleaned_before_retry() {
    use std::os::windows::ffi::OsStrExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    fn open_without_delete_share(path: &std::path::Path) -> HANDLE {
        let wide_path = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let handle = unsafe {
            CreateFileW(
                wide_path.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(
            handle,
            INVALID_HANDLE_VALUE,
            "{}",
            io::Error::last_os_error()
        );
        handle
    }

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("job_001.json");
    std::fs::write(&path, b"old record").unwrap();
    let held_handle = Arc::new(Mutex::new(None::<usize>));
    let promotions = Arc::new(AtomicUsize::new(0));
    let cleanups = Arc::new(AtomicUsize::new(0));
    let promotion_errors = Arc::new(Mutex::new(Vec::new()));
    let cleanup_errors = Arc::new(Mutex::new(Vec::new()));
    let (done_tx, done_rx) = mpsc::channel();
    let writer_path = path.clone();
    let promotion_handle = Arc::clone(&held_handle);
    let cleanup_handle = Arc::clone(&held_handle);
    let writer_promotions = Arc::clone(&promotions);
    let writer_cleanups = Arc::clone(&cleanups);
    let writer_promotion_errors = Arc::clone(&promotion_errors);
    let writer_cleanup_errors = Arc::clone(&cleanup_errors);
    let writer = std::thread::spawn(move || {
        let result = write_atomically_with_retry_and_cleanup(
            &writer_path,
            b"new record",
            platform_replace_retry_delays(),
            |_| Ok(()),
            move |temporary, destination| {
                let attempt = writer_promotions.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    *promotion_handle.lock().unwrap() =
                        Some(open_without_delete_share(temporary.path()) as usize);
                }
                let result = promote_temporary(temporary, destination);
                if let Err(failure) = &result {
                    writer_promotion_errors
                        .lock()
                        .unwrap()
                        .push(failure.error.raw_os_error());
                }
                result
            },
            move |temporary| {
                writer_cleanups.fetch_add(1, Ordering::SeqCst);
                let result = std::fs::remove_file(temporary);
                if let Err(error) = &result {
                    writer_cleanup_errors
                        .lock()
                        .unwrap()
                        .push(error.raw_os_error());
                }
                result
            },
            move |delay| {
                if let Some(handle) = cleanup_handle.lock().unwrap().take() {
                    assert_ne!(
                        unsafe { CloseHandle(handle as HANDLE) },
                        0,
                        "{}",
                        io::Error::last_os_error()
                    );
                }
                std::thread::sleep(delay);
            },
        );
        done_tx.send(result).unwrap();
    });

    let result = done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("temporary cleanup retry exceeded its bounded contention test timeout");
    writer.join().unwrap();
    if let Some(handle) = held_handle.lock().unwrap().take() {
        assert_ne!(
            unsafe { CloseHandle(handle as HANDLE) },
            0,
            "{}",
            io::Error::last_os_error()
        );
    }

    result.unwrap();
    assert_eq!(promotions.load(Ordering::SeqCst), 2);
    assert_eq!(cleanups.load(Ordering::SeqCst), 2);
    let promotion_errors = promotion_errors.lock().unwrap();
    assert_eq!(promotion_errors.len(), 1);
    for raw_os_error in promotion_errors.iter().copied() {
        assert!(is_transient_windows_replace_error(
            &io::Error::from_raw_os_error(raw_os_error.unwrap())
        ));
    }
    let cleanup_errors = cleanup_errors.lock().unwrap();
    assert_eq!(cleanup_errors.len(), 1);
    for raw_os_error in cleanup_errors.iter().copied() {
        assert!(is_transient_windows_replace_error(
            &io::Error::from_raw_os_error(raw_os_error.unwrap())
        ));
    }
    assert_eq!(std::fs::read(&path).unwrap(), b"new record");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

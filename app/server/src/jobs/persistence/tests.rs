use super::*;
use crate::events::EventBus;
use crate::jobs::{JobManager, JobOutcome, JobOutcomeReason, JobState};
use std::time::Duration;

fn record(job_id: &str, state: JobState) -> JobRecord {
    JobRecord {
        job_id: job_id.into(),
        kind: "render".into(),
        state,
        completion: None,
        outcome: None,
        outcome_reason: None,
        progress: if matches!(state, JobState::Done | JobState::Failed) {
            1.0
        } else {
            0.0
        },
        message: None,
        queue: None,
        waiting_on: None,
        retry: None,
        created_ts: "2026-08-08T00:00:00.000Z".into(),
        updated_ts: "2026-08-08T00:00:00.000Z".into(),
        result: None,
        error: None,
        persistence_error: None,
    }
}

#[test]
fn atomic_write_fault_keeps_the_previous_record_and_cleans_the_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("job_001.json");
    std::fs::write(&path, b"old record").unwrap();

    let error = write_atomically_with(&path, b"new record", |_| {
        Err(io::Error::other("injected replace fault"))
    })
    .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert_eq!(std::fs::read(&path).unwrap(), b"old record");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn transient_promotion_retries_with_fresh_synced_temps_until_it_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("job_001.json");
    std::fs::write(&path, b"old record").unwrap();
    let delays = [Duration::from_millis(25), Duration::from_millis(50)];
    let mut attempts = 0;
    let mut temporary_paths = Vec::new();
    let mut slept = Vec::new();

    write_atomically_with_retry(
        &path,
        b"new record",
        &delays,
        |_| Ok(()),
        |temporary, destination| {
            attempts += 1;
            temporary_paths.push(temporary.path().to_path_buf());
            if attempts < 3 {
                Err(PromotionFailure {
                    temporary,
                    error: io::Error::from_raw_os_error(32),
                })
            } else {
                promote_temporary(temporary, destination)
            }
        },
        |delay| slept.push(delay),
    )
    .unwrap();

    assert_eq!(attempts, 3);
    assert_eq!(slept, delays);
    assert_eq!(std::fs::read(&path).unwrap(), b"new record");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(temporary_paths.iter().all(|temporary| !temporary.exists()));
}

#[test]
fn exhausted_transient_promotion_keeps_old_json_and_cleans_every_temporary() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("job_001.json");
    std::fs::write(&path, b"old record").unwrap();
    let delays = [
        Duration::from_millis(25),
        Duration::from_millis(50),
        Duration::from_millis(100),
        Duration::from_millis(200),
        Duration::from_millis(400),
    ];
    let mut attempts = 0;
    let mut temporary_paths = Vec::new();
    let mut slept = Vec::new();

    let error = write_atomically_with_retry(
        &path,
        b"new record",
        &delays,
        |_| Ok(()),
        |temporary, _| {
            attempts += 1;
            temporary_paths.push(temporary.path().to_path_buf());
            Err(PromotionFailure {
                temporary,
                error: io::Error::from_raw_os_error(5),
            })
        },
        |delay| slept.push(delay),
    )
    .unwrap_err();

    assert_eq!(error.raw_os_error(), Some(5));
    assert_eq!(attempts, delays.len() + 1);
    assert_eq!(slept, delays);
    assert_eq!(std::fs::read(&path).unwrap(), b"old record");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(temporary_paths.iter().all(|temporary| !temporary.exists()));
}

#[test]
fn only_windows_sharing_or_access_errors_are_retryable() {
    for code in [5, 32, 33] {
        assert!(is_transient_windows_replace_error(
            &io::Error::from_raw_os_error(code)
        ));
    }
    assert!(!is_transient_windows_replace_error(
        &io::Error::from_raw_os_error(2)
    ));
    assert!(!is_transient_windows_replace_error(&io::Error::other(
        "replace failed"
    )));
}

#[test]
fn other_promotion_errors_do_not_retry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("job_001.json");
    std::fs::write(&path, b"old record").unwrap();
    let mut attempts = 0;
    let mut slept = Vec::new();

    let error = write_atomically_with_retry(
        &path,
        b"new record",
        &[Duration::from_millis(25)],
        |_| Ok(()),
        |temporary, _| {
            attempts += 1;
            Err(PromotionFailure {
                temporary,
                error: io::Error::from_raw_os_error(2),
            })
        },
        |delay| slept.push(delay),
    )
    .unwrap_err();

    assert_eq!(error.raw_os_error(), Some(2));
    assert_eq!(attempts, 1);
    assert!(slept.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), b"old record");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[cfg(not(windows))]
#[test]
fn non_windows_writer_uses_one_promotion_attempt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("job_001.json");
    let mut attempts = 0;

    let error = write_atomically_with_retry(
        &path,
        b"new record",
        platform_replace_retry_delays(),
        |_| Ok(()),
        |temporary, _| {
            attempts += 1;
            Err(PromotionFailure {
                temporary,
                error: io::Error::from_raw_os_error(32),
            })
        },
        |_| panic!("non-Windows persistence must not sleep/retry"),
    )
    .unwrap_err();

    assert_eq!(error.raw_os_error(), Some(32));
    assert_eq!(attempts, 1);
    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[cfg(windows)]
#[test]
fn windows_target_without_delete_share_retries_after_handle_release() {
    use std::os::windows::ffi::OsStrExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, OPEN_EXISTING,
    };

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("job_001.json");
    std::fs::write(&path, b"old record").unwrap();
    let wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // Share read but intentionally deny delete. A replacement must therefore
    // fail with one of the Windows sharing/access codes before the handle is
    // deterministically released by the retry sleeper below.
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ,
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

    // `HANDLE` is a raw pointer, so keep its numeric value in the cross-thread
    // release latch and cast it back only for `CloseHandle`.
    let held_handle = Arc::new(Mutex::new(Some(handle as usize)));
    let attempts = Arc::new(AtomicUsize::new(0));
    let raw_codes = Arc::new(Mutex::new(Vec::new()));
    let (done_tx, done_rx) = mpsc::channel();
    let writer_path = path.clone();
    let writer_attempts = Arc::clone(&attempts);
    let writer_raw_codes = Arc::clone(&raw_codes);
    let writer_handle = Arc::clone(&held_handle);
    let writer = std::thread::spawn(move || {
        let result = write_atomically_with_retry(
            &writer_path,
            b"new record",
            platform_replace_retry_delays(),
            |_| Ok(()),
            move |temporary, destination| {
                writer_attempts.fetch_add(1, Ordering::SeqCst);
                let result = promote_temporary(temporary, destination);
                if let Err(failure) = &result {
                    writer_raw_codes
                        .lock()
                        .unwrap()
                        .push(failure.error.raw_os_error());
                }
                result
            },
            move |delay| {
                let handle = writer_handle
                    .lock()
                    .unwrap()
                    .take()
                    .expect("only the first transient failure may release the handle");
                assert_ne!(
                    unsafe { CloseHandle(handle as HANDLE) },
                    0,
                    "{}",
                    io::Error::last_os_error()
                );
                std::thread::sleep(delay);
            },
        );
        done_tx.send(result).unwrap();
    });

    let result = done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("promote retry exceeded its bounded contention test timeout");
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
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    let raw_codes = raw_codes.lock().unwrap();
    assert_eq!(raw_codes.len(), 1);
    assert!(is_transient_windows_replace_error(
        &io::Error::from_raw_os_error(raw_codes[0].unwrap())
    ));
    assert_eq!(std::fs::read(&path).unwrap(), b"new record");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn persistence_error_clears_after_a_later_successful_transition() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = JobManager::new(EventBus::new());
    mgr.attach_project(dir.path()).unwrap();
    let created = mgr.create("render");
    let jobs_path = dir.path().join("jobs");
    std::fs::remove_dir_all(&jobs_path).unwrap();
    std::fs::write(&jobs_path, b"not a directory").unwrap();

    mgr.progress(&created.job_id, 0.25, Some("blocked".into()));
    assert!(
        mgr.get(&created.job_id)
            .unwrap()
            .persistence_error
            .is_some(),
        "jobs.status/list must expose a live persistence failure"
    );

    std::fs::remove_file(&jobs_path).unwrap();
    std::fs::create_dir(&jobs_path).unwrap();
    mgr.progress(&created.job_id, 0.5, Some("recovered".into()));

    let live = mgr.get(&created.job_id).unwrap();
    assert_eq!(live.progress, 0.5);
    assert_eq!(live.message.as_deref(), Some("recovered"));
    assert!(
        live.persistence_error.is_none(),
        "the next durable transition must clear the stale API error"
    );
    assert!(
        mgr.list()
            .into_iter()
            .find(|record| record.job_id == created.job_id)
            .unwrap()
            .persistence_error
            .is_none(),
        "jobs.list must clear the same recovered persistence error"
    );

    let persisted: JobRecord = serde_json::from_slice(
        &std::fs::read(jobs_path.join(format!("{}.json", created.job_id))).unwrap(),
    )
    .unwrap();
    assert!(
        persisted.persistence_error.is_none(),
        "the recovered transition must durably replace the stale error"
    );

    let reopened = JobManager::new(EventBus::new());
    reopened.attach_project(dir.path()).unwrap();
    let recovered = reopened.get(&created.job_id).unwrap();
    assert_eq!(recovered.state, JobState::Failed);
    assert!(
        recovered.persistence_error.is_none(),
        "a reopened project must not re-expose an error cleared by durable persistence"
    );
}

#[test]
fn corrupt_json_is_quarantined_and_disclosed() {
    let project = tempfile::tempdir().unwrap();
    let jobs = project.path().join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();
    std::fs::write(jobs.join("job_007.json"), b"{not json").unwrap();

    let recovered = recover(project.path()).unwrap();

    assert!(recovered.records.is_empty());
    assert_eq!(recovered.next_seq, 7);
    assert_eq!(recovered.notices.len(), 1);
    assert_eq!(recovered.notices[0].code, "job_record_quarantined");
    assert_eq!(recovered.notices[0].record, "job_007.json");
    assert!(recovered.notices[0].message.contains("invalid JSON"));
    assert!(Path::new(&recovered.notices[0].quarantine).starts_with("quarantine"));
    assert!(!jobs.join("job_007.json").exists());
    assert_eq!(
        std::fs::read_dir(jobs.join("quarantine")).unwrap().count(),
        1
    );
}

#[test]
fn recovery_uses_filename_and_record_history_to_avoid_id_collisions() {
    let project = tempfile::tempdir().unwrap();
    let jobs = project.path().join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();
    std::fs::write(jobs.join("job_021.json"), b"corrupt").unwrap();
    std::fs::write(
        jobs.join("job_004.json"),
        serde_json::to_vec(&record("job_020", JobState::Done)).unwrap(),
    )
    .unwrap();

    let recovered = recover(project.path()).unwrap();

    assert_eq!(recovered.next_seq, 21);
    assert!(recovered.records.is_empty());
    assert_eq!(recovered.notices.len(), 2);
}

#[test]
fn interrupted_records_are_recovered_with_an_atomic_terminal_write() {
    let project = tempfile::tempdir().unwrap();
    let jobs = project.path().join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();
    std::fs::write(
        jobs.join("job_002.json"),
        serde_json::to_vec(&record("job_002", JobState::Running)).unwrap(),
    )
    .unwrap();

    let recovered = recover(project.path()).unwrap();
    let restored = recovered.records.first().unwrap();
    let persisted: JobRecord =
        serde_json::from_slice(&std::fs::read(jobs.join("job_002.json")).unwrap()).unwrap();

    assert_eq!(restored.state, JobState::Failed);
    assert_eq!(restored.message.as_deref(), Some("interrupted by restart"));
    assert_eq!(restored.outcome, Some(JobOutcome::Interrupted));
    assert_eq!(
        restored.outcome_reason,
        Some(JobOutcomeReason::RestartInterrupted)
    );
    assert_eq!(persisted.state, JobState::Failed);
    assert_eq!(persisted.outcome, Some(JobOutcome::Interrupted));
    assert_eq!(persisted.error.as_ref().unwrap().code, "job_failed");
}

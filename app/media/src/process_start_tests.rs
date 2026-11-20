use super::*;
use std::cell::Cell;

#[cfg(unix)]
fn busy_error() -> io::Error {
    io::Error::from_raw_os_error(libc::ETXTBSY)
}

#[cfg(windows)]
fn busy_error() -> io::Error {
    io::Error::from_raw_os_error(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32)
}

#[test]
fn a_busy_start_is_retried_with_the_bounded_waits_then_succeeds() {
    let attempts = Cell::new(0);
    let mut slept = Vec::new();
    let result = retry_busy_start_with(
        &BUSY_RETRY_DELAYS,
        |d| slept.push(d),
        || {
            attempts.set(attempts.get() + 1);
            if attempts.get() < 4 {
                Err(busy_error())
            } else {
                Ok("started")
            }
        },
    );
    assert_eq!(result.unwrap(), "started");
    assert_eq!(attempts.get(), 4);
    assert_eq!(slept, BUSY_RETRY_DELAYS[..3]);
}

#[test]
fn a_start_that_stays_busy_returns_the_plain_busy_error_after_the_bound() {
    let attempts = Cell::new(0);
    let mut slept = Duration::ZERO;
    let error = retry_busy_start_with(
        &BUSY_RETRY_DELAYS,
        |d| slept += d,
        || -> io::Result<()> {
            attempts.set(attempts.get() + 1);
            Err(busy_error())
        },
    )
    .unwrap_err();
    assert!(is_program_busy(&error), "{error}");
    assert_eq!(attempts.get(), 1 + BUSY_RETRY_DELAYS.len());
    assert_eq!(slept, Duration::from_millis(630), "bounded total wait");
}

#[test]
fn other_start_errors_are_returned_at_once() {
    for error in [
        io::Error::from(io::ErrorKind::NotFound),
        io::Error::from(io::ErrorKind::PermissionDenied),
    ] {
        let kind = error.kind();
        let mut once = Some(error);
        let attempts = Cell::new(0);
        let got = retry_busy_start_with(
            &BUSY_RETRY_DELAYS,
            |_| panic!("no wait for a real failure"),
            || -> io::Result<()> {
                attempts.set(attempts.get() + 1);
                Err(once.take().unwrap())
            },
        )
        .unwrap_err();
        assert_eq!(got.kind(), kind);
        assert_eq!(attempts.get(), 1);
    }
}

/// The mechanism, deterministically: a program that some process still has
/// open for writing cannot be started (this is what an inherited descriptor
/// in another thread's just-forked child amounts to).
#[cfg(target_os = "linux")]
#[test]
fn a_program_still_open_for_writing_is_busy_and_starts_once_closed() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let tool = dir.path().join("tool");
    let mut writer = std::fs::File::create(&tool).unwrap();
    writer.write_all(b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();

    let error = std::process::Command::new(&tool).status().unwrap_err();
    assert!(is_program_busy(&error), "{error}");

    // The writer closes while the start is being retried: it then succeeds.
    let closer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        drop(writer);
    });
    let started = std::time::Instant::now();
    let status = retry_busy_start(|| std::process::Command::new(&tool).status()).unwrap();
    closer.join().unwrap();
    assert!(status.success());
    assert!(started.elapsed() < Duration::from_secs(2));

    // Held open for good: the plain busy error after at most 630 ms of waits.
    let _held = std::fs::OpenOptions::new().write(true).open(&tool).unwrap();
    let started = std::time::Instant::now();
    let error = retry_busy_start(|| std::process::Command::new(&tool).status()).unwrap_err();
    assert!(is_program_busy(&error), "{error}");
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(600) && waited < Duration::from_secs(5),
        "{waited:?}"
    );
}

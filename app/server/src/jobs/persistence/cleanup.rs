//! Explicit cleanup for failed job-record promotions.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::NamedTempFile;

/// Keep the owned file handle open while explicitly removing its private
/// same-directory path. tempfile's Windows handle permits delete sharing, so
/// this retains ownership through the bounded cleanup window rather than
/// dropping the handle and widening a path-swap opportunity.
fn retain_temporary_path(temporary: NamedTempFile) -> (std::fs::File, PathBuf) {
    let (file, mut temporary_path) = temporary.into_parts();
    let path = temporary_path.to_path_buf();
    temporary_path.disable_cleanup(true);
    drop(temporary_path);
    (file, path)
}

fn is_transient_windows_cleanup_error(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(5 | 32 | 33))
}

fn ensure_absent(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(io::Error::other(
            "temporary job record remained after cleanup",
        )),
        Err(error) => Err(error),
    }
}

/// Remove a failed promotion temporary before another attempt can begin. A
/// cleanup failure is terminal: a new record is never promoted while the old
/// temporary remains, and a caller never receives success with a remnant.
pub(super) fn cleanup_failed_temporary_with_retry<Cleanup, Sleep>(
    temporary: NamedTempFile,
    retry_delays: &[Duration],
    cleanup: &mut Cleanup,
    sleep: &mut Sleep,
) -> io::Result<()>
where
    Cleanup: FnMut(&Path) -> io::Result<()>,
    Sleep: FnMut(Duration),
{
    let (file, temporary_path) = retain_temporary_path(temporary);

    let result = (|| {
        for attempt in 1..=retry_delays.len() + 1 {
            let result = match cleanup(&temporary_path) {
                Ok(()) => ensure_absent(&temporary_path),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    ensure_absent(&temporary_path)
                }
                Err(error) => Err(error),
            };
            match result {
                Ok(()) => return Ok(()),
                Err(error) => {
                    let raw_os_error = error.raw_os_error();
                    let retry_delay = retry_delays
                        .get(attempt - 1)
                        .copied()
                        .filter(|_| is_transient_windows_cleanup_error(&error));
                    if let Some(delay) = retry_delay {
                        tracing::warn!(
                            attempt,
                            stage = "cleanup",
                            ?raw_os_error,
                            retry_delay_ms = delay.as_millis(),
                            "retrying transient failed job record cleanup"
                        );
                        sleep(delay);
                        continue;
                    }
                    tracing::error!(
                        attempt,
                        stage = "cleanup",
                        ?raw_os_error,
                        "failed job record cleanup could not remove temporary"
                    );
                    return Err(error);
                }
            }
        }
        unreachable!("cleanup loop always returns after its final attempt")
    })();

    drop(file);
    result
}

//! Retry orchestration for atomic job-record promotion.

use super::cleanup::cleanup_failed_temporary_with_retry;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;
use tempfile::NamedTempFile;

#[cfg(windows)]
const WINDOWS_REPLACE_RETRY_DELAYS: [Duration; 5] = [
    Duration::from_millis(25),
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(200),
    Duration::from_millis(400),
];

/// Holds a failed, still-owned temporary so explicit cleanup can confirm it is
/// gone before a fresh same-directory temporary is written.
pub(super) struct PromotionFailure {
    pub(super) temporary: NamedTempFile,
    pub(super) error: io::Error,
}

pub(super) fn promote_temporary(
    temporary: NamedTempFile,
    path: &Path,
) -> Result<(), PromotionFailure> {
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|error| PromotionFailure {
            temporary: error.file,
            error: error.error,
        })
}

pub(super) fn platform_replace_retry_delays() -> &'static [Duration] {
    #[cfg(windows)]
    {
        &WINDOWS_REPLACE_RETRY_DELAYS
    }
    #[cfg(not(windows))]
    {
        &[]
    }
}

pub(super) fn is_transient_windows_replace_error(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(5 | 32 | 33))
}

pub(super) fn write_atomically_with<BeforeReplace>(
    path: &Path,
    bytes: &[u8],
    before_replace: BeforeReplace,
) -> io::Result<()>
where
    BeforeReplace: FnMut(&Path) -> io::Result<()>,
{
    write_atomically_with_retry(
        path,
        bytes,
        platform_replace_retry_delays(),
        before_replace,
        promote_temporary,
        std::thread::sleep,
    )
}

pub(super) fn write_atomically_with_retry<BeforeReplace, Promote, Sleep>(
    path: &Path,
    bytes: &[u8],
    retry_delays: &[Duration],
    before_replace: BeforeReplace,
    promote: Promote,
    sleep: Sleep,
) -> io::Result<()>
where
    BeforeReplace: FnMut(&Path) -> io::Result<()>,
    Promote: FnMut(NamedTempFile, &Path) -> Result<(), PromotionFailure>,
    Sleep: FnMut(Duration),
{
    write_atomically_with_retry_and_cleanup(
        path,
        bytes,
        retry_delays,
        before_replace,
        promote,
        |temporary_path| std::fs::remove_file(temporary_path),
        sleep,
    )
}

pub(super) fn write_atomically_with_retry_and_cleanup<BeforeReplace, Promote, Cleanup, Sleep>(
    path: &Path,
    bytes: &[u8],
    retry_delays: &[Duration],
    mut before_replace: BeforeReplace,
    mut promote: Promote,
    mut cleanup: Cleanup,
    mut sleep: Sleep,
) -> io::Result<()>
where
    BeforeReplace: FnMut(&Path) -> io::Result<()>,
    Promote: FnMut(NamedTempFile, &Path) -> Result<(), PromotionFailure>,
    Cleanup: FnMut(&Path) -> io::Result<()>,
    Sleep: FnMut(Duration),
{
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "job record has no parent"))?;

    for attempt in 1..=retry_delays.len() + 1 {
        // Each replace attempt receives a new file with fully synced bytes.
        // Never reuse a temporary after a sharing violation: it might have
        // been observed or delayed by a Windows scanner/indexer.
        let mut temporary = NamedTempFile::new_in(parent)?;
        temporary.write_all(bytes)?;
        temporary.as_file().sync_all()?;
        before_replace(temporary.path())?;

        match promote(temporary, path) {
            Ok(()) => {
                // Directory sync is unavailable on some Windows filesystems.
                // The replace remains atomic; this is best-effort extra crash
                // durability there.
                if let Ok(directory) = std::fs::File::open(parent) {
                    let _ = directory.sync_all();
                }
                return Ok(());
            }
            Err(PromotionFailure { temporary, error }) => {
                let raw_os_error = error.raw_os_error();
                let retry_delay = retry_delays
                    .get(attempt - 1)
                    .copied()
                    .filter(|_| is_transient_windows_replace_error(&error));

                if let Err(cleanup_error) = cleanup_failed_temporary_with_retry(
                    temporary,
                    retry_delays,
                    &mut cleanup,
                    &mut sleep,
                ) {
                    tracing::error!(
                        attempt,
                        stage = "promote",
                        ?raw_os_error,
                        "job record promotion failed before temporary cleanup"
                    );
                    return Err(io::Error::new(
                        cleanup_error.kind(),
                        "job record temporary cleanup failed",
                    ));
                }

                if let Some(delay) = retry_delay {
                    tracing::warn!(
                        attempt,
                        stage = "promote",
                        ?raw_os_error,
                        retry_delay_ms = delay.as_millis(),
                        "retrying transient job record promotion failure"
                    );
                    sleep(delay);
                    continue;
                }

                tracing::error!(
                    attempt,
                    stage = "promote",
                    ?raw_os_error,
                    "job record promotion failed"
                );
                return Err(error);
            }
        }
    }

    unreachable!("retry loop always returns after its final promotion attempt")
}

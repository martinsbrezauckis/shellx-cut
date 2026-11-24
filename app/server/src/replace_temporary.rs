//! Atomic replacement with a bounded retry for short-lived Windows readers.
use std::{io, path::Path, time::Duration};
use tempfile::{NamedTempFile, PersistError};

const RETRIES: u32 = 200;
const READ_RETRIES: u32 = 40;
const PAUSE: Duration = Duration::from_millis(5);

pub(crate) fn replace_temporary(file: NamedTempFile, path: &Path) -> io::Result<()> {
    replace_with(
        file,
        path,
        cfg!(windows),
        |file, path| file.persist(path).map(|_| ()),
        std::thread::sleep,
    )
}

/// Read an existing UTF-8 file, retrying short-lived Windows handle/replacement
/// races before believing `NotFound` or failing with sharing/access errors.
pub(crate) fn read_optional_text(path: &Path) -> io::Result<Option<String>> {
    read_optional_text_with(
        path,
        cfg!(windows),
        |path| std::fs::read_to_string(path),
        std::thread::sleep,
    )
}

pub(crate) fn read_optional_text_with(
    path: &Path,
    windows: bool,
    mut read: impl FnMut(&Path) -> io::Result<String>,
    mut sleep: impl FnMut(Duration),
) -> io::Result<Option<String>> {
    for attempt in 0..=READ_RETRIES {
        match read(path) {
            Ok(text) => return Ok(Some(text)),
            Err(error)
                if windows
                    && (error.kind() == io::ErrorKind::NotFound
                        || matches!(error.raw_os_error(), Some(5 | 32)))
                    && attempt < READ_RETRIES =>
            {
                sleep(PAUSE);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        }
    }
    unreachable!("final read attempt always returns")
}

fn replace_with(
    file: NamedTempFile,
    path: &Path,
    windows: bool,
    mut replace: impl FnMut(NamedTempFile, &Path) -> Result<(), PersistError>,
    mut sleep: impl FnMut(Duration),
) -> io::Result<()> {
    let mut pending = file;
    for refused in 0..=RETRIES {
        match replace(pending, path) {
            Ok(()) => return Ok(()),
            Err(error)
                if windows
                    && matches!(error.error.raw_os_error(), Some(5 | 32))
                    && refused < RETRIES =>
            {
                pending = error.file;
                sleep(PAUSE);
            }
            Err(error) => return Err(error.error),
        }
    }
    unreachable!("final replacement always returns")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temporary(dir: &Path) -> NamedTempFile {
        let mut file = NamedTempFile::new_in(dir).unwrap();
        file.write_all(b"complete new contents").unwrap();
        file
    }

    #[test]
    fn retries_windows_handle_refusals_then_publishes_complete_file() {
        for code in [5, 32] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("index");
            std::fs::write(&path, b"old").unwrap();
            let mut attempts = 0;
            let mut pauses = 0;
            replace_with(
                temporary(dir.path()),
                &path,
                true,
                |file, path| {
                    attempts += 1;
                    if attempts <= 2 {
                        assert_eq!(std::fs::read(path).unwrap(), b"old");
                        Err(PersistError {
                            file,
                            error: io::Error::from_raw_os_error(code),
                        })
                    } else {
                        file.persist(path).map(|_| ())
                    }
                },
                |pause| {
                    assert_eq!(pause, PAUSE);
                    pauses += 1;
                },
            )
            .unwrap();
            assert_eq!((attempts, pauses), (3, 2));
            assert_eq!(std::fs::read(path).unwrap(), b"complete new contents");
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn final_failure_is_bounded_preserves_error_and_cleans_temporary() {
        for (windows, code, expected) in [
            (true, 5, RETRIES + 1),
            (true, 32, RETRIES + 1),
            (true, 13, 1),
            (false, 5, 1),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("index");
            std::fs::write(&path, b"old").unwrap();
            let mut attempts = 0;
            let mut pauses = 0;
            let error = replace_with(
                temporary(dir.path()),
                &path,
                windows,
                |file, _| {
                    attempts += 1;
                    Err(PersistError {
                        file,
                        error: io::Error::from_raw_os_error(code),
                    })
                },
                |_| pauses += 1,
            )
            .unwrap_err();
            assert_eq!(attempts, expected);
            assert_eq!(pauses, expected - 1);
            assert_eq!(error.raw_os_error(), Some(code));
            assert_eq!(std::fs::read(path).unwrap(), b"old");
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn retries_windows_transient_read_errors_and_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        let transient_errors = [
            || io::Error::new(io::ErrorKind::NotFound, "replacement gap"),
            || io::Error::from_raw_os_error(5),
            || io::Error::from_raw_os_error(32),
        ];
        for make_err in transient_errors {
            let mut attempts = 0;
            let mut pauses = 0;
            let text = read_optional_text_with(
                &path,
                true,
                |_| {
                    attempts += 1;
                    if attempts <= 3 {
                        Err(make_err())
                    } else {
                        Ok("{\"ok\":true}".to_string())
                    }
                },
                |pause| {
                    assert_eq!(pause, PAUSE);
                    pauses += 1;
                },
            )
            .unwrap();
            assert_eq!((attempts, pauses), (4, 3));
            assert_eq!(text.as_deref(), Some("{\"ok\":true}"));
        }
    }

    #[test]
    fn confirms_windows_not_found_and_preserves_exhausted_or_non_windows_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");

        let mut attempts = 0;
        let mut pauses = 0;
        let missing = read_optional_text_with(
            &path,
            true,
            |_| {
                attempts += 1;
                Err(io::Error::new(io::ErrorKind::NotFound, "absent"))
            },
            |_| pauses += 1,
        )
        .unwrap();
        assert_eq!(missing, None);
        assert_eq!((attempts, pauses), (READ_RETRIES + 1, READ_RETRIES));

        for (windows, code, expected) in [
            (true, 5, READ_RETRIES + 1),
            (true, 32, READ_RETRIES + 1),
            (true, 13, 1),
            (false, 5, 1),
        ] {
            let mut attempts = 0;
            let mut pauses = 0;
            let err = read_optional_text_with(
                &path,
                windows,
                |_| {
                    attempts += 1;
                    Err(io::Error::from_raw_os_error(code))
                },
                |_| pauses += 1,
            )
            .unwrap_err();
            assert_eq!(attempts, expected);
            assert_eq!(pauses, expected - 1);
            assert_eq!(err.raw_os_error(), Some(code));
        }

        let mut non_win_attempts = 0;
        let mut non_win_pauses = 0;
        assert_eq!(
            read_optional_text_with(
                &path,
                false,
                |_| {
                    non_win_attempts += 1;
                    Err(io::Error::new(io::ErrorKind::NotFound, "absent"))
                },
                |_| non_win_pauses += 1,
            )
            .unwrap(),
            None
        );
        assert_eq!((non_win_attempts, non_win_pauses), (1, 0));
    }
}

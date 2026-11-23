//! Atomic replacement with a bounded retry for short-lived Windows readers.
use std::{io, path::Path, time::Duration};
use tempfile::{NamedTempFile, PersistError};

const RETRIES: u32 = 200;
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
}

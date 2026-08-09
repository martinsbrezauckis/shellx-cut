//! Atomic final-output publication shared by ffmpeg render entry points.

use cut_core::{error_codes, CutError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static OUTPUT_SEQ: AtomicU64 = AtomicU64::new(0);

fn temporary_output_path(out: &Path) -> PathBuf {
    let parent = out.parent().unwrap_or_else(|| Path::new("."));
    let stem = out
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("ffmpeg-output");
    let ext = out.extension().and_then(|s| s.to_str()).unwrap_or("tmp");
    let sequence = OUTPUT_SEQ.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(
        ".{stem}.{}.{}.tmp.{ext}",
        std::process::id(),
        sequence
    ))
}

/// Remove only a stale zero-byte output; a prior valid render is left intact.
pub(crate) fn clear_stale_empty_output(out: &Path) -> Result<(), CutError> {
    if std::fs::metadata(out)
        .ok()
        .is_some_and(|metadata| metadata.is_file() && metadata.len() == 0)
    {
        std::fs::remove_file(out)?;
    }
    Ok(())
}

/// Publish a completed sibling output at its final path. Unix rename replaces
/// atomically. Windows `std::fs::rename` refuses an existing destination, so
/// use the native replacement operation instead of silently accepting stale
/// prior bytes as a successful render.
#[cfg(not(windows))]
fn publish_temporary_output(tmp: &Path, out: &Path) -> std::io::Result<()> {
    std::fs::rename(tmp, out)
}

#[cfg(windows)]
fn publish_temporary_output(tmp: &Path, out: &Path) -> std::io::Result<()> {
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    let existing: Vec<u16> = tmp.as_os_str().encode_wide().chain(once(0)).collect();
    let new: Vec<u16> = out.as_os_str().encode_wide().chain(once(0)).collect();
    // SAFETY: both UTF-16 path vectors are NUL-terminated and remain alive for
    // the duration of the call. Temp and final live beside each other, so the
    // native move is a same-volume replacement rather than a copy/delete move.
    let moved = unsafe {
        MoveFileExW(
            existing.as_ptr(),
            new.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Invoke an output writer with a unique sibling path, publishing it only after
/// success. A stale zero-byte final output is removed before work begins; a
/// nonempty prior output stays available if the new writer fails.
pub(crate) fn run_with_atomic_output<T>(
    args: &[String],
    out: &Path,
    run: impl FnOnce(&[String]) -> Result<T, CutError>,
) -> Result<T, CutError> {
    clear_stale_empty_output(out)?;
    let tmp = temporary_output_path(out);
    let mut tmp_args = args.to_vec();
    if let Some(last) = tmp_args.last_mut() {
        *last = tmp.display().to_string();
    }

    let value = match run(&tmp_args) {
        Ok(value) => value,
        Err(error) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(error);
        }
    };
    let temporary_metadata = std::fs::metadata(&tmp)?;
    if temporary_metadata.is_file() && temporary_metadata.len() == 0 {
        let _ = std::fs::remove_file(&tmp);
        return Err(CutError::new(
            error_codes::FFMPEG,
            "ffmpeg completed without output bytes",
            "the temporary final-render file was zero bytes",
        ));
    }
    match publish_temporary_output(&tmp, out) {
        Ok(()) => Ok(value),
        Err(error) => {
            let _ = std::fs::remove_file(&tmp);
            Err(error.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::run_with_atomic_output;
    use cut_core::{error_codes, CutError};

    fn args_for(out: &std::path::Path) -> Vec<String> {
        vec!["-i".into(), "input.mov".into(), out.display().to_string()]
    }

    #[test]
    fn failed_writer_removes_its_temp_and_a_stale_zero_byte_final_output() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("final.mp4");
        std::fs::write(&out, []).unwrap();

        let error = run_with_atomic_output(&args_for(&out), &out, |tmp_args| {
            let tmp = std::path::Path::new(tmp_args.last().unwrap());
            assert!(
                !out.exists(),
                "stale empty final output must be cleared first"
            );
            std::fs::write(tmp, []).unwrap();
            Err::<(), CutError>(CutError::new(
                error_codes::FFMPEG,
                "encode failed",
                "test failure",
            ))
        })
        .unwrap_err();

        assert_eq!(error.code, error_codes::FFMPEG);
        assert!(
            !out.exists(),
            "a failed render cannot leave an empty final file"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn failed_writer_keeps_an_existing_nonempty_final_output() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("final.mp4");
        std::fs::write(&out, b"known-good").unwrap();

        let _ = run_with_atomic_output(&args_for(&out), &out, |tmp_args| {
            std::fs::write(tmp_args.last().unwrap(), b"partial").unwrap();
            Err::<(), CutError>(CutError::new(
                error_codes::FFMPEG,
                "encode failed",
                "test failure",
            ))
        });

        assert_eq!(std::fs::read(&out).unwrap(), b"known-good");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn successful_writer_publishes_its_temp_at_the_final_path() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("final.mp4");

        let result = run_with_atomic_output(&args_for(&out), &out, |tmp_args| {
            std::fs::write(tmp_args.last().unwrap(), b"complete").unwrap();
            Ok("finished")
        })
        .unwrap();

        assert_eq!(result, "finished");
        assert_eq!(std::fs::read(&out).unwrap(), b"complete");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn second_successful_writer_replaces_the_prior_final_output() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("final.mp4");
        std::fs::write(&out, b"old-render").unwrap();

        run_with_atomic_output(&args_for(&out), &out, |tmp_args| {
            std::fs::write(tmp_args.last().unwrap(), b"new-render").unwrap();
            Ok(())
        })
        .unwrap();

        assert_eq!(std::fs::read(&out).unwrap(), b"new-render");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn publication_failure_at_an_existing_destination_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("final.mp4");
        std::fs::create_dir(&out).unwrap();

        let error = run_with_atomic_output(&args_for(&out), &out, |tmp_args| {
            std::fs::write(tmp_args.last().unwrap(), b"new-render").unwrap();
            Ok(())
        })
        .unwrap_err();

        assert_eq!(error.code, error_codes::IO);
        assert!(out.is_dir());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn zero_byte_success_is_rejected_before_it_can_be_published() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("final.mp4");

        let error = run_with_atomic_output(&args_for(&out), &out, |tmp_args| {
            std::fs::write(tmp_args.last().unwrap(), []).unwrap();
            Ok(())
        })
        .unwrap_err();

        assert_eq!(error.code, error_codes::FFMPEG);
        assert!(!out.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}

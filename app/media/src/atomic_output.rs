//! Atomic final-output publication shared by ffmpeg render entry points.

use cut_core::{error_codes, CutError};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(all(test, unix))]
use std::sync::atomic::AtomicU64;

/// Retained only to let the regression construct the predictable path used by
/// the vulnerable implementation. Production reservations never use it.
#[cfg(all(test, unix))]
static OUTPUT_SEQ: AtomicU64 = AtomicU64::new(0);

/// Reserve a random, exclusively-created regular sibling before handing its
/// path to ffmpeg. `NamedTempFile` creates the leaf with the platform's
/// create-new operation, so an existing symlink/reparse point cannot be opened
/// as the output. The handle is closed before ffmpeg starts because ffmpeg
/// opens and truncates its own output path (including on Windows).
///
/// The containing output/cache directory must itself be a local plain
/// directory. On Unix it also cannot be writable by group or other users, so
/// another user cannot replace this reservation between its creation and
/// ffmpeg's later open. The temp remains directly beside the final output,
/// preserving the same-directory atomic rename contract.
fn reserve_temporary_output(out: &Path) -> Result<PathBuf, CutError> {
    let parent = out.parent().unwrap_or_else(|| Path::new("."));
    ensure_plain_output_dir(parent)?;

    let extension = out
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("tmp");
    let suffix = format!(".tmp.{extension}");
    let temporary = tempfile::Builder::new()
        .prefix(".cut-ffmpeg-")
        .suffix(&suffix)
        .tempfile_in(parent)?;
    let (file, path) = temporary.keep().map_err(|error| error.error)?;
    drop(file);

    if !is_plain_regular_file(&fs::symlink_metadata(&path)?) {
        return Err(CutError::new(
            error_codes::IO,
            format!(
                "could not reserve a regular temporary output: {}",
                path.display()
            ),
            "exclusive ffmpeg staging reservation was replaced by an unsafe filesystem entry",
        ));
    }
    Ok(path)
}

fn ensure_plain_output_dir(path: &Path) -> Result<(), CutError> {
    let metadata = fs::symlink_metadata(path)?;
    if !is_plain_directory(&metadata) {
        return Err(CutError::new(
            error_codes::IO,
            format!(
                "output directory is not a local plain directory: {}",
                path.display()
            ),
            "ffmpeg output staging refuses symlinked or reparse-point directories",
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if metadata.mode() & 0o022 != 0 {
            return Err(CutError::new(
                error_codes::IO,
                format!(
                    "output directory is writable by group or other users: {}",
                    path.display()
                ),
                "ffmpeg output staging requires a directory protected from external replacement",
            ));
        }
    }

    Ok(())
}

fn is_plain_directory(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_dir() && !metadata.file_type().is_symlink() && !is_reparse(metadata)
}

fn is_plain_regular_file(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_file() && !metadata.file_type().is_symlink() && !is_reparse(metadata)
}

#[cfg(windows)]
fn is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

#[cfg(not(windows))]
fn is_reparse(_metadata: &fs::Metadata) -> bool {
    false
}

/// Remove only a known plain regular reservation. An unexpected replacement is
/// left for inspection instead of unlinking something the renderer did not
/// create.
fn remove_reserved_temporary_output(path: &Path) {
    if fs::symlink_metadata(path)
        .map(|metadata| is_plain_regular_file(&metadata))
        .unwrap_or(false)
    {
        let _ = fs::remove_file(path);
    }
}

/// Remove only a stale zero-byte output; a prior valid render is left intact.
pub(crate) fn clear_stale_empty_output(out: &Path) -> Result<(), CutError> {
    if fs::symlink_metadata(out)
        .ok()
        .is_some_and(|metadata| is_plain_regular_file(&metadata) && metadata.len() == 0)
    {
        fs::remove_file(out)?;
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
    run_with_validated_atomic_output(args, out, run, |_| Ok(()))
}

/// Variant of [`run_with_atomic_output`] that validates the completed temporary
/// file before it becomes visible at the final path. This is for cache outputs
/// whose non-zero size alone is not enough to establish that ffmpeg completed
/// a usable container.
pub(crate) fn run_with_validated_atomic_output<T>(
    args: &[String],
    out: &Path,
    run: impl FnOnce(&[String]) -> Result<T, CutError>,
    validate: impl FnOnce(&Path) -> Result<(), CutError>,
) -> Result<T, CutError> {
    clear_stale_empty_output(out)?;
    let tmp = reserve_temporary_output(out)?;
    let mut tmp_args = args.to_vec();
    if let Some(last) = tmp_args.last_mut() {
        *last = tmp.display().to_string();
    }

    let value = match run(&tmp_args) {
        Ok(value) => value,
        Err(error) => {
            remove_reserved_temporary_output(&tmp);
            return Err(error);
        }
    };
    let temporary_metadata = fs::symlink_metadata(&tmp)?;
    if !is_plain_regular_file(&temporary_metadata) {
        remove_reserved_temporary_output(&tmp);
        return Err(CutError::new(
            error_codes::FFMPEG,
            "ffmpeg output was not a regular local file",
            "the temporary final-render path became a symlink or reparse point",
        ));
    }
    if temporary_metadata.len() == 0 {
        remove_reserved_temporary_output(&tmp);
        return Err(CutError::new(
            error_codes::FFMPEG,
            "ffmpeg completed without output bytes",
            "the temporary final-render file was zero bytes",
        ));
    }
    if let Err(error) = validate(&tmp) {
        remove_reserved_temporary_output(&tmp);
        return Err(error);
    }
    match publish_temporary_output(&tmp, out) {
        Ok(()) => Ok(value),
        Err(error) => {
            remove_reserved_temporary_output(&tmp);
            Err(error.into())
        }
    }
}

#[cfg(test)]
mod tests;

struct StageDirectory {
    path: PathBuf,
}
fn create_stage(parent: &Path, name: &str) -> Result<StageDirectory, CutError> {
    ensure_plain_dir(parent)?;
    for attempt in 0..PACKAGE_STAGE_ATTEMPTS {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let stage = parent.join(format!(
            ".{name}.package-{}-{nonce}-{attempt}.stage",
            std::process::id()
        ));
        match fs::create_dir(&stage) {
            Ok(()) => return Ok(StageDirectory { path: stage }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(CutError::new(
        error_codes::CONFLICT,
        "could not reserve a private portable package stage",
        "multiple random stage names already existed",
    ))
}

fn ensure_plain_dir(path: &Path) -> Result<(), CutError> {
    let metadata = fs::symlink_metadata(path)?;
    if is_plain_dir(&metadata) {
        Ok(())
    } else {
        Err(CutError::new(
            error_codes::INVALID_ARGS,
            "portable package path is not a plain local directory",
            path.display().to_string(),
        ))
    }
}

fn is_plain_dir(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_dir() && !metadata.file_type().is_symlink() && !is_reparse(metadata)
}

fn is_plain_regular(metadata: &fs::Metadata) -> bool {
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

fn cleanup_stage(stage: &Path, name: &str) {
    if !stage_is_expected(stage, name) {
        tracing::warn!(stage = %stage.display(), "portable package stage retained because it contains unexpected entries");
        return;
    }
    let _ = fs::remove_dir_all(stage);
}

fn stage_is_expected(stage: &Path, name: &str) -> bool {
    let Ok(entries) = fs::read_dir(stage) else {
        return false;
    };
    let expected = format!("{name}.cutproj");
    let entries = entries.collect::<Result<Vec<_>, _>>();
    let Ok(entries) = entries else {
        return false;
    };
    match entries.as_slice() {
        [] => true,
        [entry] if entry.file_name() == std::ffi::OsStr::new(&expected) => {
            fs::symlink_metadata(entry.path())
                .map(|metadata| is_plain_dir(&metadata) && package_tree_is_expected(&entry.path()))
                .unwrap_or(false)
        }
        _ => false,
    }
}

fn package_tree_is_expected(root: &Path) -> bool {
    let Ok(entries) = fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            return false;
        };
        let accepted = match name.to_str() {
            Some("ops.jsonl" | "project.json" | "package.manifest.json") => {
                is_plain_regular(&metadata)
            }
            Some("receipts" | "proxies" | "filmstrip") => {
                is_plain_dir(&metadata)
                    && fs::read_dir(entry.path())
                        .map(|mut children| children.next().is_none())
                        .unwrap_or(false)
            }
            Some("media") => is_plain_dir(&metadata) && media_tree_is_expected(&entry.path()),
            _ => false,
        };
        if !accepted {
            return false;
        }
    }
    true
}

fn media_tree_is_expected(root: &Path) -> bool {
    let Ok(entries) = fs::read_dir(root) else {
        return false;
    };
    let entries = entries.collect::<Result<Vec<_>, _>>();
    let Ok(entries) = entries else {
        return false;
    };
    match entries.as_slice() {
        [entry] if entry.file_name() == "sha256" => fs::symlink_metadata(entry.path())
            .map(|metadata| {
                is_plain_dir(&metadata)
                    && fs::read_dir(entry.path())
                        .map(|children| {
                            children.flatten().all(|child| {
                                fs::symlink_metadata(child.path())
                                    .map(|metadata| {
                                        is_plain_regular(&metadata)
                                            && child
                                                .file_name()
                                                .to_str()
                                                .is_some_and(is_expected_media_stage_file)
                                    })
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false)
            })
            .unwrap_or(false),
        _ => false,
    }
}

fn is_expected_media_stage_file(name: &str) -> bool {
    if name.starts_with(".package-") && name.ends_with(".part") {
        return true;
    }
    let Some((hash, extension)) = name.split_once('.') else {
        return false;
    };
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && !extension.is_empty()
        && extension.len() <= 16
        && extension.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn sync_dir(path: &Path) -> Result<(), CutError> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn publish_new_directory(stage: &Path, target: &Path) -> Result<(), CutError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    ensure_plain_dir(stage.parent().unwrap_or_else(|| Path::new(".")))?;
    ensure_plain_dir(target.parent().unwrap_or_else(|| Path::new(".")))?;
    let from = CString::new(stage.as_os_str().as_bytes()).map_err(|_| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "portable package stage path contains NUL",
            "unsafe stage path",
        )
    })?;
    let to = CString::new(target.as_os_str().as_bytes()).map_err(|_| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "portable package target path contains NUL",
            "unsafe destination path",
        )
    })?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn publish_new_directory(stage: &Path, target: &Path) -> Result<(), CutError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    ensure_plain_dir(stage.parent().unwrap_or_else(|| Path::new(".")))?;
    ensure_plain_dir(target.parent().unwrap_or_else(|| Path::new(".")))?;
    let from = CString::new(stage.as_os_str().as_bytes()).map_err(|_| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "portable package stage path contains NUL",
            "unsafe stage path",
        )
    })?;
    let to = CString::new(target.as_os_str().as_bytes()).map_err(|_| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "portable package target path contains NUL",
            "unsafe destination path",
        )
    })?;
    let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn publish_new_directory(stage: &Path, target: &Path) -> Result<(), CutError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    ensure_plain_dir(stage.parent().unwrap_or_else(|| Path::new(".")))?;
    ensure_plain_dir(target.parent().unwrap_or_else(|| Path::new(".")))?;
    let from = stage
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let to = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: the paths are nul-terminated UTF-16 buffers that remain alive
    // for the call. Omitting MOVEFILE_REPLACE_EXISTING is the no-clobber gate.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn publish_new_directory(_stage: &Path, _target: &Path) -> Result<(), CutError> {
    Err(CutError::new(
        error_codes::UNIMPLEMENTED,
        "portable package publication is unavailable on this platform",
        "Cut has no verified no-replace directory primitive for this target",
    ))
}

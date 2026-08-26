//! Bounded symlink-refusing directory scan and complete SHA-256 hashing.

use super::*;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;

pub(super) fn scan_folder(requested: &Path) -> Result<(PathBuf, Vec<PathBuf>, usize), CutError> {
    let metadata = fs::symlink_metadata(requested).map_err(|error| {
        CutError::new(
            error_codes::NOT_FOUND,
            "recovery folder is unavailable",
            error.to_string(),
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "recovery root must be a real local directory, not a symlink",
            "choose a folder without symlink traversal",
        ));
    }
    let root = requested.canonicalize()?;
    let mut pending = vec![(root.clone(), 0usize)];
    let mut files = Vec::new();
    let mut directories = 0usize;
    let mut total_bytes = 0u64;
    while let Some((directory, depth)) = pending.pop() {
        directories += 1;
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(CutError::new(
                    error_codes::CONFLICT,
                    "recovery preview refused a symlink",
                    format!("{} is a symlink", path.display()),
                )
                .with_suggested_action("choose a symlink-free folder"));
            }
            if metadata.is_dir() {
                if depth >= MAX_SCAN_DEPTH {
                    return Err(CutError::new(
                        error_codes::CONFLICT,
                        "recovery preview exceeded its folder-depth limit",
                        format!("maximum depth is {MAX_SCAN_DEPTH}"),
                    )
                    .with_suggested_action("choose a narrower folder"));
                }
                pending.push((path, depth + 1));
            } else if metadata.is_file() {
                files.push(path);
                total_bytes = total_bytes.saturating_add(metadata.len());
                if files.len() > MAX_SCAN_FILES || total_bytes > MAX_SCAN_BYTES {
                    return Err(CutError::new(
                        error_codes::CONFLICT,
                        "recovery preview exceeds its bounded scan limit",
                        format!(
                            "at most {MAX_SCAN_FILES} files and {} GiB",
                            MAX_SCAN_BYTES / 1024 / 1024 / 1024
                        ),
                    )
                    .with_suggested_action("choose a narrower folder"));
                }
            }
        }
    }
    files.sort();
    for file in &files {
        let canonical = file.canonicalize()?;
        if !canonical.starts_with(&root) {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "recovery preview path escaped its selected folder",
                canonical.display().to_string(),
            ));
        }
    }
    Ok((root, files, directories))
}

pub(super) fn full_sha256_under_root(path: &Path, root: &Path) -> Result<String, CutError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "recovery candidate became a symlink or non-regular file",
            path.display().to_string(),
        ));
    }
    let canonical = path.canonicalize()?;
    if !canonical.starts_with(root) {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "recovery candidate escaped the selected folder",
            canonical.display().to_string(),
        ));
    }
    full_sha256(&canonical)
}

pub(super) fn full_sha256(path: &Path) -> Result<String, CutError> {
    let before = fs::symlink_metadata(path)?;
    if !before.is_file() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "recovery candidate is not a regular file",
            path.display().to_string(),
        ));
    }
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?
    };
    #[cfg(not(unix))]
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let bytes = file.read(&mut buffer)?;
        if bytes == 0 {
            break;
        }
        digest.update(&buffer[..bytes]);
    }
    let after = fs::symlink_metadata(path)?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "recovery candidate changed while it was being hashed",
            path.display().to_string(),
        )
        .with_suggested_action("run preview again after file copying finishes"));
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

pub(super) fn hash_json<T: serde::Serialize>(value: &T) -> Result<String, CutError> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value)?)
    ))
}

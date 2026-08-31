//! Stable regular-file identity and complete-file hashing for camera media.

use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom};

use record_core::Result;
use sha2::{Digest, Sha256};

#[cfg(target_os = "linux")]
use crate::camera_finalization_anchored::require_read_only_descriptor;
use crate::camera_finalization_error::finalization_error;

#[derive(Debug)]
pub(crate) struct VerifiedRegularFile {
    file: File,
    identity: FileIdentity,
    bytes: u64,
}

#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(not(unix))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileIdentity;

impl VerifiedRegularFile {
    pub(crate) fn file(&self) -> &File {
        &self.file
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn raw_fd(&self) -> std::os::fd::RawFd {
        use std::os::fd::AsRawFd;

        self.file.as_raw_fd()
    }
}

pub(crate) fn from_opened_regular(file: File) -> Result<VerifiedRegularFile> {
    let metadata = file.metadata().map_err(|error| {
        finalization_error(
            "camera media handle cannot be inspected",
            &error.to_string(),
        )
    })?;
    ensure_regular_metadata(&metadata, "camera media handle")?;
    Ok(VerifiedRegularFile {
        identity: identity_from_metadata(&metadata)?,
        bytes: metadata.len(),
        file,
    })
}

/// A closed stage must have no directory entry. Linux `O_TMPFILE` satisfies
/// this invariant and lets the finalizer link the exact descriptor later; an
/// unlinked file cannot be replaced through a staging pathname.
pub(crate) fn verify_nameless_stage(
    file: File,
    capture_filesystem: u64,
) -> Result<VerifiedRegularFile> {
    let verified = from_opened_regular(file)?;
    #[cfg(target_os = "linux")]
    require_read_only_descriptor(verified.file())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let metadata = verified.file.metadata().map_err(|error| {
            finalization_error(
                "camera staged handle cannot be inspected",
                &error.to_string(),
            )
        })?;
        if metadata.dev() != capture_filesystem {
            return Err(finalization_error(
                "camera stage is not on the capture filesystem",
                "capture-owned publication requires an unnamed stage on the anchored filesystem",
            ));
        }
        if metadata.nlink() != 0 {
            return Err(finalization_error(
                "camera stage still has a mutable pathname",
                "close must return an unnamed O_TMPFILE-style handle before finalization",
            ));
        }
        Ok(verified)
    }
    #[cfg(not(unix))]
    {
        let _ = capture_filesystem;
        Err(finalization_error(
            "camera unnamed-stage identity is unavailable on this host",
            "finalization fails closed until the native platform can prove a nameless stage",
        ))
    }
}

pub(crate) fn hash_verified_file(file: &VerifiedRegularFile) -> Result<(String, u64)> {
    verify_handle_identity(file)?;
    let mut reader = file.file.try_clone().map_err(|error| {
        finalization_error(
            "camera media cannot be duplicated for hashing",
            &error.to_string(),
        )
    })?;
    reader.seek(SeekFrom::Start(0)).map_err(|error| {
        finalization_error("camera media cannot seek for hashing", &error.to_string())
    })?;
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 32 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|error| {
            finalization_error(
                "camera media cannot be read for hashing",
                &error.to_string(),
            )
        })?;
        if read == 0 {
            break;
        }
        bytes = bytes.checked_add(read as u64).ok_or_else(|| {
            finalization_error(
                "camera media exceeds the supported hash range",
                "byte count overflow",
            )
        })?;
        hasher.update(&buffer[..read]);
    }
    verify_handle_identity(file)?;
    if bytes != file.bytes {
        return Err(finalization_error(
            "camera media changed while hashing",
            "the verified file length changed during a complete-file hash",
        ));
    }
    Ok((format!("{:x}", hasher.finalize()), bytes))
}

pub(crate) fn same_file_identity(left: &VerifiedRegularFile, right: &VerifiedRegularFile) -> bool {
    left.identity == right.identity
}

fn ensure_regular_metadata(metadata: &Metadata, label: &str) -> Result<()> {
    if !metadata.is_file() || is_windows_reparse_point(metadata) {
        return Err(finalization_error(
            &format!("{label} is not a non-redirected regular file"),
            "camera finalization accepts only direct regular files",
        ));
    }
    if metadata.len() == 0 {
        return Err(finalization_error(
            "camera media file is empty",
            "camera finalization requires a complete non-empty staged file",
        ));
    }
    Ok(())
}

fn verify_handle_identity(file: &VerifiedRegularFile) -> Result<()> {
    let metadata = file.file.metadata().map_err(|error| {
        finalization_error(
            "camera media handle cannot be inspected",
            &error.to_string(),
        )
    })?;
    ensure_regular_metadata(&metadata, "camera media handle")?;
    if identity_from_metadata(&metadata)? != file.identity || metadata.len() != file.bytes {
        return Err(finalization_error(
            "camera media handle identity changed",
            "the finalization handle did not remain stable",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn identity_from_metadata(metadata: &Metadata) -> Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(not(unix))]
fn identity_from_metadata(_metadata: &Metadata) -> Result<FileIdentity> {
    Err(finalization_error(
        "camera media stable identity is unavailable on this host",
        "native finalization fails closed until this host has a supported file-identity adapter",
    ))
}

#[cfg(windows)]
fn is_windows_reparse_point(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    has_windows_reparse_attributes(metadata.file_attributes())
}

#[cfg(windows)]
fn has_windows_reparse_attributes(attributes: u32) -> bool {
    attributes & 0x0000_0400 != 0
}

#[cfg(not(windows))]
fn is_windows_reparse_point(_metadata: &Metadata) -> bool {
    false
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::has_windows_reparse_attributes;

    #[test]
    fn windows_reparse_attributes_are_rejected_before_identity_or_open() {
        assert!(has_windows_reparse_attributes(0x0000_0400));
        assert!(!has_windows_reparse_attributes(0));
    }
}

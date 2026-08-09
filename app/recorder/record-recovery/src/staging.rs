//! Random, exclusively-created local staging for physical capture artifacts.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Component, Path, PathBuf};

const ATTEMPTS: u8 = 32;
/// `StorageFile::GetFileFromPathAsync` in the WGC encoder keeps the legacy
/// `MAX_PATH` contract: this includes the terminating NUL.
pub const WINDOWS_WGC_MAX_UTF16_WITH_NUL: usize = 260;
const WINDOWS_WGC_STAGE_LEAF: &str = "s.mp4";
const WINDOWS_WGC_STAGE_TOKEN_LEN: usize = 22;
const WINDOWS_WGC_BUDGET_TOKEN: &str = "0000000000000000000000";

/// A private directory whose output leaf remains absent for a native encoder to
/// create. The directory is removed only when its one known leaf can be safely
/// unlinked; unexpected contents are retained for inspection.
pub struct PrivateStaging {
    dir: PathBuf,
    leaf: PathBuf,
}

impl PrivateStaging {
    pub fn create(parent: &Path, prefix: &str, leaf: &str) -> io::Result<Self> {
        if !is_safe_leaf(leaf) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "staging output leaf must be one normal path component",
            ));
        }
        ensure_plain_dir(parent)?;
        let dir = reserve_dir(parent, prefix)?;
        Ok(Self {
            leaf: dir.join(leaf),
            dir,
        })
    }

    /// Reserve the compact private stage used only by Windows Graphics Capture.
    ///
    /// WGC's upstream encoder hands its output to a legacy WinRT path API. Its
    /// random private stage must therefore stay compact even when the project
    /// capture root is deliberately deep. The token still carries 128 bits of
    /// entropy; base64url makes that entropy fit in 22 ASCII code units.
    pub fn create_windows_wgc(parent: &Path) -> io::Result<Self> {
        ensure_plain_dir(parent)?;
        let dir = reserve_windows_wgc_dir(parent)?;
        Ok(Self {
            leaf: dir.join(WINDOWS_WGC_STAGE_LEAF),
            dir,
        })
    }

    pub fn path(&self) -> &Path {
        &self.leaf
    }

    /// Remove only this reservation's known leaf and only when the directory is
    /// still local. A non-empty or unexpected directory is deliberately left
    /// behind rather than recursively deleting capture evidence.
    pub fn cleanup(&self) -> io::Result<()> {
        match fs::symlink_metadata(&self.dir) {
            Ok(metadata) if is_plain_dir_metadata(&metadata) => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "private staging directory is no longer local",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        }
        match fs::symlink_metadata(&self.leaf) {
            Ok(metadata)
                if metadata.file_type().is_file()
                    || metadata.file_type().is_symlink()
                    || is_reparse(&metadata) =>
            {
                fs::remove_file(&self.leaf)?;
            }
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "private staging leaf is not a removable file",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::remove_dir(&self.dir)
    }
}

impl Drop for PrivateStaging {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

/// Reserve a random local regular file for writers that can accept an already
/// opened file, such as `hound::WavWriter`. The name is never a predictable
/// sibling `.part` path.
pub fn create_staging_file(parent: &Path, prefix: &str) -> io::Result<(PathBuf, File)> {
    ensure_plain_dir(parent)?;
    for _ in 0..ATTEMPTS {
        let path = parent.join(format!(".{prefix}-{}.part", random_token()?));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve a unique local staging path",
    ))
}

/// The exact native WGC output shape used both for the budget preflight and
/// the real private reservation. Keeping construction here prevents a
/// preflight/reservation off-by-one in separators, leaf name, or NUL accounting.
pub fn windows_wgc_path_budget(parent: &Path) -> WindowsWgcPathBudget {
    let candidate = windows_wgc_stage_path(parent, WINDOWS_WGC_BUDGET_TOKEN);
    WindowsWgcPathBudget {
        utf16_units_with_nul: utf16_units_with_nul(&candidate),
        max_utf16_units_with_nul: WINDOWS_WGC_MAX_UTF16_WITH_NUL,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsWgcPathBudget {
    pub utf16_units_with_nul: usize,
    pub max_utf16_units_with_nul: usize,
}

impl WindowsWgcPathBudget {
    pub fn supported(self) -> bool {
        self.utf16_units_with_nul <= self.max_utf16_units_with_nul
    }
}

fn reserve_windows_wgc_dir(parent: &Path) -> io::Result<PathBuf> {
    for _ in 0..ATTEMPTS {
        let token = random_base64url_128()?;
        let path = windows_wgc_stage_dir(parent, &token);
        match create_private_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve a compact Windows WGC staging directory",
    ))
}

fn windows_wgc_stage_path(parent: &Path, token: &str) -> PathBuf {
    windows_wgc_stage_dir(parent, token).join(WINDOWS_WGC_STAGE_LEAF)
}

fn windows_wgc_stage_dir(parent: &Path, token: &str) -> PathBuf {
    debug_assert_eq!(token.len(), WINDOWS_WGC_STAGE_TOKEN_LEN);
    parent.join(format!("._{token}.d"))
}

#[cfg(windows)]
fn utf16_units_with_nul(path: &Path) -> usize {
    use std::os::windows::ffi::OsStrExt;

    path.as_os_str().encode_wide().count().saturating_add(1)
}

#[cfg(not(windows))]
fn utf16_units_with_nul(path: &Path) -> usize {
    // The boundary regression uses ASCII roots and the same one-code-unit
    // separators/leaf names as Windows. Native Windows uses OsStrExt above.
    path.to_string_lossy()
        .encode_utf16()
        .count()
        .saturating_add(1)
}

fn reserve_dir(parent: &Path, prefix: &str) -> io::Result<PathBuf> {
    for _ in 0..ATTEMPTS {
        let path = parent.join(format!(".{prefix}-{}.stage", random_token()?));
        let created = create_private_dir(&path);
        match created {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve a unique private staging directory",
    ))
}

#[cfg(unix)]
fn create_private_dir(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder.create(path)
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir(path)
}

fn ensure_plain_dir(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if is_plain_dir_metadata(&metadata) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "staging parent is not a local directory",
        ))
    }
}

fn is_plain_dir_metadata(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_dir() && !metadata.file_type().is_symlink() && !is_reparse(metadata)
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

fn is_safe_leaf(value: &str) -> bool {
    matches!(
        Path::new(value).components().next(),
        Some(Component::Normal(_))
    ) && Path::new(value).components().count() == 1
}

fn random_token() -> io::Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| io::Error::other(format!("obtain staging randomness: {error}")))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn random_base64url_128() -> io::Result<String> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| io::Error::other(format!("obtain staging randomness: {error}")))?;
    let mut token = String::with_capacity(WINDOWS_WGC_STAGE_TOKEN_LEN);
    for chunk in bytes[..15].chunks_exact(3) {
        token.push(ALPHABET[usize::from(chunk[0] >> 2)] as char);
        token.push(ALPHABET[usize::from((chunk[0] & 0x03) << 4 | chunk[1] >> 4)] as char);
        token.push(ALPHABET[usize::from((chunk[1] & 0x0f) << 2 | chunk[2] >> 6)] as char);
        token.push(ALPHABET[usize::from(chunk[2] & 0x3f)] as char);
    }
    let tail = bytes[15];
    token.push(ALPHABET[usize::from(tail >> 2)] as char);
    token.push(ALPHABET[usize::from((tail & 0x03) << 4)] as char);
    debug_assert_eq!(token.len(), WINDOWS_WGC_STAGE_TOKEN_LEN);
    Ok(token)
}

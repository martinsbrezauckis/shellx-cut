//! Portable matte cache leaves shared by the bake writer and renderer.

use crate::{error_codes, CutError};
use std::path::{Path, PathBuf};

fn unsafe_entry(path: &Path) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "unsafe matte cache entry",
        format!(
            "matte cache requires plain unlinked local entries: {}",
            path.display()
        ),
    )
}
#[cfg(test)]
#[path = "matte_cache_filesystem_tests.rs"]
mod filesystem_tests;

fn reparse(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return metadata.file_attributes() & 0x400 != 0;
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

pub fn require_plain_directory(path: &Path) -> Result<(), CutError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() && !reparse(&metadata) {
        return Ok(());
    }
    Err(unsafe_entry(path))
}

/// Validate every project-owned parent at the time of use, optionally creating
/// missing cache directories one level at a time without following links.
pub fn cache_dir(project: &Path, create: bool) -> Result<PathBuf, CutError> {
    let dir = project.join("cache").join("matte");
    for path in [project.to_owned(), project.join("cache"), dir.clone()] {
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && path != project => {
                if !create {
                    continue;
                }
                #[cfg(unix)]
                let mut builder = std::fs::DirBuilder::new();
                #[cfg(not(unix))]
                let builder = std::fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                match builder.create(&path) {
                    Ok(()) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                    Err(e) => return Err(e.into()),
                }
                std::fs::symlink_metadata(&path)?
            }
            Err(e) => return Err(e.into()),
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() || reparse(&metadata) {
            return Err(unsafe_entry(&path));
        }
    }
    Ok(dir)
}

/// A cache hit or publication destination must be a plain singly-linked file.
/// NotFound is distinct from a dangling link, which symlink_metadata observes.
pub fn plain_file_exists(path: &Path) -> Result<bool, CutError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || reparse(&metadata) {
        return Err(unsafe_entry(path));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(unsafe_entry(path));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let file = std::fs::File::open(path)?;
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: file owns a live handle and info is the exact writable ABI.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if info.nNumberOfLinks != 1 {
            return Err(unsafe_entry(path));
        }
    }
    Ok(true)
}

/// Writer/reader agreement: new paths are portable. Unix-only known-prefix
/// legacy leaves remain readable offline; Windows never resolves colon/ADS.
/// Older Windows colon entries require a normal source rebake. No project
/// asset identity or media path is changed to migrate generated cache state.
pub fn alpha_path(
    project: &Path,
    hash: &str,
    matte: &crate::ClipMatte,
) -> Result<PathBuf, CutError> {
    validate_asset_hash(hash)?;
    let dir = cache_dir(project, false)?;
    let portable = dir.join(matte.cache_filename(hash));
    if plain_file_exists(&portable)? {
        return Ok(portable);
    }
    if let Some(legacy) = legacy_alpha_path(&dir, hash, matte) {
        if plain_file_exists(&legacy)? {
            return Ok(legacy);
        }
    }
    Ok(portable)
}

pub fn legacy_alpha_path(dir: &Path, hash: &str, matte: &crate::ClipMatte) -> Option<PathBuf> {
    #[cfg(unix)]
    {
        validate_asset_hash(hash).ok()?;
        let (prefix, digest) = hash.split_once(':')?;
        if !matches!(prefix, "sha256" | "sha256s") {
            return None;
        }
        let portable = matte.cache_filename(hash);
        return Some(dir.join(portable.replacen(&format!("{prefix}={digest}"), hash, 1)));
    }
    #[cfg(not(unix))]
    {
        let _ = (dir, hash, matte);
        None
    }
}

/// Keep safe historical tokens and raw digests unchanged. The hash producers'
/// `sha256:` and sampled `sha256s:` spellings use a portable equals in filenames;
/// project asset identities themselves are never modified.
pub fn portable_asset_key(hash: &str) -> String {
    for prefix in ["sha256:", "sha256s:"] {
        if let Some(digest) = hash.strip_prefix(prefix) {
            return format!("{}={digest}", prefix.trim_end_matches(':'));
        }
    }
    hash.to_owned()
}

pub fn validate_asset_hash(hash: &str) -> Result<(), CutError> {
    let key = portable_asset_key(hash);
    let prefixed = hash
        .strip_prefix("sha256:")
        .or_else(|| hash.strip_prefix("sha256s:"));
    let token = !key.is_empty()
        && key.len() <= 160
        && key.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(b, b'.' | b'_' | b'-')
                || (prefixed.is_some() && b == b'=')
        })
        && !key.starts_with('.')
        && !key.ends_with('.')
        && prefixed.is_none_or(|digest| {
            !digest.is_empty()
                && digest
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        });
    let stem = key.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix)
                .is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'))
        });
    if token && !reserved {
        return Ok(());
    }
    Err(CutError::new(
        error_codes::INVALID_ARGS,
        "unsafe asset hash for matte cache",
        "asset hashes must identify one portable cache leaf",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_and_safe_historical_keys_keep_their_identity() {
        for hash in [
            "abc123",
            "asset-hash_1",
            "legacy.hash",
            "sha256:a2",
            "sha256s:abcd",
            "sha256:subject",
        ] {
            validate_asset_hash(hash).unwrap();
        }
        assert_eq!(portable_asset_key("abc123"), "abc123");
        assert_eq!(portable_asset_key("sha256:a2"), "sha256=a2");
        assert_eq!(portable_asset_key("sha256s:abcd"), "sha256s=abcd");
        assert_ne!(
            portable_asset_key("sha256:a2"),
            portable_asset_key("sha256-a2")
        );
        assert_ne!(
            portable_asset_key("sha256s:abcd"),
            portable_asset_key("sha256s-abcd")
        );
        assert!(validate_asset_hash("sha256=a2").is_err());
    }

    #[test]
    fn path_aliases_device_names_and_stream_syntax_are_rejected() {
        for hash in [
            "",
            ".",
            "..",
            "../../outside",
            "/outside",
            "C:\\outside",
            "a\\..\\outside",
            "a/b",
            "a:stream",
            "sha256:a2:stream",
            "sha256:../a",
            "CON",
            "nul.txt",
            "LPT1",
            "a\0b",
            "a\nb",
            "a.",
        ] {
            assert!(validate_asset_hash(hash).is_err(), "{hash:?}");
        }
    }
}

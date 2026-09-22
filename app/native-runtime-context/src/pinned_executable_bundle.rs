//! Strict typed consumer for Runner-pinned external executable bundles.

use crate::{
    canonical_root, clean_absolute, context_io::read_context, directory_no_link,
    no_reparse_ancestors, regular_no_link, sha, CONTEXT_ENV, MAX_FILES,
    PINNED_EXECUTABLE_BUNDLE_CONTEXT_CONTRACT,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const MAX_EXECUTABLES: usize = 32;
const MAX_TOTAL_BYTES: u64 = 32 * 1024 * 1024 * 1024;

/// An opaque Runner-admitted executable identity. Consumers choose their own
/// names and must verify an entry before using its path to start a child.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PinnedExecutable {
    pub name: String,
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
    pub mode: u32,
}

/// Typed consumer for a Runner-pinned executable bundle. This is deliberately
/// separate from the Python runtime v1 context and its stable byte shape.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PinnedExecutableBundleContext {
    pub schema: String,
    pub root: PathBuf,
    pub manifest_sha256: String,
    pub receipt_sha256: String,
    pub platform: String,
    pub architecture: String,
    pub executables: Vec<PinnedExecutable>,
    pub files: usize,
    pub total_bytes: u64,
}

impl PinnedExecutableBundleContext {
    /// A supplied locator that is malformed, wrong-host, or not this contract
    /// is an error; callers must not fall through to ambient tool discovery.
    pub fn from_env() -> Result<Option<Self>, String> {
        match std::env::var_os(CONTEXT_ENV) {
            None => Ok(None),
            Some(path) if path.is_empty() => {
                Err("pinned executable bundle context locator is empty".into())
            }
            Some(path) => Self::from_path(Path::new(&path)).map(Some),
        }
    }

    pub fn from_path(path: &Path) -> Result<Self, String> {
        let bytes = read_context(path)?;
        let context: Self = serde_json::from_slice(&bytes)
            .map_err(|e| format!("parse pinned executable bundle context: {e}"))?;
        context.validate()?;
        Ok(context)
    }

    pub(crate) fn from_value(value: serde_json::Value) -> Result<Self, String> {
        let context: Self = serde_json::from_value(value)
            .map_err(|e| format!("parse pinned executable bundle context: {e}"))?;
        context.validate()?;
        Ok(context)
    }

    /// Select a product-owned executable name. Runner leaves names opaque.
    pub fn executable(&self, name: &str) -> Option<&PinnedExecutable> {
        self.executables
            .iter()
            .find(|executable| executable.name == name)
    }

    /// Re-check one selected executable immediately before it is passed to a
    /// child. The stream never loads its full contents into memory.
    pub fn verify_executable(&self, executable: &PinnedExecutable) -> Result<(), String> {
        let canonical_root = canonical_root(&self.root)?;
        self.validate_path(&canonical_root, executable)?;
        let metadata = fs::metadata(&executable.path)
            .map_err(|e| format!("inspect pinned executable {}: {e}", executable.name))?;
        if metadata.len() != executable.bytes
            || !executable_mode_matches(&metadata, executable.mode)
        {
            return Err(format!(
                "pinned executable {} bytes or mode differs from context",
                executable.name
            ));
        }
        let mut file = File::open(&executable.path)
            .map_err(|e| format!("open pinned executable {}: {e}", executable.name))?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|e| format!("read pinned executable {}: {e}", executable.name))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        if format!("{:x}", hasher.finalize()) != executable.sha256 {
            return Err(format!(
                "pinned executable {} hash differs from context",
                executable.name
            ));
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema != PINNED_EXECUTABLE_BUNDLE_CONTEXT_CONTRACT
            || !self.root.is_absolute()
            || !clean_absolute(&self.root)
            || self.platform != std::env::consts::OS
            || self.architecture != std::env::consts::ARCH
            || !sha(&self.manifest_sha256)
            || !sha(&self.receipt_sha256)
            || self.files == 0
            || self.files > MAX_FILES
            || self.total_bytes == 0
            || self.total_bytes > MAX_TOTAL_BYTES
            || self.executables.is_empty()
            || self.executables.len() > MAX_EXECUTABLES
            || self.files < self.executables.len()
        {
            return Err(
                "pinned executable bundle context has an invalid contract or bounds".into(),
            );
        }
        directory_no_link(&self.root, "pinned executable bundle root")?;
        let canonical_root = fs::canonicalize(&self.root)
            .map_err(|e| format!("canonicalize pinned executable bundle root: {e}"))?;
        let mut names = BTreeSet::new();
        let mut paths = BTreeSet::new();
        let mut executable_bytes = 0_u64;
        for executable in &self.executables {
            if !executable_name(&executable.name)
                || !names.insert(&executable.name)
                || !paths.insert(&executable.path)
                || executable.bytes == 0
                || executable.mode != 0o755
            {
                return Err(
                    "pinned executable bundle entries must be unique executable identities".into(),
                );
            }
            executable_bytes = executable_bytes
                .checked_add(executable.bytes)
                .ok_or("pinned executable bundle byte count overflow")?;
            self.validate_path(&canonical_root, executable)?;
        }
        if executable_bytes > self.total_bytes {
            return Err("pinned executable bundle entries exceed declared total bytes".into());
        }
        Ok(())
    }

    fn validate_path(
        &self,
        canonical_root: &Path,
        executable: &PinnedExecutable,
    ) -> Result<(), String> {
        let path = &executable.path;
        if !sha(&executable.sha256) || !path.is_absolute() || !path.starts_with(&self.root) {
            return Err(format!(
                "pinned executable {} path or hash is invalid",
                executable.name
            ));
        }
        let relative = path.strip_prefix(&self.root).map_err(|_| {
            format!(
                "pinned executable {} escapes the admitted root",
                executable.name
            )
        })?;
        if relative.as_os_str().is_empty()
            || relative.components().any(|part| {
                matches!(
                    part,
                    Component::CurDir
                        | Component::ParentDir
                        | Component::RootDir
                        | Component::Prefix(_)
                )
            })
        {
            return Err(format!(
                "pinned executable {} is not a child of the admitted root",
                executable.name
            ));
        }
        regular_no_link(path, &format!("pinned executable {}", executable.name))?;
        no_reparse_ancestors(
            &self.root,
            relative,
            &format!("executable {}", executable.name),
        )?;
        let canonical = fs::canonicalize(path)
            .map_err(|e| format!("canonicalize pinned executable {}: {e}", executable.name))?;
        if !canonical.starts_with(canonical_root) {
            return Err(format!(
                "pinned executable {} path changed or escapes its root",
                executable.name
            ));
        }
        Ok(())
    }
}

fn executable_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes[0].is_ascii_alphabetic()
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-'))
}

#[cfg(unix)]
fn executable_mode_matches(metadata: &fs::Metadata, expected: u32) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o777 == expected
}

#[cfg(not(unix))]
fn executable_mode_matches(_: &fs::Metadata, expected: u32) -> bool {
    // Runner inventory carries 0755 as a cross-platform executable semantic.
    expected == 0o755
}

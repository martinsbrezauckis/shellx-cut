//! Typed consumer for a Release Runner prepared native runtime.
//!
//! The Runner owns admission, fresh materialization, and final verification.
//! Cut uses this crate to reject a malformed locator and to select only the
//! interpreter/import/model identities in the sealed context. It deliberately
//! contains no sidecar script or product model policy.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub const CONTEXT_ENV: &str = "RELEASE_RUNNER_NATIVE_RUNTIME_CONTEXT";
pub const CONTEXT_CONTRACT: &str = "release-runner.native-runtime-context/v1";
pub const CONTEXT_MAX_BYTES: u64 = 1024 * 1024;
const MAX_FILES: usize = 131_072;
const MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_IMPORTS: usize = 256;
const MAX_MODELS: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeContext {
    pub schema: String,
    pub root: PathBuf,
    pub manifest_sha256: String,
    pub receipt_sha256: String,
    pub interpreter: Interpreter,
    pub imports: Vec<Import>,
    pub models: Vec<Model>,
    pub files: usize,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Interpreter {
    pub path: PathBuf,
    pub sha256: String,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Import {
    pub module: String,
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Model {
    pub id: String,
    pub path: PathBuf,
    pub sha256: String,
    pub provenance_sha256: String,
}

impl RuntimeContext {
    /// Load the Runner-owned locator when present. An explicitly supplied but
    /// invalid locator is an error: consumers must never fall through to a
    /// host Python or model path in that case.
    pub fn from_env() -> Result<Option<Self>, String> {
        match std::env::var_os(CONTEXT_ENV) {
            None => Ok(None),
            Some(path) if path.is_empty() => Err("native runtime context locator is empty".into()),
            Some(path) => Self::from_path(Path::new(&path)).map(Some),
        }
    }

    pub fn from_path(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() {
            return Err("native runtime context locator must be absolute".into());
        }
        regular_no_link(path, "native runtime context")?;
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(|e| format!("open native runtime context: {e}"))?
            .take(CONTEXT_MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("read native runtime context: {e}"))?;
        if bytes.len() as u64 > CONTEXT_MAX_BYTES {
            return Err("native runtime context exceeds 1 MiB".into());
        }
        let context: Self = serde_json::from_slice(&bytes)
            .map_err(|e| format!("parse native runtime context: {e}"))?;
        context.validate()?;
        Ok(context)
    }

    /// Re-hash the selected interpreter through a bounded streaming read.
    pub fn verify_interpreter(&self) -> Result<(), String> {
        self.verify_asset(
            &self.interpreter.path,
            &self.interpreter.sha256,
            "interpreter",
        )
    }

    /// Re-hash all declared import origins before a generic Python sidecar is
    /// launched. Product-specific consumers select and verify their own models.
    pub fn verify_imports(&self) -> Result<(), String> {
        for import in &self.imports {
            self.verify_asset(&import.path, &import.sha256, "import")?;
        }
        Ok(())
    }

    /// Look up a product-owned semantic model ID. Callers must invoke
    /// [`verify_model`] before passing its path to Python.
    pub fn model(&self, id: &str) -> Option<&Model> {
        self.models.iter().find(|model| model.id == id)
    }

    /// Re-hash one selected model without ever reading its full contents into
    /// memory. This intentionally supports models larger than the context bound.
    pub fn verify_model(&self, model: &Model) -> Result<(), String> {
        self.verify_asset(&model.path, &model.sha256, "model")
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema != CONTEXT_CONTRACT
            || !self.root.is_absolute()
            || !clean_absolute(&self.root)
            || !sha(&self.manifest_sha256)
            || !sha(&self.receipt_sha256)
            || self.files == 0
            || self.files > MAX_FILES
            || self.total_bytes > MAX_TOTAL_BYTES
            || self.imports.is_empty()
            || self.imports.len() > MAX_IMPORTS
            || self.models.len() > MAX_MODELS
        {
            return Err("native runtime context has an invalid contract or bounds".into());
        }
        directory_no_link(&self.root, "native runtime root")?;
        let canonical_root = fs::canonicalize(&self.root)
            .map_err(|e| format!("canonicalize native runtime root: {e}"))?;
        self.validate_path(
            &canonical_root,
            &self.interpreter.path,
            &self.interpreter.sha256,
            "interpreter",
        )?;
        if !python_version(&self.interpreter.version) {
            return Err("native runtime interpreter version must be MAJOR.MINOR.PATCH".into());
        }
        let mut imports = BTreeSet::new();
        for import in &self.imports {
            if !label(&import.module)
                || !import.module.split('.').all(identifier)
                || !imports.insert(&import.module)
            {
                return Err(
                    "native runtime import names must be unique Python module names".into(),
                );
            }
            self.validate_path(&canonical_root, &import.path, &import.sha256, "import")?;
        }
        let mut models = BTreeSet::new();
        for model in &self.models {
            if !label(&model.id) || !models.insert(&model.id) || !sha(&model.provenance_sha256) {
                return Err("native runtime model identities must be unique and pinned".into());
            }
            self.validate_path(&canonical_root, &model.path, &model.sha256, "model")?;
        }
        Ok(())
    }

    fn validate_path(
        &self,
        canonical_root: &Path,
        path: &Path,
        expected: &str,
        label: &str,
    ) -> Result<(), String> {
        if !sha(expected) || !path.is_absolute() || !path.starts_with(&self.root) {
            return Err(format!("native runtime {label} path or hash is invalid"));
        }
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| format!("native runtime {label} escapes the admitted root"))?;
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
                "native runtime {label} is not a child of the admitted root"
            ));
        }
        regular_no_link(path, &format!("native runtime {label}"))?;
        no_reparse_ancestors(&self.root, relative, label)?;
        let canonical = fs::canonicalize(path)
            .map_err(|e| format!("canonicalize native runtime {label}: {e}"))?;
        if !canonical.starts_with(canonical_root) {
            return Err(format!(
                "native runtime {label} path changed or escapes its root"
            ));
        }
        Ok(())
    }

    fn verify_asset(&self, path: &Path, expected: &str, label: &str) -> Result<(), String> {
        let canonical_root = canonical_root(&self.root)?;
        self.validate_path(&canonical_root, path, expected, label)?;
        let mut file = File::open(path).map_err(|e| format!("open native runtime {label}: {e}"))?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|e| format!("read native runtime {label}: {e}"))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        let actual = format!("{:x}", hasher.finalize());
        if actual != expected {
            return Err(format!("native runtime {label} hash differs from context"));
        }
        Ok(())
    }
}

fn regular_no_link(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("inspect {label}: {e}"))?;
    if metadata.file_type().is_symlink() || reparse_point(&metadata) || !metadata.is_file() {
        return Err(format!("{label} must be a regular non-link file"));
    }
    Ok(())
}

fn directory_no_link(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("inspect {label}: {e}"))?;
    if metadata.file_type().is_symlink() || reparse_point(&metadata) || !metadata.is_dir() {
        return Err(format!("{label} must be a non-link directory"));
    }
    Ok(())
}

fn canonical_root(root: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(root).map_err(|e| format!("canonicalize native runtime root: {e}"))
}

fn clean_absolute(path: &Path) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
}

fn no_reparse_ancestors(root: &Path, relative: &Path, label: &str) -> Result<(), String> {
    let mut current = root.to_path_buf();
    for part in relative.components() {
        let Component::Normal(name) = part else {
            return Err(format!("native runtime {label} is not a clean root child"));
        };
        current.push(name);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|e| format!("inspect native runtime {label}: {e}"))?;
        if metadata.file_type().is_symlink() || reparse_point(&metadata) {
            return Err(format!(
                "native runtime {label} contains a link or reparse point"
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    windows_reparse_attributes(metadata.file_attributes())
}

#[cfg(windows)]
fn windows_reparse_attributes(attributes: u32) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn reparse_point(_: &fs::Metadata) -> bool {
    false
}

fn sha(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn label(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn python_version(value: &str) -> bool {
    let parts: Vec<_> = value.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

#[cfg(test)]
mod tests;

//! Product-owned admission for the prepared premium MatAnyone2 matte runtime.
//!
//! This resolver seals the RVM bootstrap, MatAnyone2 checkpoint, SAM2 picker,
//! and bundled runners into one context-verified group. It never resolves a
//! mutable setting, app-data asset, hub cache, or network fallback.

use super::native_runtime::{
    prepared_error, regular_file, RVM_FILENAME, RVM_MODEL_ID, RVM_MODEL_SHA256,
};
use cut_core::CutError;
use cut_native_runtime_context::RuntimeContext;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(crate) const MATANYONE_MODEL_ID: &str = "cut.matte.matanyone/matanyone2.pth";
pub(crate) const MATANYONE_MODEL_SHA256: &str =
    "5e9821e4087231427376b437c85bb6e072b41e582314f06fd524f75bc4af5914";
const MATANYONE_FILENAME: &str = "matanyone2.pth";

pub(crate) const SAM2_MODEL_ID: &str = "cut.matte.sam2/sam2_hiera_base_plus.pt";
pub(crate) const SAM2_MODEL_SHA256: &str =
    "d0bb7f236400a49669ffdd1be617959a8b1d1065081789d7bbff88eded3a8071";
const SAM2_FILENAME: &str = "sam2_hiera_base_plus.pt";

/// Immutable identity required before a prepared premium matte cache entry can
/// be reused. MatAnyone2 always binds its RVM bootstrap, checkpoint, and SAM2
/// selection model as one sealed group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedMatanyoneBinding {
    pub(crate) contract: String,
    pub(crate) manifest_sha256: String,
    pub(crate) receipt_sha256: String,
    pub(crate) rvm_model_id: String,
    pub(crate) rvm_model_sha256: String,
    pub(crate) matanyone_model_id: String,
    pub(crate) matanyone_model_sha256: String,
    pub(crate) sam2_model_id: String,
    pub(crate) sam2_model_sha256: String,
}

/// The context-selected premium runtime. Every path comes from the admitted
/// context or installed payload; no mutable selection participates.
#[derive(Debug, Clone)]
pub(crate) struct PreparedMatanyoneRuntime {
    pub(crate) python: PathBuf,
    pub(crate) rvm_script: PathBuf,
    pub(crate) matanyone_script: PathBuf,
    pub(crate) sam2_script: PathBuf,
    pub(crate) rvm_model: PathBuf,
    pub(crate) matanyone_model: PathBuf,
    pub(crate) sam2_model: PathBuf,
    pub(crate) binding: PreparedMatanyoneBinding,
}

pub(crate) fn prepared_matanyone_from_context(
    context: &RuntimeContext,
    python: &Path,
    instruments: &Path,
) -> Result<PreparedMatanyoneRuntime, CutError> {
    prepared_matanyone_from_context_with_hashes(
        context,
        python,
        instruments,
        RVM_MODEL_SHA256,
        MATANYONE_MODEL_SHA256,
        SAM2_MODEL_SHA256,
    )
}

pub(crate) fn prepared_matanyone_from_context_with_hashes(
    context: &RuntimeContext,
    python: &Path,
    instruments: &Path,
    rvm_hash: &str,
    matanyone_hash: &str,
    sam2_hash: &str,
) -> Result<PreparedMatanyoneRuntime, CutError> {
    let rvm_model = prepared_model(context, RVM_MODEL_ID, RVM_FILENAME, rvm_hash, "RVM")?;
    let matanyone_model = prepared_model(
        context,
        MATANYONE_MODEL_ID,
        MATANYONE_FILENAME,
        matanyone_hash,
        "MatAnyone2",
    )?;
    let sam2_model = prepared_model(context, SAM2_MODEL_ID, SAM2_FILENAME, sam2_hash, "SAM2")?;

    let rvm_script = bundled_script(instruments, "matte_runner.py")?;
    let matanyone_script = bundled_script(instruments, "matanyone_runner.py")?;
    let sam2_script = bundled_script(instruments, "sam2_runner.py")?;

    Ok(PreparedMatanyoneRuntime {
        python: python.to_path_buf(),
        rvm_script,
        matanyone_script,
        sam2_script,
        rvm_model: rvm_model.path.clone(),
        matanyone_model: matanyone_model.path.clone(),
        sam2_model: sam2_model.path.clone(),
        binding: PreparedMatanyoneBinding {
            contract: context.schema.clone(),
            manifest_sha256: context.manifest_sha256.clone(),
            receipt_sha256: context.receipt_sha256.clone(),
            rvm_model_id: rvm_model.id.clone(),
            rvm_model_sha256: rvm_model.sha256.clone(),
            matanyone_model_id: matanyone_model.id.clone(),
            matanyone_model_sha256: matanyone_model.sha256.clone(),
            sam2_model_id: sam2_model.id.clone(),
            sam2_model_sha256: sam2_model.sha256.clone(),
        },
    })
}

fn prepared_model<'a>(
    context: &'a RuntimeContext,
    id: &str,
    filename: &str,
    expected_hash: &str,
    label: &str,
) -> Result<&'a cut_native_runtime_context::Model, CutError> {
    let model = context.model(id).ok_or_else(|| {
        prepared_error(format!("prepared native runtime has no {label} asset {id}"))
    })?;
    if model.sha256 != expected_hash {
        return Err(prepared_error(format!(
            "prepared {label} asset hash is not the Cut-pinned model"
        )));
    }
    if model.path.file_name().and_then(|name| name.to_str()) != Some(filename) {
        return Err(prepared_error(format!(
            "prepared {label} asset filename is invalid"
        )));
    }
    context.verify_model(model).map_err(prepared_error)?;
    Ok(model)
}

fn bundled_script(instruments: &Path, name: &str) -> Result<PathBuf, CutError> {
    instruments
        .parent()
        .map(|dir| dir.join(name))
        .filter(|path| regular_file(path))
        .ok_or_else(|| prepared_error(format!("prepared native runtime requires bundled {name}")))
}

#[cfg(test)]
#[path = "matte_premium_runtime_tests.rs"]
mod test_contracts;

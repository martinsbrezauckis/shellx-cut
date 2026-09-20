//! Product-owned admission for the prepared standard RVM matte runtime.
//!
//! The Runner verifies the generic interpreter/import inventory. This module
//! selects Cut's one semantic RVM asset and the bundled runner beside the
//! already-admitted `instruments.py`; it deliberately does not select any
//! mutable matte setting, environment override, app-data file, or HTTP route.

use super::premium_runtime::{PreparedMatanyoneBinding, PreparedMatanyoneRuntime};
use cut_core::{error_codes, CutError, MatteModel};
use cut_native_runtime_context::RuntimeContext;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(crate) const RVM_MODEL_ID: &str = "cut.matte.rvm/rvm_mobilenetv3_fp32.onnx";
pub(crate) const RVM_MODEL_SHA256: &str =
    "88d4531297118f595bf2fd60f6f566aec2e559393802d1f436c380f0cbbd2828";
pub(crate) const RVM_FILENAME: &str = "rvm_mobilenetv3_fp32.onnx";

/// Immutable identity required before a prepared RVM cache entry can be reused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedRvmBinding {
    pub(crate) contract: String,
    pub(crate) manifest_sha256: String,
    pub(crate) receipt_sha256: String,
    pub(crate) model_id: String,
    pub(crate) model_sha256: String,
}

/// The prepared runtime identity stored with a matte cache receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PreparedMatteBinding {
    Rvm(PreparedRvmBinding),
    Matanyone(PreparedMatanyoneBinding),
}

/// The context-selected local RVM runner. Every path comes from the admitted
/// context or the installed application payload; no mutable selection is read.
#[derive(Debug, Clone)]
pub(crate) struct PreparedRvmRuntime {
    pub(crate) python: PathBuf,
    pub(crate) script: PathBuf,
    pub(crate) model: PathBuf,
    pub(crate) binding: PreparedRvmBinding,
}

#[derive(Debug, Clone)]
pub(crate) enum PreparedMatteRuntime {
    Rvm(PreparedRvmRuntime),
    Matanyone(PreparedMatanyoneRuntime),
}

impl PreparedMatteRuntime {
    pub(crate) fn binding(&self) -> PreparedMatteBinding {
        match self {
            Self::Rvm(runtime) => PreparedMatteBinding::Rvm(runtime.binding.clone()),
            Self::Matanyone(runtime) => PreparedMatteBinding::Matanyone(runtime.binding.clone()),
        }
    }
}

/// Resolve a prepared RVM runtime if, and only if, a native context was supplied.
/// A supplied invalid or incomplete context is an error rather than a normal
/// local/HTTP fallback.
pub(crate) fn prepared_rvm_runtime() -> Result<Option<PreparedRvmRuntime>, CutError> {
    let sidecar = cut_perception::sidecar_runtime()?;
    let Some(context) = sidecar.native_context.as_ref() else {
        return Ok(None);
    };
    prepared_rvm_from_context(context, &sidecar.python, &sidecar.script).map(Some)
}

/// Resolve the runtime for the requested matte model. A native context is
/// authoritative: incomplete premium groups fail closed instead of reading the
/// ordinary app-data setup, user overrides, or the HTTP sidecar.
pub(crate) fn prepared_matte_runtime(
    model: &MatteModel,
) -> Result<Option<PreparedMatteRuntime>, CutError> {
    let sidecar = cut_perception::sidecar_runtime()?;
    let Some(context) = sidecar.native_context.as_ref() else {
        return Ok(None);
    };
    match model {
        MatteModel::Rvm => prepared_rvm_from_context(context, &sidecar.python, &sidecar.script)
            .map(PreparedMatteRuntime::Rvm)
            .map(Some),
        MatteModel::Matanyone => super::premium_runtime::prepared_matanyone_from_context(
            context,
            &sidecar.python,
            &sidecar.script,
        )
        .map(PreparedMatteRuntime::Matanyone)
        .map(Some),
    }
}

fn prepared_rvm_from_context(
    context: &RuntimeContext,
    python: &Path,
    instruments: &Path,
) -> Result<PreparedRvmRuntime, CutError> {
    prepared_rvm_from_context_with_hash(context, python, instruments, RVM_MODEL_SHA256)
}

fn prepared_rvm_from_context_with_hash(
    context: &RuntimeContext,
    python: &Path,
    instruments: &Path,
    expected_hash: &str,
) -> Result<PreparedRvmRuntime, CutError> {
    let model = context.model(RVM_MODEL_ID).ok_or_else(|| {
        prepared_error(format!(
            "prepared native runtime has no RVM asset {RVM_MODEL_ID}"
        ))
    })?;
    if model.sha256 != expected_hash {
        return Err(prepared_error(
            "prepared RVM asset hash is not the Cut-pinned model",
        ));
    }
    if model.path.file_name().and_then(|name| name.to_str()) != Some(RVM_FILENAME) {
        return Err(prepared_error("prepared RVM asset filename is invalid"));
    }
    context.verify_model(model).map_err(prepared_error)?;

    let script = instruments.parent().map(|dir| dir.join("matte_runner.py"));
    let script = script.filter(|path| regular_file(path)).ok_or_else(|| {
        prepared_error("prepared native runtime requires bundled matte_runner.py")
    })?;

    Ok(PreparedRvmRuntime {
        python: python.to_path_buf(),
        script,
        model: model.path.clone(),
        binding: PreparedRvmBinding {
            contract: context.schema.clone(),
            manifest_sha256: context.manifest_sha256.clone(),
            receipt_sha256: context.receipt_sha256.clone(),
            model_id: model.id.clone(),
            model_sha256: model.sha256.clone(),
        },
    })
}

pub(crate) fn regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| !metadata.file_type().is_symlink() && metadata.is_file())
        .unwrap_or(false)
}

pub(crate) fn prepared_error(reason: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::SIDECAR,
        format!("prepared native runtime rejected: {}", reason.into()),
        "repair or reprepare the declared native runtime and its required model inputs before using Background Removal",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cut_native_runtime_context::{Interpreter, Model, CONTEXT_CONTRACT};
    use sha2::{Digest, Sha256};

    fn hash(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn fixture() -> (tempfile::TempDir, RuntimeContext, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("runtime");
        let payload = temp.path().join("payload");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&payload).unwrap();
        let python = root.join("python");
        let model = root.join(RVM_FILENAME);
        let instruments = payload.join("instruments.py");
        let runner = payload.join("matte_runner.py");
        std::fs::write(&python, b"python").unwrap();
        std::fs::write(&model, b"rvm").unwrap();
        std::fs::write(&instruments, b"instruments").unwrap();
        std::fs::write(&runner, b"runner").unwrap();
        let context = RuntimeContext {
            schema: CONTEXT_CONTRACT.into(),
            root: root.clone(),
            manifest_sha256: "a".repeat(64),
            receipt_sha256: "b".repeat(64),
            interpreter: Interpreter {
                path: python.clone(),
                sha256: hash(b"python"),
                version: "3.12.13".into(),
            },
            imports: vec![cut_native_runtime_context::Import {
                module: "onnxruntime".into(),
                path: root.join("onnxruntime.py"),
                sha256: hash(b"onnxruntime"),
            }],
            models: vec![Model {
                id: RVM_MODEL_ID.into(),
                path: model,
                sha256: hash(b"rvm"),
                provenance_sha256: "c".repeat(64),
            }],
            files: 3,
            total_bytes: 32,
        };
        std::fs::write(root.join("onnxruntime.py"), b"onnxruntime").unwrap();
        (temp, context, python, instruments)
    }

    #[test]
    fn admits_only_the_pinned_rvm_asset_and_bundled_runner() {
        let (_temp, context, python, instruments) = fixture();
        let expected = hash(b"rvm");
        let error = prepared_rvm_from_context(&context, &python, &instruments).unwrap_err();
        assert!(error.message.contains("Cut-pinned model"));
        let runtime =
            prepared_rvm_from_context_with_hash(&context, &python, &instruments, &expected)
                .unwrap();
        assert_eq!(runtime.model, context.models[0].path);
        assert_eq!(
            runtime.script,
            instruments.parent().unwrap().join("matte_runner.py")
        );

        let (_temp, mut context, python, instruments) = fixture();
        let model = context.models.pop().unwrap();
        let wrong = Model {
            id: "cut.matte.rvm/other.onnx".into(),
            ..model
        };
        context.models.push(wrong);
        assert!(
            prepared_rvm_from_context_with_hash(&context, &python, &instruments, &expected)
                .unwrap_err()
                .message
                .contains("no RVM asset")
        );
    }

    #[test]
    fn rejects_missing_runner_without_a_legacy_path() {
        let (_temp, context, python, instruments) = fixture();
        let expected = hash(b"rvm");
        std::fs::remove_file(instruments.parent().unwrap().join("matte_runner.py")).unwrap();
        let error = prepared_rvm_from_context_with_hash(&context, &python, &instruments, &expected)
            .unwrap_err();
        assert!(error.message.contains("matte_runner.py"));
    }
}

#[cfg(test)]
#[path = "matte_native_runtime_tests.rs"]
mod test_contracts;

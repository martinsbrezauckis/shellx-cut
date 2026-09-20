//! SigLIP runtime selection and prepared-index identity.

use super::EmbeddingIndex;
use cut_core::{error_codes, CutError};
use cut_native_runtime_context::RuntimeContext;
use cut_perception::{
    apply_python_command_policy, native_runtime_context, sidecar_runtime, SidecarRuntime,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const SIGLIP_MODEL_ID: &str = "google/siglip2-base-patch16-224";
pub(super) const SIGLIP_MODEL_GROUP: &str = "cut.siglip.google.siglip2-base-patch16-224";
pub(super) const SIGLIP_REQUIRED_FILES: &[&str] = &[
    "config.json",
    "model.safetensors",
    "preprocessor_config.json",
    "special_tokens_map.json",
    "tokenizer.json",
    "tokenizer.model",
    "tokenizer_config.json",
];

/// Identity that binds a stored index to one admitted prepared runtime and its
/// product-selected model group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeRuntimeProvenance {
    pub contract: String,
    pub manifest_sha256: String,
    pub receipt_sha256: String,
    pub model_group: String,
    pub model_id: String,
}

/// The interpreter, bundled indexer and model source selected for an operation.
#[derive(Debug, Clone)]
pub struct Runtime {
    pub python: PathBuf,
    pub script: PathBuf,
    pub model: PathBuf,
    provenance: Option<NativeRuntimeProvenance>,
}

/// The visual-cache authority available to metadata-only consumers. A valid
/// prepared context without the product's SigLIP group must not admit a legacy
/// visual index, but it must not block unrelated media intelligence evidence.
#[derive(Debug, Clone)]
pub(crate) enum VisualCacheRuntime {
    /// No Runner context: retain ordinary installed cache behavior.
    Legacy,
    /// The selected SigLIP group passed full prepared-runtime verification.
    Prepared(Runtime),
    /// A valid Runner context is present, but does not supply SigLIP assets.
    Unavailable,
}

impl Runtime {
    pub fn is_native_context(&self) -> bool {
        self.provenance.is_some()
    }

    /// Add the isolated Python flags, bundled script, and admitted model path.
    /// The stable model ID keeps native index JSON independent of the materialized
    /// local directory name.
    pub fn configure_command(&self, command: &mut Command) {
        apply_python_command_policy(command, self.provenance.is_some());
        command.arg(&self.script).arg("--model").arg(&self.model);
        if let Some(provenance) = &self.provenance {
            command.arg("--model-id").arg(&provenance.model_id);
        }
    }

    pub fn accepts_index(&self, index: &EmbeddingIndex) -> bool {
        self.provenance.as_ref().is_none_or(|provenance| {
            index.model == provenance.model_id && index.native_runtime.as_ref() == Some(provenance)
        })
    }

    /// Bind the Python-produced index to the current prepared runtime before
    /// accepting it as a future cache hit.
    pub fn stamp_index(&self, index: &mut EmbeddingIndex) -> Result<(), String> {
        let Some(provenance) = &self.provenance else {
            return Ok(());
        };
        if index.model != provenance.model_id {
            return Err("SigLIP index model does not match the prepared runtime".into());
        }
        index.native_runtime = Some(provenance.clone());
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn prepared_for_test(provenance: NativeRuntimeProvenance) -> Self {
        Self {
            python: PathBuf::from("/prepared/python"),
            script: PathBuf::from("/bundle/siglip_index.py"),
            model: PathBuf::from("/prepared/models/siglip"),
            provenance: Some(provenance),
        }
    }
}

/// The ordinary installed model id/path, retaining its established override.
fn legacy_model_id() -> String {
    std::env::var("SHELLX_CUT_VISSEARCH_MODEL")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| SIGLIP_MODEL_ID.to_string())
}

/// Resolve a complete runnable SigLIP consumer. A supplied invalid context or
/// incomplete group is an error, never an ordinary Python/model fallback.
pub fn runtime() -> Result<Option<Runtime>, CutError> {
    let sidecar = sidecar_runtime()?;
    let script = siglip_script(&sidecar.script).ok_or_else(|| {
        CutError::new(
            error_codes::SIDECAR,
            "visual-search indexer script is unavailable",
            "repair the bundled perception payload before using SigLIP visual search",
        )
    })?;
    if sidecar.native_context.is_none() && (!sidecar.python.is_file() || !script.is_file()) {
        return Ok(None);
    }
    // Do not consult the ordinary model override when a supplied context is
    // authoritative, even though the native branch below would ignore it.
    let legacy_model = sidecar
        .native_context
        .is_none()
        .then(legacy_model_id)
        .unwrap_or_default();
    runtime_from_sidecar(sidecar, script, legacy_model).map(Some)
}

/// Resolve the selected native group before a vector-only search reads its
/// cache. With no supplied context this intentionally performs no legacy setup.
pub fn native_runtime() -> Result<Option<Runtime>, CutError> {
    if native_runtime_context()?.is_none() {
        return Ok(None);
    }
    runtime()
}

/// Resolve visual-cache authority without requiring SigLIP from a valid
/// prepared runtime that deliberately serves another consumer such as STT or
/// translation. If a SigLIP group is declared, it remains fully verified
/// before any cache is admitted.
pub(crate) fn visual_cache_runtime() -> Result<VisualCacheRuntime, CutError> {
    let Some(context) = native_runtime_context()? else {
        return Ok(VisualCacheRuntime::Legacy);
    };
    if !has_prepared_siglip_group(&context) {
        context
            .verify_interpreter()
            .and_then(|()| context.verify_imports())
            .map_err(native_runtime_error)?;
        return Ok(VisualCacheRuntime::Unavailable);
    }
    native_runtime()?
        .map(VisualCacheRuntime::Prepared)
        .ok_or_else(|| native_model_error("prepared context disappeared during visual selection"))
}

pub(super) fn runtime_from_sidecar(
    sidecar: SidecarRuntime,
    script: PathBuf,
    legacy_model: String,
) -> Result<Runtime, CutError> {
    match sidecar.native_context {
        Some(context) => {
            native_regular_file(&sidecar.python, "admitted Python")?;
            native_regular_file(&script, "bundled SigLIP indexer")?;
            let model = prepared_siglip_directory(&context)?;
            Ok(Runtime {
                python: sidecar.python,
                script,
                model,
                provenance: Some(NativeRuntimeProvenance {
                    contract: context.schema,
                    manifest_sha256: context.manifest_sha256,
                    receipt_sha256: context.receipt_sha256,
                    model_group: SIGLIP_MODEL_GROUP.to_string(),
                    model_id: SIGLIP_MODEL_ID.to_string(),
                }),
            })
        }
        None => Ok(Runtime {
            python: sidecar.python,
            script,
            model: PathBuf::from(legacy_model),
            provenance: None,
        }),
    }
}

/// Native bundles name regular payload files. Do not follow a link from the
/// application-owned perception directory to a host-controlled indexer.
pub(super) fn native_regular_file(path: &Path, resource: &str) -> Result<(), CutError> {
    if std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_file())
        .unwrap_or(false)
    {
        return Ok(());
    }
    Err(CutError::new(
        error_codes::SIDECAR,
        format!("prepared native runtime requires regular {resource}"),
        format!("the installed application payload is missing {resource}"),
    ))
}

fn siglip_script(instruments: &Path) -> Option<PathBuf> {
    instruments
        .parent()
        .map(|directory| directory.join("siglip_index.py"))
}

pub(super) fn has_prepared_siglip_group(context: &RuntimeContext) -> bool {
    let prefix = format!("{SIGLIP_MODEL_GROUP}/");
    context
        .models
        .iter()
        .any(|model| model.id.starts_with(&prefix))
}

fn prepared_siglip_directory(context: &RuntimeContext) -> Result<PathBuf, CutError> {
    let prefix = format!("{SIGLIP_MODEL_GROUP}/");
    let mut declared = BTreeSet::new();
    let mut directory = None;
    for model in context
        .models
        .iter()
        .filter(|model| model.id.starts_with(&prefix))
    {
        let relative = &model.id[prefix.len()..];
        if !clean_flat_name(relative) || !declared.insert(relative.to_string()) {
            return Err(native_model_error(
                "model group contains an invalid file identity",
            ));
        }
        let Some(parent) = model.path.parent() else {
            return Err(native_model_error(
                "model group path has no parent directory",
            ));
        };
        if model.path.file_name().and_then(|name| name.to_str()) != Some(relative) {
            return Err(native_model_error(
                "model group path does not match its identity",
            ));
        }
        if directory.as_ref().is_some_and(|current| current != parent) {
            return Err(native_model_error(
                "model group spans more than one directory",
            ));
        }
        context.verify_model(model).map_err(native_model_error)?;
        directory = Some(parent.to_path_buf());
    }
    if !SIGLIP_REQUIRED_FILES
        .iter()
        .all(|file| declared.contains(*file))
    {
        return Err(native_model_error("model group is incomplete"));
    }
    let Some(directory) = directory else {
        return Err(native_model_error("model group is missing"));
    };
    let actual = std::fs::read_dir(&directory)
        .map_err(|error| native_model_error(format!("read model directory: {error}")))?
        .map(|entry| {
            let entry =
                entry.map_err(|error| native_model_error(format!("read model entry: {error}")))?;
            let file_type = entry
                .file_type()
                .map_err(|error| native_model_error(format!("inspect model entry: {error}")))?;
            if !file_type.is_file() || file_type.is_symlink() {
                return Err(native_model_error(
                    "model directory contains a non-regular entry",
                ));
            }
            entry
                .file_name()
                .into_string()
                .map_err(|_| native_model_error("model directory contains a non-UTF-8 entry"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if actual != declared {
        return Err(native_model_error("model directory has undeclared entries"));
    }
    Ok(directory)
}

fn clean_flat_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
        })
}

fn native_model_error(reason: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::SIDECAR,
        format!("prepared SigLIP model rejected: {}", reason.into()),
        "repair RELEASE_RUNNER_NATIVE_RUNTIME_CONTEXT or its SigLIP model group before visual search",
    )
}

fn native_runtime_error(reason: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::SIDECAR,
        format!("prepared SigLIP runtime rejected: {}", reason.into()),
        "repair RELEASE_RUNNER_NATIVE_RUNTIME_CONTEXT before media intelligence status or visual search",
    )
}

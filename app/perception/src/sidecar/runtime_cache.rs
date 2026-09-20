//! Prepared-runtime cache identity and effective STT selection.
//!
//! The selected model and language must be identical for the cache receipt,
//! Doctor's prepared-runtime admission, and the Python child. Persisted
//! preferences override inherited values per field; an unset field retains the
//! inherited value that the child would otherwise receive.

use super::{read_stt_setting, InstrumentSet, SidecarRuntime};
use crate::types::{PerceptionReport, RuntimeContextProvenance, SttRuntimeBinding};
use cut_core::{error_codes, CutError};

/// STT settings as seen by the Python child after Cut's persisted preferences
/// take precedence over inherited process values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectiveSttSelection {
    pub model: Option<String>,
    pub language: Option<String>,
}

/// Resolve the model and language a Python child will receive.
pub fn effective_stt_selection() -> EffectiveSttSelection {
    EffectiveSttSelection::from_sources(
        read_stt_setting(),
        stt_environment_value("SHELLX_CUT_STT_MODEL"),
        stt_environment_value("SHELLX_CUT_STT_LANG"),
    )
}

impl EffectiveSttSelection {
    pub(super) fn from_sources(
        (model, language): (Option<String>, Option<String>),
        inherited_model: Option<String>,
        inherited_language: Option<String>,
    ) -> Self {
        Self {
            model: model.or(inherited_model),
            language: language.or(inherited_language),
        }
    }
}

fn stt_environment_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// A prepared cache entry must name the same Runner receipt and manifest. Words
/// also bind the model and language supplied to the native Python invocation. A
/// normal run intentionally retains its established cache behavior.
pub(super) fn cache_matches_runtime_context(
    cached: &PerceptionReport,
    expected: Option<&RuntimeContextProvenance>,
) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    let Some(actual) = cached.runtime_context.as_ref() else {
        return false;
    };
    actual.contract == expected.contract
        && actual.manifest_sha256 == expected.manifest_sha256
        && actual.receipt_sha256 == expected.receipt_sha256
        && expected
            .stt
            .as_ref()
            .is_none_or(|stt| actual.stt.as_ref() == Some(stt))
}

pub(super) fn runtime_context_provenance(
    runtime: &SidecarRuntime,
    set: InstrumentSet,
    stt_selection: &EffectiveSttSelection,
) -> Result<Option<RuntimeContextProvenance>, CutError> {
    let Some(context) = runtime.native_context.as_ref() else {
        return Ok(None);
    };
    let stt = if set.names().contains(&"words") {
        let selected = crate::prepared_stt_model(
            context,
            stt_selection.model.as_deref().unwrap_or_default(),
            stt_selection.model.is_none(),
            stt_selection.language.as_deref(),
        )
        .map_err(prepared_runtime_error)?;
        Some(SttRuntimeBinding {
            model: selected.model,
            language: stt_selection.language.clone(),
        })
    } else {
        None
    };
    Ok(Some(RuntimeContextProvenance {
        contract: context.schema.clone(),
        manifest_sha256: context.manifest_sha256.clone(),
        receipt_sha256: context.receipt_sha256.clone(),
        stt,
    }))
}

fn prepared_runtime_error(reason: String) -> CutError {
    CutError::new(
        error_codes::SIDECAR,
        format!("prepared native runtime rejected: {reason}"),
        "repair or remove RELEASE_RUNNER_NATIVE_RUNTIME_CONTEXT before using Python tools",
    )
}

//! Product-owned structural admission for the prepared Parakeet STT groups.
//!
//! Runner pins the runtime inventory. This module deliberately does not hash or
//! load a model: it answers whether the selected group has the exact declared
//! local shape needed before Cut starts Python. The Python sidecar re-hashes the
//! selected bytes immediately before giving that directory to onnx-asr.

use cut_native_runtime_context::RuntimeContext;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const DEFAULT_STT_MODEL: &str = "nemo-parakeet-tdt-0.6b-v3";
const PARAKEET_MODELS: &[&str] = &["nemo-parakeet-tdt-0.6b-v2", "nemo-parakeet-tdt-0.6b-v3"];
const REQUIRED_FILES: &[&str] = &[
    "config.json",
    "encoder-model.onnx",
    "decoder_joint-model.onnx",
    "vocab.txt",
];
// Must match `instruments.py::PARAKEET_WEAK_LANGS`: an implicit Parakeet
// selection on one of these routes is served by Canary/MMS_FA in a normal
// install, but that product model has no prepared-runtime consumer yet.
const PARAKEET_WEAK_LANGS: &[&str] = &[
    "lv", "lt", "et", "mt", "sl", "hr", "sk", "fi", "bg", "da", "el", "hu", "ro", "sv",
];

/// A selected prepared STT group has the declared on-disk shape.
///
/// This is structural admission only. It says neither that the model bytes were
/// re-hashed nor that onnx-asr loaded it or performed inference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedSttModel {
    pub model: String,
}

/// Admit the STT selection that `instruments.py::instrument_words` will use
/// under a prepared context.
///
/// `model_is_default` preserves the important difference between an absent
/// persisted selection (which routes weak languages to Canary/MMS_FA) and an
/// explicit choice of the same v3 default. This function performs only cheap
/// directory and inventory checks; selected bytes remain the Python consumer's
/// responsibility immediately before use.
pub fn prepared_stt_model(
    context: &RuntimeContext,
    model: &str,
    model_is_default: bool,
    language: Option<&str>,
) -> Result<PreparedSttModel, String> {
    let selected = (!model_is_default)
        .then(|| model.trim())
        .filter(|model| !model.is_empty());
    let weak_language = language
        .map(str::trim)
        .filter(|language| !language.is_empty())
        .map(|language| language.to_ascii_lowercase())
        .is_some_and(|language| PARAKEET_WEAK_LANGS.contains(&language.as_str()));
    if selected.is_none() && weak_language {
        return Err(
            "the preferred Canary/MMS_FA language route has no prepared local consumer".into(),
        );
    }

    let model = selected.unwrap_or(DEFAULT_STT_MODEL);
    if !PARAKEET_MODELS.contains(&model) {
        return Err(format!("STT model has no prepared local consumer: {model}"));
    }

    let prefix = format!("cut.stt.{model}/");
    let mut declared = BTreeMap::new();
    for asset in &context.models {
        let Some(filename) = asset.id.strip_prefix(&prefix) else {
            continue;
        };
        if !valid_filename(filename) {
            return Err("prepared STT asset filename is invalid".into());
        }
        if asset.path.file_name().and_then(|name| name.to_str()) != Some(filename) {
            return Err("prepared STT asset path does not match its declared filename".into());
        }
        let metadata = std::fs::symlink_metadata(&asset.path)
            .map_err(|error| format!("cannot inspect prepared STT asset: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("prepared STT asset is not a regular file".into());
        }
        if declared.insert(filename, asset.path.as_path()).is_some() {
            return Err("prepared STT asset filenames must be unique".into());
        }
    }
    if !REQUIRED_FILES
        .iter()
        .all(|filename| declared.contains_key(filename))
    {
        return Err(format!("prepared STT model is incomplete: {model}"));
    }

    let parents: BTreeSet<_> = declared.values().filter_map(|path| path.parent()).collect();
    if parents.len() != 1 {
        return Err("prepared STT model must occupy one exact directory".into());
    }
    let directory = parents
        .into_iter()
        .next()
        .expect("one parent was checked above");
    let actual = std::fs::read_dir(directory)
        .map_err(|error| format!("cannot inspect prepared STT directory: {error}"))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<BTreeSet<PathBuf>, _>>()
        .map_err(|error| format!("cannot inspect prepared STT directory: {error}"))?;
    let expected = declared
        .values()
        .map(|path| (*path).to_path_buf())
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err("prepared STT model directory has undeclared entries".into());
    }

    Ok(PreparedSttModel {
        model: model.to_string(),
    })
}

fn valid_filename(filename: &str) -> bool {
    filename.len() <= 256
        && filename
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && filename
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cut_native_runtime_context::{Interpreter, Model, CONTEXT_CONTRACT};
    use tempfile::TempDir;

    const V3: &str = "nemo-parakeet-tdt-0.6b-v3";

    fn context() -> (TempDir, RuntimeContext) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("runtime");
        let model_dir = root.join("models").join(V3);
        std::fs::create_dir_all(&model_dir).unwrap();
        let models = [
            "config.json",
            "encoder-model.onnx",
            "decoder_joint-model.onnx",
            "vocab.txt",
            "encoder-model.onnx.data",
        ]
        .into_iter()
        .map(|filename| {
            let path = model_dir.join(filename);
            std::fs::write(&path, filename).unwrap();
            Model {
                id: format!("cut.stt.{V3}/{filename}"),
                path,
                sha256: "a".repeat(64),
                provenance_sha256: "b".repeat(64),
            }
        })
        .collect();
        let interpreter = root.join("python");
        std::fs::write(&interpreter, b"python").unwrap();
        (
            temp,
            RuntimeContext {
                schema: CONTEXT_CONTRACT.into(),
                root,
                manifest_sha256: "c".repeat(64),
                receipt_sha256: "d".repeat(64),
                interpreter: Interpreter {
                    path: interpreter,
                    sha256: "e".repeat(64),
                    version: "3.12.13".into(),
                },
                imports: Vec::new(),
                models,
                files: 6,
                total_bytes: 1,
            },
        )
    }

    #[test]
    fn default_and_explicit_selected_parakeet_are_structurally_admitted() {
        let (_temp, context) = context();
        assert_eq!(
            prepared_stt_model(&context, V3, true, None).unwrap().model,
            V3
        );
        assert_eq!(
            prepared_stt_model(&context, V3, false, Some("lv"))
                .unwrap()
                .model,
            V3,
            "an explicit v3 choice must not take the default weak-language route"
        );
    }

    #[test]
    fn weak_default_and_unsupported_selected_models_are_unavailable() {
        let (_temp, context) = context();
        assert!(prepared_stt_model(&context, V3, true, Some("LV"))
            .unwrap_err()
            .contains("Canary/MMS_FA"));
        assert!(
            prepared_stt_model(&context, "nemo-canary-1b-v2", false, None)
                .unwrap_err()
                .contains("no prepared local consumer")
        );
    }

    #[test]
    fn missing_or_undeclared_group_content_is_not_admitted() {
        let (_temp, mut incomplete_context) = context();
        incomplete_context
            .models
            .retain(|asset| !asset.id.ends_with("/vocab.txt"));
        assert!(prepared_stt_model(&incomplete_context, V3, true, None)
            .unwrap_err()
            .contains("incomplete"));

        let (_temp, context) = context();
        let extra = context.models[0].path.parent().unwrap().join("stray.bin");
        std::fs::write(extra, b"stray").unwrap();
        assert!(prepared_stt_model(&context, V3, true, None)
            .unwrap_err()
            .contains("undeclared"));
    }
}

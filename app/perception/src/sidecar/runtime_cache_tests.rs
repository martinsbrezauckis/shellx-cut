use super::runtime_cache::{cache_matches_runtime_context, runtime_context_provenance};
use super::{EffectiveSttSelection, InstrumentSet, SidecarRuntime};
use crate::types::{PerceptionReport, RuntimeContextProvenance, PERCEPTION_SCHEMA};
use cut_native_runtime_context::{Import, Interpreter, Model, RuntimeContext, CONTEXT_CONTRACT};

fn prepared_runtime() -> (tempfile::TempDir, SidecarRuntime) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("runtime");
    let models_dir = root.join("models");
    std::fs::create_dir_all(&models_dir).unwrap();
    let python = root.join("python");
    let import = root.join("onnx_asr.py");
    let script = root.join("instruments.py");
    for path in [&python, &import, &script] {
        std::fs::write(path, b"fixture").unwrap();
    }
    let models = [
        "config.json",
        "encoder-model.onnx",
        "decoder_joint-model.onnx",
        "vocab.txt",
    ]
    .into_iter()
    .map(|filename| {
        let path = models_dir.join(filename);
        std::fs::write(&path, b"fixture").unwrap();
        Model {
            id: format!("cut.stt.nemo-parakeet-tdt-0.6b-v3/{filename}"),
            path,
            sha256: "a".repeat(64),
            provenance_sha256: "b".repeat(64),
        }
    })
    .collect();
    (
        temp,
        SidecarRuntime {
            python: python.clone(),
            script,
            native_context: Some(RuntimeContext {
                schema: CONTEXT_CONTRACT.into(),
                root,
                manifest_sha256: "c".repeat(64),
                receipt_sha256: "d".repeat(64),
                interpreter: Interpreter {
                    path: python,
                    sha256: "e".repeat(64),
                    version: "3.12.13".into(),
                },
                imports: vec![Import {
                    module: "onnx_asr".into(),
                    path: import,
                    sha256: "f".repeat(64),
                }],
                models,
                files: 7,
                total_bytes: 7,
            }),
        },
    )
}

fn report(instruments: &[&str]) -> PerceptionReport {
    PerceptionReport {
        schema: PERCEPTION_SCHEMA.into(),
        asset_hash: "sha256:cafe".into(),
        source_path: "/gone.mp4".into(),
        instruments_run: instruments
            .iter()
            .map(|instrument| instrument.to_string())
            .collect(),
        words: None,
        silences: vec![],
        scenes: vec![],
        beats: None,
        loudness: None,
        black_spans: vec![],
        frozen_spans: vec![],
        content_bbox: None,
        subject_track: None,
        speaker_turns: vec![],
        diarization: None,
        runtime_context: None,
    }
}

#[test]
fn effective_stt_selection_prioritizes_persisted_values_per_field() {
    let inherited = EffectiveSttSelection::from_sources(
        (None, None),
        Some("nemo-parakeet-tdt-0.6b-v2".into()),
        Some("lv".into()),
    );
    assert_eq!(
        inherited.model.as_deref(),
        Some("nemo-parakeet-tdt-0.6b-v2")
    );
    assert_eq!(inherited.language.as_deref(), Some("lv"));

    let persisted = EffectiveSttSelection::from_sources(
        (Some("nemo-parakeet-tdt-0.6b-v3".into()), None),
        Some("nemo-parakeet-tdt-0.6b-v2".into()),
        Some("lv".into()),
    );
    assert_eq!(
        persisted.model.as_deref(),
        Some("nemo-parakeet-tdt-0.6b-v3")
    );
    assert_eq!(persisted.language.as_deref(), Some("lv"));
}

#[test]
fn prepared_cache_requires_exact_context_model_and_language_binding() {
    let (_temp, runtime) = prepared_runtime();
    let selection = EffectiveSttSelection {
        model: Some("nemo-parakeet-tdt-0.6b-v3".into()),
        language: Some("fr".into()),
    };
    let expected = runtime_context_provenance(&runtime, InstrumentSet::WordsOnly, &selection)
        .unwrap()
        .unwrap();
    let mut cached = report(&["words"]);
    cached.runtime_context = Some(expected.clone());
    assert!(cache_matches_runtime_context(&cached, Some(&expected)));

    let mut wrong_manifest = expected.clone();
    wrong_manifest.manifest_sha256 = "e".repeat(64);
    cached.runtime_context = Some(wrong_manifest);
    assert!(!cache_matches_runtime_context(&cached, Some(&expected)));

    let mut wrong_model = expected.clone();
    wrong_model.stt.as_mut().unwrap().model = "nemo-parakeet-tdt-0.6b-v2".into();
    cached.runtime_context = Some(wrong_model);
    assert!(!cache_matches_runtime_context(&cached, Some(&expected)));

    cached.runtime_context = Some(RuntimeContextProvenance {
        stt: None,
        ..expected.clone()
    });
    assert!(!cache_matches_runtime_context(&cached, Some(&expected)));

    let mut wrong_language = expected.clone();
    wrong_language.stt.as_mut().unwrap().language = Some("de".into());
    cached.runtime_context = Some(wrong_language);
    assert!(!cache_matches_runtime_context(&cached, Some(&expected)));
}

#[test]
fn prepared_words_reject_weak_default_language_and_incomplete_model_group() {
    let (_temp, runtime) = prepared_runtime();
    let error = runtime_context_provenance(
        &runtime,
        InstrumentSet::WordsOnly,
        &EffectiveSttSelection {
            model: None,
            language: Some("lv".into()),
        },
    )
    .expect_err("weak default language must not claim a prepared Parakeet group");
    assert!(error.message.contains("Canary/MMS_FA"));

    let (_temp, mut runtime) = prepared_runtime();
    runtime
        .native_context
        .as_mut()
        .unwrap()
        .models
        .retain(|asset| !asset.id.ends_with("/vocab.txt"));
    let error = runtime_context_provenance(
        &runtime,
        InstrumentSet::WordsOnly,
        &EffectiveSttSelection::default(),
    )
    .expect_err("prepared words must require the selected model group");
    assert!(error.message.contains("incomplete"));
}

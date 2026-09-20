use super::runtime::{
    has_prepared_siglip_group, runtime_from_sidecar, SIGLIP_MODEL_GROUP, SIGLIP_MODEL_ID,
    SIGLIP_REQUIRED_FILES,
};
use super::{EmbeddingIndex, FrameEmbedding};
use cut_native_runtime_context::{Interpreter, Model, RuntimeContext};
use cut_perception::SidecarRuntime;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

fn hash(path: &Path) -> String {
    let mut digest = Sha256::new();
    digest.update(std::fs::read(path).unwrap());
    format!("{:x}", digest.finalize())
}

fn fixture_context(
    temp: &tempfile::TempDir,
    manifest: &str,
    complete: bool,
) -> (RuntimeContext, PathBuf, PathBuf) {
    let root = temp.path().join("runtime");
    let model_dir = root.join("models").join("siglip2-base-patch16-224");
    let scripts = temp.path().join("payload");
    std::fs::create_dir_all(&model_dir).unwrap();
    std::fs::create_dir_all(&scripts).unwrap();
    let python = root.join("python");
    std::fs::write(&python, b"python").unwrap();
    let instruments = scripts.join("instruments.py");
    std::fs::write(&instruments, b"instruments").unwrap();
    std::fs::write(scripts.join("siglip_index.py"), b"siglip").unwrap();
    let models = SIGLIP_REQUIRED_FILES
        .iter()
        .copied()
        .filter(|name| complete || *name != "config.json")
        .map(|name| {
            let path = model_dir.join(name);
            std::fs::write(&path, name.as_bytes()).unwrap();
            Model {
                id: format!("{SIGLIP_MODEL_GROUP}/{name}"),
                path: path.clone(),
                sha256: hash(&path),
                provenance_sha256: "a".repeat(64),
            }
        })
        .collect();
    (
        RuntimeContext {
            schema: "release-runner.native-runtime-context/v1".into(),
            root,
            manifest_sha256: manifest.into(),
            receipt_sha256: "b".repeat(64),
            interpreter: Interpreter {
                path: python.clone(),
                sha256: hash(&python),
                version: "3.12.13".into(),
            },
            imports: vec![],
            models,
            files: 1,
            total_bytes: 1,
        },
        python,
        instruments,
    )
}

fn sidecar(python: PathBuf, instruments: PathBuf, context: RuntimeContext) -> SidecarRuntime {
    SidecarRuntime {
        python,
        script: instruments,
        native_context: Some(context),
    }
}

fn index() -> EmbeddingIndex {
    EmbeddingIndex {
        schema: "shellx-cut/vissearch/1".into(),
        model: SIGLIP_MODEL_ID.into(),
        dim: 2,
        asset: "a1".into(),
        frames: vec![FrameEmbedding {
            ms: 0,
            v: vec![1.0, 0.0],
        }],
        native_runtime: None,
    }
}

#[test]
fn prepared_runtime_uses_admitted_group_and_isolated_python_command() {
    let temp = tempfile::tempdir().unwrap();
    let (context, python, instruments) = fixture_context(&temp, &"1".repeat(64), true);
    let runtime = runtime_from_sidecar(
        sidecar(python, instruments, context),
        temp.path().join("payload").join("siglip_index.py"),
        "/host/model-override".into(),
    )
    .unwrap();
    assert_eq!(
        runtime.model,
        temp.path().join("runtime/models/siglip2-base-patch16-224")
    );
    assert!(runtime.is_native_context());
    let mut command = Command::new(&runtime.python);
    runtime.configure_command(&mut command);
    assert_eq!(
        command
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        vec![
            "-E".to_string(),
            "-s".to_string(),
            "-B".to_string(),
            temp.path()
                .join("payload/siglip_index.py")
                .display()
                .to_string(),
            "--model".to_string(),
            temp.path()
                .join("runtime/models/siglip2-base-patch16-224")
                .display()
                .to_string(),
            "--model-id".to_string(),
            SIGLIP_MODEL_ID.to_string(),
        ],
    );
}

#[test]
fn incomplete_prepared_group_fails_without_legacy_model_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let (context, python, instruments) = fixture_context(&temp, &"1".repeat(64), false);
    assert!(runtime_from_sidecar(
        sidecar(python, instruments, context),
        temp.path().join("payload").join("siglip_index.py"),
        "google/legacy-model".into(),
    )
    .is_err());
}

#[test]
fn siglip_group_detection_keeps_valid_partial_runtimes_distinct() {
    let temp = tempfile::tempdir().unwrap();
    let (context, _python, _instruments) = fixture_context(&temp, &"1".repeat(64), true);
    assert!(has_prepared_siglip_group(&context));

    let mut partial = context;
    partial.models.clear();
    assert!(
        !has_prepared_siglip_group(&partial),
        "an STT-only or translation-only prepared context does not select SigLIP"
    );
}

#[test]
fn prepared_index_cache_requires_exact_context_identity() {
    let temp = tempfile::tempdir().unwrap();
    let (context, python, instruments) = fixture_context(&temp, &"1".repeat(64), true);
    let runtime = runtime_from_sidecar(
        sidecar(python, instruments, context),
        temp.path().join("payload").join("siglip_index.py"),
        SIGLIP_MODEL_ID.into(),
    )
    .unwrap();
    let mut cached = index();
    assert!(!runtime.accepts_index(&cached));
    runtime.stamp_index(&mut cached).unwrap();
    assert!(runtime.accepts_index(&cached));

    let next_temp = tempfile::tempdir().unwrap();
    let (next_context, python, instruments) = fixture_context(&next_temp, &"2".repeat(64), true);
    let next_runtime = runtime_from_sidecar(
        sidecar(python, instruments, next_context),
        next_temp.path().join("payload").join("siglip_index.py"),
        SIGLIP_MODEL_ID.into(),
    )
    .unwrap();
    assert!(!next_runtime.accepts_index(&cached));
}

#[test]
fn installed_runtime_keeps_existing_model_override_and_cache_behavior() {
    let runtime = runtime_from_sidecar(
        SidecarRuntime {
            python: PathBuf::from("/configured/python"),
            script: PathBuf::from("/configured/instruments.py"),
            native_context: None,
        },
        PathBuf::from("/configured/siglip_index.py"),
        "google/custom-siglip".into(),
    )
    .unwrap();
    assert!(!runtime.is_native_context());
    assert!(runtime.accepts_index(&index()));
    let mut command = Command::new(&runtime.python);
    runtime.configure_command(&mut command);
    assert_eq!(
        command
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        vec![
            "-B".to_string(),
            "/configured/siglip_index.py".to_string(),
            "--model".to_string(),
            "google/custom-siglip".to_string(),
        ],
    );
}

#[cfg(unix)]
#[test]
fn prepared_runtime_refuses_symlinked_bundled_indexer() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let (context, python, instruments) = fixture_context(&temp, &"1".repeat(64), true);
    let target = temp.path().join("host-indexer.py");
    let link = temp.path().join("payload").join("siglip_index.py");
    std::fs::write(&target, b"fixture").unwrap();
    std::fs::remove_file(&link).unwrap();
    symlink(&target, &link).unwrap();

    let error = runtime_from_sidecar(
        sidecar(python, instruments, context),
        link,
        SIGLIP_MODEL_ID.into(),
    )
    .expect_err("prepared runtime must not follow a bundled indexer link");
    assert_eq!(error.code, cut_core::error_codes::SIDECAR);
    assert!(error.message.contains("regular bundled SigLIP indexer"));
}

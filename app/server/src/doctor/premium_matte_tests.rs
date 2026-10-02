use super::*;
use std::path::PathBuf;

use cut_native_runtime_context::RuntimeContext;

fn native_context_without_matte_models() -> (tempfile::TempDir, RuntimeContext, PathBuf) {
    use cut_native_runtime_context::{Import, Interpreter, CONTEXT_CONTRACT};
    use sha2::{Digest, Sha256};

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("runtime");
    std::fs::create_dir(&root).unwrap();
    let python = root.join("python");
    let import = root.join("onnxruntime.py");
    std::fs::write(&python, b"python").unwrap();
    std::fs::write(&import, b"onnxruntime").unwrap();
    let hash = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
    let context = RuntimeContext {
        schema: CONTEXT_CONTRACT.into(),
        root,
        manifest_sha256: "a".repeat(64),
        receipt_sha256: "b".repeat(64),
        interpreter: Interpreter {
            path: python,
            sha256: hash(b"python"),
            version: "3.12.13".into(),
        },
        imports: vec![Import {
            module: "onnxruntime".into(),
            path: import,
            sha256: hash(b"onnxruntime"),
        }],
        models: vec![],
        files: 2,
        total_bytes: 17,
    };
    let instruments = temp.path().join("payload").join("instruments.py");
    std::fs::create_dir_all(instruments.parent().unwrap()).unwrap();
    std::fs::write(&instruments, b"instruments").unwrap();
    (temp, context, instruments)
}

fn prepared_premium_runtime() -> crate::matte::premium_runtime::PreparedMatanyoneRuntime {
    use crate::matte::premium_runtime::PreparedMatanyoneBinding;

    crate::matte::premium_runtime::PreparedMatanyoneRuntime {
        python: PathBuf::from("/sealed/runtime/bin/python"),
        rvm_script: PathBuf::from("/installed/perception/matte_runner.py"),
        matanyone_script: PathBuf::from("/installed/perception/matanyone_runner.py"),
        sam2_script: PathBuf::from("/installed/perception/sam2_runner.py"),
        rvm_model: PathBuf::from("/sealed/runtime/models/rvm_mobilenetv3_fp32.onnx"),
        matanyone_model: PathBuf::from("/sealed/runtime/models/matanyone2.pth"),
        sam2_model: PathBuf::from("/sealed/runtime/models/sam2_hiera_base_plus.pt"),
        binding: PreparedMatanyoneBinding {
            contract: "release-runner.native-runtime-context/v1".into(),
            manifest_sha256: "a".repeat(64),
            receipt_sha256: "b".repeat(64),
            rvm_model_id: "cut.matte.rvm/rvm_mobilenetv3_fp32.onnx".into(),
            rvm_model_sha256: "c".repeat(64),
            matanyone_model_id: "cut.matte.matanyone/matanyone2.pth".into(),
            matanyone_model_sha256: "d".repeat(64),
            sam2_model_id: "cut.matte.sam2/sam2_hiera_base_plus.pt".into(),
            sam2_model_sha256: "e".repeat(64),
        },
    }
}

#[test]
fn prepared_premium_matte_card_requires_a_sealed_group_and_confirmed_cuda() {
    let runtime = prepared_premium_runtime();
    let ready = prepared_matte_premium_card_with_cuda(&runtime, Some(true));
    assert_eq!(ready.id, "matte_premium");
    assert_eq!(ready.status, CardStatus::Ok);
    assert_eq!(ready.details["installed"], json!(true));
    assert_eq!(ready.details["checkpoint_present"], json!(true));
    assert_eq!(ready.details["cuda_available"], json!(true));
    assert_eq!(
        ready.details["prepared_runtime"]["matanyone_model_id"],
        json!("cut.matte.matanyone/matanyone2.pth")
    );
    assert_eq!(
        ready.details["prepared_runtime"]["sam2_model_id"],
        json!("cut.matte.sam2/sam2_hiera_base_plus.pt")
    );

    assert_eq!(
        prepared_matte_premium_card_with_cuda(&runtime, Some(false)).status,
        CardStatus::Degraded,
        "a sealed model group on a CPU-only runtime must not be advertised as premium-ready"
    );
    assert_eq!(
        prepared_matte_premium_card_with_cuda(&runtime, None).status,
        CardStatus::Unknown,
        "a timed-out CUDA probe must not be advertised as premium-ready"
    );
}

#[test]
fn prepared_premium_matte_failure_refuses_legacy_runtime_fallback() {
    let card = prepared_matte_premium_card_failure("missing MatAnyone2 asset");
    assert_eq!(card.status, CardStatus::Degraded);
    assert_eq!(card.details["installed"], json!(false));
    assert_eq!(card.details["checkpoint_present"], json!(false));
    assert_eq!(card.details["prepared_runtime"]["rejected"], json!(true));
    assert!(card
        .hint
        .as_deref()
        .is_some_and(|hint| hint.contains("did not fall back")));
}

#[test]
fn prepared_premium_card_rejects_an_incomplete_resolver_group_without_legacy_fallback() {
    let (_temp, context, instruments) = native_context_without_matte_models();
    let resolution = crate::matte::premium_runtime::prepared_matanyone_from_context(
        &context,
        &context.interpreter.path,
        &instruments,
    )
    .map(Box::new)
    .map(crate::matte::native_runtime::PreparedMatteRuntime::Matanyone)
    .map(Some);

    let card = prepared_matte_premium_card_from_resolution(resolution);

    assert_eq!(card.status, CardStatus::Degraded);
    assert_eq!(card.details["installed"], json!(false));
    assert_eq!(card.details["prepared_runtime"]["rejected"], json!(true));
    assert!(card.details["prepared_runtime"]["reason"]
        .as_str()
        .is_some_and(|reason| reason.contains("RVM asset")));
    assert!(card
        .hint
        .as_deref()
        .is_some_and(|hint| hint.contains("did not fall back")));
}

#[test]
fn native_premium_cuda_probe_keeps_the_sealed_python_policy() {
    let command =
        matte_premium_cuda_probe_command(Path::new("/sealed/runtime/python"), true).unwrap();
    assert_eq!(
        command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        [
            "-E",
            "-s",
            "-B",
            "-c",
            "import torch; print('cuda', torch.cuda.is_available())",
        ]
    );
    assert_eq!(
        command
            .get_envs()
            .find(|(name, _)| *name == "PYTHONDONTWRITEBYTECODE")
            .and_then(|(_, value)| value)
            .and_then(|value| value.to_str()),
        Some("1")
    );
}

//! Focused contracts for the sealed prepared premium matte model group.

use super::{
    prepared_matanyone_from_context_with_hashes, PreparedMatanyoneBinding,
    PreparedMatanyoneRuntime, MATANYONE_MODEL_ID, SAM2_MODEL_ID,
};
use crate::matte::native_runtime::{PreparedMatteBinding, RVM_MODEL_ID};
use crate::matte::prepared::prepared_seed_path;
use crate::matte::{cache_matches_prepared_runtime, MatteStats};
use cut_core::MatteSeed;
use cut_native_runtime_context::{Interpreter, Model, RuntimeContext, CONTEXT_CONTRACT};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn premium_binding() -> PreparedMatanyoneBinding {
    PreparedMatanyoneBinding {
        contract: CONTEXT_CONTRACT.into(),
        manifest_sha256: "a".repeat(64),
        receipt_sha256: "b".repeat(64),
        rvm_model_id: RVM_MODEL_ID.into(),
        rvm_model_sha256: "c".repeat(64),
        matanyone_model_id: MATANYONE_MODEL_ID.into(),
        matanyone_model_sha256: "d".repeat(64),
        sam2_model_id: SAM2_MODEL_ID.into(),
        sam2_model_sha256: "e".repeat(64),
    }
}

fn stats(native_runtime: Option<PreparedMatteBinding>) -> MatteStats {
    MatteStats {
        frames: 1,
        fps: 30.0,
        width: 1,
        height: 1,
        downsample_ratio: 1.0,
        coverage_mean: 0.5,
        cov_min: 0.5,
        cov_max: 0.5,
        temporal_flicker: 0.0,
        edge_softness: None,
        model: Some("matanyone2".into()),
        device: Some("cpu".into()),
        cached: false,
        native_runtime,
    }
}

#[test]
fn prepared_premium_cache_binds_each_required_model() {
    let expected = PreparedMatteBinding::Matanyone(premium_binding());
    assert!(cache_matches_prepared_runtime(
        &stats(Some(expected.clone())),
        Some(&expected)
    ));

    let mut changed = premium_binding();
    changed.sam2_model_sha256 = "f".repeat(64);
    assert!(
        !cache_matches_prepared_runtime(
            &stats(Some(PreparedMatteBinding::Matanyone(changed))),
            Some(&expected)
        ),
        "a changed SAM2 checkpoint must bypass the alpha cache"
    );
}

fn premium_fixture() -> (
    tempfile::TempDir,
    RuntimeContext,
    PathBuf,
    PathBuf,
    (String, String, String),
) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("runtime");
    let payload = temp.path().join("payload");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&payload).unwrap();
    let python = root.join("python");
    let rvm = root.join("rvm_mobilenetv3_fp32.onnx");
    let matanyone = root.join("matanyone2.pth");
    let sam2 = root.join("sam2_hiera_base_plus.pt");
    for (path, bytes) in [
        (&python, b"python".as_slice()),
        (&rvm, b"rvm".as_slice()),
        (&matanyone, b"matanyone".as_slice()),
        (&sam2, b"sam2".as_slice()),
    ] {
        std::fs::write(path, bytes).unwrap();
    }
    for name in [
        "instruments.py",
        "matte_runner.py",
        "matanyone_runner.py",
        "sam2_runner.py",
    ] {
        std::fs::write(payload.join(name), b"runner").unwrap();
    }
    let hashes = (hash(b"rvm"), hash(b"matanyone"), hash(b"sam2"));
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
        imports: vec![],
        models: vec![
            Model {
                id: RVM_MODEL_ID.into(),
                path: rvm,
                sha256: hashes.0.clone(),
                provenance_sha256: "c".repeat(64),
            },
            Model {
                id: MATANYONE_MODEL_ID.into(),
                path: matanyone,
                sha256: hashes.1.clone(),
                provenance_sha256: "d".repeat(64),
            },
            Model {
                id: SAM2_MODEL_ID.into(),
                path: sam2,
                sha256: hashes.2.clone(),
                provenance_sha256: "e".repeat(64),
            },
        ],
        files: 7,
        total_bytes: 64,
    };
    (
        temp,
        context,
        python,
        payload.join("instruments.py"),
        hashes,
    )
}

#[test]
fn prepared_premium_runtime_requires_the_complete_model_and_script_group() {
    let (_temp, context, python, instruments, hashes) = premium_fixture();
    let runtime = prepared_matanyone_from_context_with_hashes(
        &context,
        &python,
        &instruments,
        &hashes.0,
        &hashes.1,
        &hashes.2,
    )
    .unwrap();
    assert_eq!(runtime.rvm_model, context.models[0].path);
    assert_eq!(runtime.matanyone_model, context.models[1].path);
    assert_eq!(runtime.sam2_model, context.models[2].path);
    assert_eq!(
        runtime.sam2_script,
        instruments.parent().unwrap().join("sam2_runner.py")
    );

    for (missing_id, label) in [
        (RVM_MODEL_ID, "RVM"),
        (MATANYONE_MODEL_ID, "MatAnyone2"),
        (SAM2_MODEL_ID, "SAM2"),
    ] {
        let (_temp, mut context, python, instruments, hashes) = premium_fixture();
        context.models.retain(|model| model.id != missing_id);
        let error = prepared_matanyone_from_context_with_hashes(
            &context,
            &python,
            &instruments,
            &hashes.0,
            &hashes.1,
            &hashes.2,
        )
        .unwrap_err();
        assert!(
            error.message.contains(&format!("{label} asset")),
            "{missing_id}: {}",
            error.message
        );
    }
}

#[test]
fn prepared_sam2_launch_passes_only_the_context_verified_checkpoint() {
    let runtime = PreparedMatanyoneRuntime {
        python: PathBuf::from("/runtime/python"),
        rvm_script: PathBuf::from("/bundle/matte_runner.py"),
        matanyone_script: PathBuf::from("/bundle/matanyone_runner.py"),
        sam2_script: PathBuf::from("/bundle/sam2_runner.py"),
        rvm_model: PathBuf::from("/runtime/rvm_mobilenetv3_fp32.onnx"),
        matanyone_model: PathBuf::from("/runtime/matanyone2.pth"),
        sam2_model: PathBuf::from("/runtime/sam2_hiera_base_plus.pt"),
        binding: premium_binding(),
    };
    let command = crate::matte::prepared::sam2_seed_command(
        &runtime,
        Path::new("/project/input.mp4"),
        Path::new("/project/seed.png"),
        &MatteSeed {
            at_ms: 77,
            point: Some([3, 4]),
            bbox: None,
        },
    )
    .unwrap();
    let args = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(args
        .windows(2)
        .any(|pair| { pair == ["--checkpoint", "/runtime/sam2_hiera_base_plus.pt"] }));
    assert!(!args
        .iter()
        .any(|arg| arg == "--hf-home" || arg == "--hf-revision"));
    assert_eq!(&args[..4], ["-E", "-s", "-B", "/bundle/sam2_runner.py"]);
}

#[test]
fn prepared_seed_cache_name_is_bounded_and_binds_the_full_identity_group() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = PreparedMatanyoneRuntime {
        python: PathBuf::from("/runtime/python"),
        rvm_script: PathBuf::from("/bundle/matte_runner.py"),
        matanyone_script: PathBuf::from("/bundle/matanyone_runner.py"),
        sam2_script: PathBuf::from("/bundle/sam2_runner.py"),
        rvm_model: PathBuf::from("/runtime/rvm_mobilenetv3_fp32.onnx"),
        matanyone_model: PathBuf::from("/runtime/matanyone2.pth"),
        sam2_model: PathBuf::from("/runtime/sam2_hiera_base_plus.pt"),
        binding: premium_binding(),
    };
    let asset_hash = "a".repeat(64);
    let seed = prepared_seed_path(temp.path(), &asset_hash, "sam2-0123456789abcdef", &runtime);
    let name = seed.file_name().and_then(|name| name.to_str()).unwrap();
    assert!(
        name.len() < 255,
        "cache entry must fit common filename limits"
    );
    std::fs::write(&seed, b"prepared seed").unwrap();
    assert!(seed.is_file(), "bounded filename must be materializable");

    let mut changed_receipt = runtime.clone();
    changed_receipt.binding.receipt_sha256 = "f".repeat(64);
    let receipt_changed = prepared_seed_path(
        temp.path(),
        &asset_hash,
        "sam2-0123456789abcdef",
        &changed_receipt,
    );
    assert_ne!(seed, receipt_changed);

    let mut changed_sam2 = runtime;
    changed_sam2.binding.sam2_model_sha256 = "1".repeat(64);
    let sam2_changed = prepared_seed_path(
        temp.path(),
        &asset_hash,
        "sam2-0123456789abcdef",
        &changed_sam2,
    );
    assert_ne!(seed, sam2_changed);
}

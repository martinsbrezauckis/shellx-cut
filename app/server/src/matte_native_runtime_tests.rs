//! Focused contracts for prepared RVM launch isolation and cache provenance.

use super::{PreparedMatteBinding, PreparedRvmBinding, RVM_MODEL_ID};
use crate::matte::{cache_matches_prepared_runtime, matte_runner_command, MatteStats};
use std::path::Path;

fn binding() -> PreparedRvmBinding {
    PreparedRvmBinding {
        contract: "release-runner.native-runtime-context/v1".into(),
        manifest_sha256: "a".repeat(64),
        receipt_sha256: "b".repeat(64),
        model_id: RVM_MODEL_ID.into(),
        model_sha256: "c".repeat(64),
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
        model: Some("rvm".into()),
        device: Some("cpu".into()),
        cached: false,
        native_runtime,
    }
}

#[test]
fn prepared_rvm_cache_requires_the_exact_runtime_and_model_identity() {
    let expected = PreparedMatteBinding::Rvm(binding());
    assert!(cache_matches_prepared_runtime(
        &stats(Some(expected.clone())),
        Some(&expected)
    ));
    assert!(cache_matches_prepared_runtime(&stats(None), None));
    assert!(
        !cache_matches_prepared_runtime(&stats(None), Some(&expected)),
        "a legacy alpha receipt cannot satisfy a prepared RVM bake"
    );

    let mut stale = binding();
    stale.receipt_sha256 = "d".repeat(64);
    assert!(
        !cache_matches_prepared_runtime(
            &stats(Some(PreparedMatteBinding::Rvm(stale))),
            Some(&expected)
        ),
        "a prior prepared runtime receipt cannot satisfy the current one"
    );

    let mut changed_model = binding();
    changed_model.model_sha256 = "e".repeat(64);
    assert!(!cache_matches_prepared_runtime(
        &stats(Some(PreparedMatteBinding::Rvm(changed_model))),
        Some(&expected)
    ));
}

#[test]
fn prepared_rvm_launches_with_the_resolved_isolated_python_policy() {
    let python = Path::new("/runtime/python");
    let runner = Path::new("/bundle/perception/matte_runner.py");
    let normal = matte_runner_command(python, runner, false);
    assert_eq!(
        normal
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        vec!["-B".to_string(), runner.display().to_string()]
    );

    let prepared = matte_runner_command(python, runner, true);
    assert_eq!(
        prepared
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        vec![
            "-E".to_string(),
            "-s".to_string(),
            "-B".to_string(),
            runner.display().to_string(),
        ]
    );
    assert_eq!(
        prepared
            .get_envs()
            .find(|(name, _)| *name == "PYTHONDONTWRITEBYTECODE")
            .and_then(|(_, value)| value)
            .and_then(|value| value.to_str()),
        Some("1")
    );
    for variable in [
        "MATTE_RUNNER_PY",
        "MATTE_RUNNER_SCRIPT",
        "MATTE_MODEL",
        "MATTE_PROVIDERS",
    ] {
        assert_eq!(
            prepared
                .get_envs()
                .find(|(name, _)| *name == std::ffi::OsStr::new(variable))
                .map(|(_, value)| value),
            Some(None),
            "prepared runtime must remove {variable}",
        );
    }
}

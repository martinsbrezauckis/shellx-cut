//! Tests for prepared native Python and packaged sidecar selection.

use super::*;

// Keep this guard inside NATIVE_RUNTIME_ENV_LOCK so unwinding restores the
// locator before another test can use the environment.
struct NativeRuntimeEnvRestore(Option<std::ffi::OsString>);

impl NativeRuntimeEnvRestore {
    fn capture() -> Self {
        Self(std::env::var_os(cut_native_runtime_context::CONTEXT_ENV))
    }
}

impl Drop for NativeRuntimeEnvRestore {
    fn drop(&mut self) {
        match self.0.take() {
            Some(value) => std::env::set_var(cut_native_runtime_context::CONTEXT_ENV, value),
            None => std::env::remove_var(cut_native_runtime_context::CONTEXT_ENV),
        }
    }
}

#[test]
fn premium_member_is_selected_without_changing_ordinary_python() {
    let _guard = super::super::NATIVE_RUNTIME_ENV_LOCK.lock().unwrap();
    let _restore = NativeRuntimeEnvRestore::capture();
    let temp = tempfile::tempdir().unwrap();
    let context = |name: &str, version: &str| {
        let root = temp.path().join(name);
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let interpreter = root.join("python");
        let import = root.join("module.py");
        std::fs::write(&interpreter, name.as_bytes()).unwrap();
        std::fs::write(&import, b"module").unwrap();
        let hash = |bytes: &[u8]| {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(bytes))
        };
        serde_json::json!({
            "schema": cut_native_runtime_context::CONTEXT_CONTRACT,
            "root": root,
            "manifestSha256": "a".repeat(64),
            "receiptSha256": "b".repeat(64),
            "interpreter": {"path": interpreter, "sha256": hash(name.as_bytes()), "version": version},
            "imports": [{"module": "module", "path": import, "sha256": hash(b"module")}],
            "models": [], "files": 2, "totalBytes": 16
        })
    };
    let ordinary = context("ordinary", "3.12.13");
    let premium = context("premium", "3.11.15");
    let locator = temp.path().join("native-runtime-context.json");
    let write_set = |runtimes: serde_json::Value| {
        std::fs::write(
            &locator,
            serde_json::to_vec(&serde_json::json!({
                "schema": cut_native_runtime_context::SET_CONTEXT_CONTRACT,
                "runtimes": runtimes
            }))
            .unwrap(),
        )
        .unwrap();
        std::env::set_var(cut_native_runtime_context::CONTEXT_ENV, &locator);
    };
    write_set(serde_json::json!([
        {"id": "premium", "context": premium},
        {"id": "python", "context": ordinary}
    ]));
    let base = selected_native_runtime_context(false).unwrap().unwrap();
    let premium = selected_native_runtime_context(true).unwrap().unwrap();
    assert_eq!(base.interpreter.version, "3.12.13");
    assert_eq!(premium.interpreter.version, "3.11.15");
    assert_ne!(base.root, premium.root);
    premium.verify_interpreter().unwrap();
    premium.verify_imports().unwrap();

    write_set(serde_json::json!([{"id": "python", "context": ordinary}]));
    assert_eq!(
        selected_native_runtime_context(true).unwrap().unwrap().root,
        base.root,
        "legacy single-Python sets retain their premium binding"
    );
    write_set(serde_json::json!([
        {"id": "premium", "context": {"schema": "invalid"}},
        {"id": "python", "context": ordinary}
    ]));
    assert!(selected_native_runtime_context(true).is_err());
}

#[test]
fn native_runtime_absent_keeps_the_existing_sidecar_ladder() {
    let _guard = super::super::NATIVE_RUNTIME_ENV_LOCK.lock().unwrap();
    let _restore = NativeRuntimeEnvRestore::capture();
    std::env::remove_var(cut_native_runtime_context::CONTEXT_ENV);
    let runtime = sidecar_runtime().unwrap();
    assert!(runtime.native_context.is_none());
}

#[test]
fn native_runtime_set_selects_the_declared_python_member() {
    let _guard = super::super::NATIVE_RUNTIME_ENV_LOCK.lock().unwrap();
    let _restore = NativeRuntimeEnvRestore::capture();
    let temp = tempfile::tempdir().unwrap();
    let python_root = temp.path().join("python-runtime");
    let tools_root = temp.path().join("media-tools");
    std::fs::create_dir_all(&python_root).unwrap();
    std::fs::create_dir_all(tools_root.join("bin")).unwrap();
    let python_root = std::fs::canonicalize(python_root).unwrap();
    let tools_root = std::fs::canonicalize(tools_root).unwrap();
    let interpreter = python_root.join("python");
    let import = python_root.join("onnx_asr.py");
    let ffmpeg = tools_root.join("bin").join("ffmpeg");
    let ffprobe = tools_root.join("bin").join("ffprobe");
    for path in [&interpreter, &import, &ffmpeg, &ffprobe] {
        std::fs::write(path, b"fixture").unwrap();
    }
    let locator = temp.path().join("native-runtime-context.json");
    let context = serde_json::json!({
        "schema": cut_native_runtime_context::SET_CONTEXT_CONTRACT,
        "runtimes": [
            {"id":"media-tools", "context": {
                "schema": cut_native_runtime_context::PINNED_EXECUTABLE_BUNDLE_CONTEXT_CONTRACT,
                "root": tools_root,
                "manifestSha256": "a".repeat(64),
                "receiptSha256": "b".repeat(64),
                "platform": std::env::consts::OS,
                "architecture": std::env::consts::ARCH,
                "executables": [
                    {"name":"ffmpeg", "path": ffmpeg, "sha256":"c".repeat(64), "bytes":7, "mode":493},
                    {"name":"ffprobe", "path": ffprobe, "sha256":"d".repeat(64), "bytes":7, "mode":493}
                ],
                "files":2,
                "totalBytes":14
            }},
            {"id":"python", "context": {
                "schema": cut_native_runtime_context::CONTEXT_CONTRACT,
                "root": python_root,
                "manifestSha256": "e".repeat(64),
                "receiptSha256": "f".repeat(64),
                "interpreter":{"path": interpreter, "sha256":"0".repeat(64), "version":"3.12.13"},
                "imports":[{"module":"onnx_asr", "path":import, "sha256":"1".repeat(64)}],
                "models":[],
                "files":2,
                "totalBytes":14
            }}
        ]
    });
    std::fs::write(&locator, serde_json::to_vec(&context).unwrap()).unwrap();
    std::env::set_var(cut_native_runtime_context::CONTEXT_ENV, &locator);
    let selected = native_runtime_context().unwrap().unwrap();
    assert_eq!(selected.interpreter.path, python_root.join("python"));
    assert_eq!(selected.imports[0].module, "onnx_asr");
}

#[test]
fn invalid_native_runtime_never_falls_through_to_the_legacy_python() {
    let _guard = super::super::NATIVE_RUNTIME_ENV_LOCK.lock().unwrap();
    let _restore = NativeRuntimeEnvRestore::capture();
    let temp = tempfile::tempdir().unwrap();
    let invalid = temp.path().join("native-runtime-context.json");
    std::fs::write(&invalid, b"not json").unwrap();
    std::env::set_var(cut_native_runtime_context::CONTEXT_ENV, &invalid);
    assert!(sidecar_runtime().is_err());
}

#[test]
fn native_runtime_environment_is_restored_during_unwind() {
    let _guard = super::super::NATIVE_RUNTIME_ENV_LOCK.lock().unwrap();
    let _restore = NativeRuntimeEnvRestore::capture();
    for prior in [
        None,
        Some(std::ffi::OsString::from("prior-runtime-context")),
    ] {
        match &prior {
            Some(value) => std::env::set_var(cut_native_runtime_context::CONTEXT_ENV, value),
            None => std::env::remove_var(cut_native_runtime_context::CONTEXT_ENV),
        }
        let result = std::panic::catch_unwind(|| {
            let _restore = NativeRuntimeEnvRestore::capture();
            std::env::set_var(cut_native_runtime_context::CONTEXT_ENV, "abandoned-context");
            panic!("fixture assertion failure");
        });
        assert!(result.is_err());
        assert_eq!(
            std::env::var_os(cut_native_runtime_context::CONTEXT_ENV),
            prior
        );
    }
}

#[test]
fn native_python_command_policy_isolated_and_normal_compatible() {
    let mut normal = Command::new("python");
    apply_python_command_policy(&mut normal, false);
    assert_eq!(
        normal
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        ["-B"]
    );

    let mut command = Command::new("python");
    apply_python_command_policy(&mut command, true);
    assert_eq!(
        command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        ["-E", "-s", "-B"]
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

#[cfg(target_os = "linux")]
#[test]
fn native_python_command_ignores_hostile_pythonpath() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("sitecustomize.py"),
        "print('hostile-pythonpath')\n",
    )
    .unwrap();
    let mut command = Command::new("python3");
    apply_python_command_policy(&mut command, true);
    command
        .args(["-c", "print('native-safe')"])
        .env("PYTHONPATH", temp.path());
    let output = command
        .output()
        .expect("Linux source check requires python3 for the hostile-PYTHONPATH proof");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "native-safe\n",
        "-E must prevent PYTHONPATH from importing hostile sitecustomize"
    );
}

#[test]
fn native_script_resolution_uses_only_installed_resource_candidates() {
    let temp = tempfile::tempdir().unwrap();
    let macos = temp.path().join("Contents").join("MacOS");
    let resources = temp
        .path()
        .join("Contents")
        .join("Resources")
        .join("perception");
    std::fs::create_dir_all(&macos).unwrap();
    std::fs::create_dir_all(&resources).unwrap();
    let script = resources.join("instruments.py");
    std::fs::write(&script, b"fixture").unwrap();

    let bases = native_bundled_sidecar_bases(Some(&macos.join("cutd")));
    assert_eq!(
        native_bundled_sidecar_script_from_bases(&bases),
        Some(script),
        "native selection must resolve the application resource, not app-data or an env override"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn native_script_resolution_supports_the_deb_lib_payload() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("usr").join("bin");
    let resources = temp
        .path()
        .join("usr")
        .join("lib")
        .join("ShellX Cut")
        .join("perception");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&resources).unwrap();
    let script = resources.join("instruments.py");
    std::fs::write(&script, b"fixture").unwrap();

    let bases = native_bundled_sidecar_bases(Some(&bin.join("cutd")));
    assert_eq!(
        native_bundled_sidecar_script_from_bases(&bases),
        Some(script),
        "the native DEB route must bind its packaged /usr/lib payload"
    );
}

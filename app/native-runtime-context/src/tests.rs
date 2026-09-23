use super::*;
use std::io::Write;
use tempfile::TempDir;

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn fixture() -> (TempDir, PathBuf, RuntimeContext) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("runtime");
    fs::create_dir(&root).unwrap();
    let interpreter = root.join("python");
    let import = root.join("onnx_asr.py");
    let model = root.join("model.onnx");
    for (path, bytes) in [
        (&interpreter, b"python".as_slice()),
        (&import, b"import".as_slice()),
        (&model, b"model".as_slice()),
    ] {
        let mut file = File::create(path).unwrap();
        file.write_all(bytes).unwrap();
    }
    let root = fs::canonicalize(root).unwrap();
    let context = RuntimeContext {
        schema: CONTEXT_CONTRACT.into(),
        root: root.clone(),
        manifest_sha256: "a".repeat(64),
        receipt_sha256: "b".repeat(64),
        interpreter: Interpreter {
            path: root.join("python"),
            sha256: hash(b"python"),
            version: "3.12.13".into(),
        },
        imports: vec![Import {
            module: "onnx_asr".into(),
            path: root.join("onnx_asr.py"),
            sha256: hash(b"import"),
        }],
        models: vec![Model {
            id: "cut.stt.fixture.model".into(),
            path: root.join("model.onnx"),
            sha256: hash(b"model"),
            provenance_sha256: "c".repeat(64),
        }],
        files: 3,
        total_bytes: 17,
    };
    let context_path = temp.path().join("native-runtime-context.json");
    fs::write(&context_path, serde_json::to_vec(&context).unwrap()).unwrap();
    (temp, context_path, context)
}

fn bundle_fixture() -> (TempDir, PathBuf, PinnedExecutableBundleContext) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("runtime");
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let ffmpeg = bin.join("ffmpeg");
    let ffprobe = bin.join("ffprobe");
    fs::write(&ffmpeg, b"pinned ffmpeg bytes").unwrap();
    fs::write(&ffprobe, b"pinned ffprobe bytes").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&ffmpeg, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&ffprobe, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let root = fs::canonicalize(root).unwrap();
    let ffmpeg = root.join("bin").join("ffmpeg");
    let ffprobe = root.join("bin").join("ffprobe");
    let total_bytes = fs::metadata(&ffmpeg).unwrap().len() + fs::metadata(&ffprobe).unwrap().len();
    let context = PinnedExecutableBundleContext {
        schema: PINNED_EXECUTABLE_BUNDLE_CONTEXT_CONTRACT.into(),
        root: root.clone(),
        manifest_sha256: "a".repeat(64),
        receipt_sha256: "b".repeat(64),
        platform: std::env::consts::OS.into(),
        architecture: std::env::consts::ARCH.into(),
        executables: vec![
            PinnedExecutable {
                name: "ffmpeg".into(),
                path: ffmpeg,
                sha256: hash(b"pinned ffmpeg bytes"),
                bytes: b"pinned ffmpeg bytes".len() as u64,
                mode: 0o755,
            },
            PinnedExecutable {
                name: "ffprobe".into(),
                path: ffprobe,
                sha256: hash(b"pinned ffprobe bytes"),
                bytes: b"pinned ffprobe bytes".len() as u64,
                mode: 0o755,
            },
        ],
        files: 2,
        total_bytes,
    };
    let context_path = temp.path().join("native-runtime-context.json");
    fs::write(&context_path, serde_json::to_vec(&context).unwrap()).unwrap();
    (temp, context_path, context)
}

#[test]
fn accepts_context_and_streams_selected_pins() {
    let (_temp, path, _) = fixture();
    let context = RuntimeContext::from_path(&path).unwrap();
    context.verify_interpreter().unwrap();
    context.verify_imports().unwrap();
    context
        .verify_model(context.model("cut.stt.fixture.model").unwrap())
        .unwrap();
}

#[test]
fn accepts_runner_admitted_windows_cuda_size_but_rejects_larger_contexts() {
    let (_temp, path, mut context) = fixture();
    context.total_bytes = 9_383_536_384;
    fs::write(&path, serde_json::to_vec(&context).unwrap()).unwrap();
    assert_eq!(
        RuntimeContext::from_path(&path).unwrap().total_bytes,
        9_383_536_384
    );

    context.total_bytes = MAX_TOTAL_BYTES + 1;
    fs::write(&path, serde_json::to_vec(&context).unwrap()).unwrap();
    assert!(RuntimeContext::from_path(&path).is_err());
}

#[test]
fn rejects_unknown_field_escape_and_changed_selected_model() {
    let (_temp, path, context) = fixture();
    let mut value = serde_json::to_value(&context).unwrap();
    value["unexpected"] = serde_json::json!(true);
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(RuntimeContext::from_path(&path).is_err());

    let (_temp, path, context) = fixture();
    let mut value = serde_json::to_value(&context).unwrap();
    value["interpreter"]["path"] = serde_json::json!("/outside/python");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(RuntimeContext::from_path(&path).is_err());

    let (_temp, path, context) = fixture();
    fs::write(
        context.model("cut.stt.fixture.model").unwrap().path.clone(),
        b"changed",
    )
    .unwrap();
    let context = RuntimeContext::from_path(&path).unwrap();
    assert!(context
        .verify_model(context.model("cut.stt.fixture.model").unwrap())
        .is_err());
}

#[test]
fn accepts_pinned_executable_bundle_and_rehashes_selected_pair() {
    let (_temp, path, _) = bundle_fixture();
    let context = PinnedExecutableBundleContext::from_path(&path).unwrap();
    for name in ["ffmpeg", "ffprobe"] {
        context
            .verify_executable(context.executable(name).unwrap())
            .unwrap();
    }
}

#[test]
fn accepts_a_sorted_typed_runtime_context_set() {
    let (_python_temp, _python_path, python) = fixture();
    let (_bundle_temp, _bundle_path, bundle) = bundle_fixture();
    let set_temp = TempDir::new().unwrap();
    let set_path = set_temp.path().join("native-runtime-context.json");
    let set = serde_json::json!({
        "schema": SET_CONTEXT_CONTRACT,
        "runtimes": [
            {"id":"media-tools", "context": serde_json::to_value(&bundle).unwrap()},
            {"id":"python", "context": serde_json::to_value(&python).unwrap()}
        ]
    });
    fs::write(&set_path, serde_json::to_vec(&set).unwrap()).unwrap();

    let set = RuntimeContextSet::from_path(&set_path).unwrap();
    let python = set.python_context("python").unwrap();
    python.verify_interpreter().unwrap();
    python.verify_imports().unwrap();
    for name in ["ffmpeg", "ffprobe"] {
        let bundle = set.pinned_executable_bundle("media-tools").unwrap();
        bundle
            .verify_executable(bundle.executable(name).unwrap())
            .unwrap();
    }
    assert!(set.python_context("media-tools").is_err());
    assert!(set.pinned_executable_bundle("python").is_err());
}

#[test]
fn rejects_context_set_with_unsorted_or_invalid_members() {
    let (_python_temp, _python_path, python) = fixture();
    let (_bundle_temp, _bundle_path, bundle) = bundle_fixture();
    let set_temp = TempDir::new().unwrap();
    let set_path = set_temp.path().join("native-runtime-context.json");
    let base = serde_json::json!({
        "schema": SET_CONTEXT_CONTRACT,
        "runtimes": [
            {"id":"media-tools", "context": serde_json::to_value(&bundle).unwrap()},
            {"id":"python", "context": serde_json::to_value(&python).unwrap()}
        ]
    });

    let mut unsorted = base.clone();
    unsorted["runtimes"].as_array_mut().unwrap().swap(0, 1);
    fs::write(&set_path, serde_json::to_vec(&unsorted).unwrap()).unwrap();
    assert!(RuntimeContextSet::from_path(&set_path).is_err());

    let mut invalid_id = base;
    invalid_id["runtimes"][0]["id"] = serde_json::json!("media_tools");
    fs::write(&set_path, serde_json::to_vec(&invalid_id).unwrap()).unwrap();
    assert!(RuntimeContextSet::from_path(&set_path).is_err());
}

#[test]
fn rejects_pinned_bundle_context_drift_and_changed_selected_tool() {
    let (_temp, path, context) = bundle_fixture();
    let mut value = serde_json::to_value(&context).unwrap();
    value["unexpected"] = serde_json::json!(true);
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(PinnedExecutableBundleContext::from_path(&path).is_err());

    let (_temp, path, context) = bundle_fixture();
    let mut value = serde_json::to_value(&context).unwrap();
    value["platform"] = serde_json::json!("other-host");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(PinnedExecutableBundleContext::from_path(&path).is_err());

    let (_temp, path, context) = bundle_fixture();
    let mut value = serde_json::to_value(&context).unwrap();
    value["executables"][1]["name"] = serde_json::json!("ffmpeg");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(PinnedExecutableBundleContext::from_path(&path).is_err());

    let (_temp, path, context) = bundle_fixture();
    let mut value = serde_json::to_value(&context).unwrap();
    value["executables"][0]["mode"] = serde_json::json!(420);
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(PinnedExecutableBundleContext::from_path(&path).is_err());

    let (_temp, path, context) = bundle_fixture();
    fs::write(
        context.executable("ffprobe").unwrap().path.clone(),
        b"changed",
    )
    .unwrap();
    let context = PinnedExecutableBundleContext::from_path(&path).unwrap();
    assert!(context
        .verify_executable(context.executable("ffprobe").unwrap())
        .is_err());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let (_temp, path, context) = bundle_fixture();
        let ffmpeg = context.executable("ffmpeg").unwrap().path.clone();
        fs::set_permissions(&ffmpeg, fs::Permissions::from_mode(0o644)).unwrap();
        let context = PinnedExecutableBundleContext::from_path(&path).unwrap();
        assert!(context
            .verify_executable(context.executable("ffmpeg").unwrap())
            .is_err());
    }
}

#[cfg(windows)]
#[test]
fn windows_accepts_clean_nonverbatim_declared_paths() {
    let (_temp, path, mut context) = fixture();
    let root = path.parent().unwrap().join("runtime");
    context.root = root.clone();
    context.interpreter.path = root.join("python");
    context.imports[0].path = root.join("onnx_asr.py");
    context.models[0].path = root.join("model.onnx");
    fs::write(&path, serde_json::to_vec(&context).unwrap()).unwrap();

    let context = RuntimeContext::from_path(&path).unwrap();
    context.verify_interpreter().unwrap();
    context.verify_imports().unwrap();
}

#[cfg(windows)]
#[test]
fn windows_reparse_attribute_is_not_admitted() {
    assert!(windows_reparse_attributes(0x0400));
    assert!(windows_reparse_attributes(0x8400));
    assert!(!windows_reparse_attributes(0));
}

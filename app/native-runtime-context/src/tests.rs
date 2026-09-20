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

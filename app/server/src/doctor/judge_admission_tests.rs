//! Source tests for the bounded render-judge admission probe.

use super::*;
use crate::provider_runtime::ProviderChildLaunch;
use serde_json::{json, Map};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

fn test_python() -> Option<PathBuf> {
    let python = crate::dispatch::adapter_python::resolve_adapter_runtime(
        "judge/adapters/ladder_judge.py",
        || None,
    )
    .ok()?
    .python?;
    (Command::new(&python)
        .arg("--version")
        .status()
        .ok()?
        .success())
    .then_some(python)
}

fn provider_entry(root: &Path, id: &str, provider: &str) -> Value {
    let home = root.join(format!("{id}-home"));
    let mut canonical = Map::new();
    canonical.insert("home".into(), json!(home));
    let mut environment = Map::new();
    environment.insert("HOME".into(), json!(home));
    environment.insert("PATH".into(), json!(root.join(format!("{id}-bin"))));
    #[cfg(windows)]
    {
        let app_data = root.join(format!("{id}-app-data"));
        let local_app_data = root.join(format!("{id}-local-app-data"));
        canonical.insert("userProfile".into(), json!(home));
        canonical.insert("appData".into(), json!(app_data));
        canonical.insert("localAppData".into(), json!(local_app_data));
        environment.insert("USERPROFILE".into(), json!(home));
        environment.insert("APPDATA".into(), json!(app_data));
        environment.insert("LOCALAPPDATA".into(), json!(local_app_data));
        environment.insert("SystemRoot".into(), json!(root.join("windows")));
    }
    let executable = root.join(format!("{id}-provider"));
    #[cfg(windows)]
    let executable = executable.with_extension("exe");
    json!({
        "admission": {
            "schema": "release-runner.provider-runtime/v1",
            "resourceLease": format!("provider-login:test:{provider}"),
            "enrollment": {
                "id": id,
                "provider": provider,
                "userIdentity": if cfg!(windows) { "S-1-5-21-1000-1000-1000-1000" } else { "uid:1000" },
                "executable": { "path": executable, "sha256": "a".repeat(64) },
                "entrypoint": { "path": root.join(format!("{id}-entrypoint.mjs")), "sha256": "b".repeat(64) },
                "runtimeCode": [],
                "canonicalEnvironment": canonical
            }
        },
        "effectiveEnvironment": environment
    })
}

fn write_context(root: &Path) -> tempfile::NamedTempFile {
    let document = json!({
        "schema": "release-runner.provider-runtime/v2",
        "providers": [
            provider_entry(root, "claude", "claude"),
            provider_entry(root, "other", "other")
        ]
    });
    let mut file = tempfile::NamedTempFile::new_in(root).unwrap();
    file.write_all(&serde_json::to_vec(&document).unwrap())
        .unwrap();
    file.flush().unwrap();
    file
}

fn write_fake_adapter(path: &Path, marker: &Path) {
    let marker = serde_json::to_string(&marker.to_string_lossy()).unwrap();
    fs::write(
        path,
        [
            "import json, os, pathlib, sys\n",
            "assert sys.argv[1:] == ['detect', '--provider-launches-stdin']\n",
            "payload = json.load(sys.stdin)\n",
            "pathlib.Path(",
            &marker,
            ").write_text(json.dumps({'payload': payload, 'path': os.environ.get('PATH')}), encoding='utf-8')\n",
            "print(json.dumps({'rungs': [{'provider': 'claude', 'found': True, 'judge_ready': True}]}))\n",
        ]
        .concat(),
    )
    .unwrap();
}

fn write_native_policy_fake_adapter(path: &Path) {
    fs::write(
        path,
        [
            "import json, sys\n",
            "assert sys.flags.ignore_environment == 1\n",
            "assert sys.flags.no_user_site == 1\n",
            "assert sys.dont_write_bytecode\n",
            "assert sys.argv[1:] == ['detect']\n",
            "print(json.dumps({'rungs': [{'provider': 'claude', 'found': True, 'judge_ready': True}]}))\n",
        ]
        .concat(),
    )
    .unwrap();
}

#[test]
fn prepared_probe_applies_the_shared_python_policy() {
    let Some(python) = test_python() else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let adapter = root.path().join("native-policy-adapter.py");
    write_native_policy_fake_adapter(&adapter);

    assert!(matches!(
        probe(&adapter, &python, None, true),
        JudgeAdmissions::Verified(_)
    ));
}

#[test]
fn selected_context_probe_passes_exact_handoff_without_path_augmentation() {
    let Some(python) = test_python() else {
        return;
    };
    let root = tempfile::Builder::new()
        .prefix("judge-admission-selected-")
        .tempdir_in(crate::provider_runtime::test_fixture_root())
        .unwrap();
    let context = write_context(root.path());
    let _context =
        crate::provider_runtime::install_test_provider_context(context.path().as_os_str());
    let adapter = root.path().join("fake-adapter.py");
    let marker = root.path().join("fake-adapter.json");
    write_fake_adapter(&adapter, &marker);
    let injected_path = root.path().join("must-not-be-supplied-path");

    let admissions = probe(&adapter, &python, Some(injected_path.as_os_str()), false);

    assert!(matches!(admissions, JudgeAdmissions::Verified(_)));
    let observed: Value = serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
    assert_ne!(
        observed["path"],
        Value::String(injected_path.to_string_lossy().into_owned())
    );
    assert_eq!(
        observed["payload"]["schema"],
        crate::provider_runtime::PYTHON_CHILD_LAUNCHES_SCHEMA
    );
    assert_eq!(
        observed["payload"]["launches"]["claude"]["executable"],
        root.path()
            .join("claude-provider")
            .to_string_lossy()
            .as_ref()
    );
    assert!(observed["payload"]["launches"].get("other").is_none());
}

#[test]
fn malformed_context_refuses_before_the_doctor_probe_spawns() {
    let Some(python) = test_python() else {
        return;
    };
    let root = tempfile::Builder::new()
        .prefix("judge-admission-malformed-")
        .tempdir_in(crate::provider_runtime::test_fixture_root())
        .unwrap();
    let context = root.path().join("malformed-provider-runtime.json");
    fs::write(&context, b"{").unwrap();
    let _context = crate::provider_runtime::install_test_provider_context(context.as_os_str());
    let adapter = root.path().join("fake-adapter.py");
    let marker = root.path().join("fake-adapter.json");
    write_fake_adapter(&adapter, &marker);

    assert!(matches!(
        probe(&adapter, &python, None, false),
        JudgeAdmissions::Unverified
    ));
    assert!(!marker.exists());
}

#[test]
fn selected_context_probe_uses_only_admitted_judge_launches() {
    let launches = BTreeMap::from([
        (
            "claude".to_string(),
            ProviderChildLaunch {
                executable: PathBuf::from("/runner/claude"),
                entrypoint: Some(PathBuf::from("/runner/claude.mjs")),
                environment: BTreeMap::from([
                    ("HOME".to_string(), "/runner/claude-home".to_string()),
                    ("PATH".to_string(), "/runner/claude-bin".to_string()),
                ]),
            },
        ),
        (
            "other".to_string(),
            ProviderChildLaunch {
                executable: PathBuf::from("/runner/other"),
                entrypoint: None,
                environment: BTreeMap::from([(
                    "HOME".to_string(),
                    "/runner/other-home".to_string(),
                )]),
            },
        ),
    ]);
    let handoff: Value = serde_json::from_str(&admitted_probe_input(&launches)).unwrap();
    assert_eq!(
        handoff["schema"],
        crate::provider_runtime::PYTHON_CHILD_LAUNCHES_SCHEMA
    );
    assert_eq!(
        handoff["launches"]["claude"]["executable"],
        "/runner/claude"
    );
    assert_eq!(
        handoff["launches"]["claude"]["environment"]["HOME"],
        "/runner/claude-home"
    );
    assert!(handoff["launches"].get("other").is_none());
}

#[test]
fn parses_provider_admission_without_inferring_readiness() {
    let admissions = parse_detect_output(
        br#"{"rungs":[
            {"provider":"claude","found":true,"judge_ready":false,
             "restricted_read_reason":"restricted Read capability is unavailable"},
            {"provider":"codex","found":true,"judge_ready":false,
             "availability_reason":"render judge unavailable until restricted tool/file access is verified"},
            {"provider":"antigravity","found":true},
            {"provider":"grok","found":false,"judge_ready":true}
        ]}"#,
    )
    .expect("valid detect response");

    assert!(!admissions["claude"].judge_ready);
    assert_eq!(
        admissions["claude"].availability_reason.as_deref(),
        Some("restricted Read capability is unavailable")
    );
    assert!(!admissions["codex"].judge_ready);
    assert_eq!(
        admissions["codex"].availability_reason.as_deref(),
        Some("render judge unavailable until restricted tool/file access is verified")
    );
    assert!(!admissions["antigravity"].judge_ready);
    assert!(!admissions["grok"].judge_ready);
}

#[test]
fn resolve_keeps_installed_but_unready_provider_out_of_ok() {
    let admissions = JudgeAdmissions::Verified(BTreeMap::from([(
        "codex".into(),
        ProviderAdmission {
            found: true,
            judge_ready: false,
            availability_reason: Some(
                "render judge unavailable until restricted tool/file access is verified".into(),
            ),
        },
    )]));
    let codex = resolve("codex", true, true, &admissions);
    assert_eq!(codex.status, CardStatus::Degraded);
    assert!(!codex.judge_ready);
    assert_eq!(
        codex.availability_reason.as_deref(),
        Some("render judge unavailable until restricted tool/file access is verified")
    );

    let unverified = resolve("claude", true, true, &JudgeAdmissions::Unverified);
    assert_eq!(unverified.status, CardStatus::Unknown);
    assert!(!unverified.judge_ready);
}

#[test]
fn rejects_missing_or_duplicate_provider_protocol_entries() {
    assert!(parse_detect_output(br#"{"rungs":[{"found":true}]}"#).is_err());
    assert!(parse_detect_output(
        br#"{"rungs":[
            {"provider":"claude","found":true,"judge_ready":false,
             "restricted_read_reason":"restricted Read capability is unavailable"},
            {"provider":"claude","found":true,"judge_ready":true}
        ]}"#,
    )
    .is_err());
}

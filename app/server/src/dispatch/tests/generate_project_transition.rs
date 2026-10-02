//! Generate adapter completion must not materialize into a replacement project.

use super::*;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

const TEST_TIMEOUT: Duration = Duration::from_secs(2);
const MATERIALIZATION_TIMEOUT: Duration = Duration::from_secs(20);

struct EnvRestore {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvRestore {
    fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

async fn create_project(state: &AppState, name: &str, path: &Path) {
    // These fixtures exercise project ownership, not full-resolution rendering.
    let settings = cut_core::ProjectSettings {
        width: 320,
        height: 180,
        ..Default::default()
    };
    let result = dispatch(
        state,
        "project.create",
        json!({"name": name, "dir": path, "settings": settings}),
        test_actor(),
    )
    .await;
    assert!(result.ok, "{name} create failed: {:?}", result.error);
}

async fn open_a_with_same_revision(state: &AppState, root: &Path) -> (PathBuf, PathBuf) {
    let a_path = root.join("a.cutproj");
    let b_path = root.join("b.cutproj");
    create_project(state, "a", &a_path).await;
    create_project(state, "b", &b_path).await;
    let a = cut_core::ProjectStore::open(&a_path).unwrap();
    let b = cut_core::ProjectStore::open(&b_path).unwrap();
    assert_eq!(
        a.log.current_revision().unwrap(),
        b.log.current_revision().unwrap(),
        "the replacement fixture must have the same project-local revision"
    );
    let opened = dispatch(state, "project.open", json!({"path": a_path}), test_actor()).await;
    assert!(opened.ok, "A open failed: {:?}", opened.error);
    (a_path, b_path)
}

fn path_literal(path: &Path) -> String {
    serde_json::to_string(&path.to_string_lossy()).unwrap()
}

fn write_blocking_adapter(root: &Path, name: &str, result: &str) -> (PathBuf, PathBuf, PathBuf) {
    let adapter = root.join(format!("{name}_adapter.py"));
    let started = root.join(format!("{name}.started"));
    let release = root.join(format!("{name}.release"));
    let script = format!(
        r#"
import pathlib, sys, time
sys.stdin.read()
started = pathlib.Path({started})
release = pathlib.Path({release})
started.write_text('started', encoding='utf-8')
deadline = time.monotonic() + 5
while not release.exists() and time.monotonic() < deadline:
    time.sleep(0.01)
print({result})
"#,
        started = path_literal(&started),
        release = path_literal(&release),
        result = serde_json::to_string(result).unwrap(),
    );
    std::fs::write(&adapter, script).unwrap();
    (adapter, started, release)
}

async fn wait_for_adapter(path: &Path) {
    tokio::time::timeout(TEST_TIMEOUT, async {
        while !path.is_file() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("blocking adapter did not receive Generate request before test timeout");
}

async fn switch_to_b_then_release(
    state: &AppState,
    b_path: &Path,
    release: &Path,
    started: &Path,
    pending: tokio::task::JoinHandle<VerbResult>,
) -> VerbResult {
    wait_for_adapter(started).await;
    let switched = dispatch(state, "project.open", json!({"path": b_path}), test_actor()).await;
    assert!(switched.ok, "B open failed: {:?}", switched.error);
    std::fs::write(release, b"release").unwrap();
    tokio::time::timeout(TEST_TIMEOUT, pending)
        .await
        .expect("Generate request did not finish after adapter release")
        .expect("Generate task panicked")
}

async fn release_and_wait(
    release: &Path,
    started: &Path,
    pending: tokio::task::JoinHandle<VerbResult>,
) -> VerbResult {
    wait_for_adapter(started).await;
    std::fs::write(release, b"release").unwrap();
    tokio::time::timeout(MATERIALIZATION_TIMEOUT, pending)
        .await
        .expect("Generate request did not finish after adapter release")
        .expect("Generate task panicked")
}

async fn assert_b_is_unmodified(state: &AppState) {
    let state_result = dispatch(state, "project.state", json!({}), test_actor()).await;
    assert!(state_result.ok, "B state failed: {:?}", state_result.error);
    assert_eq!(state_result.result.as_ref().unwrap()["name"], "b");
    let ops = dispatch(state, "project.ops", json!({}), test_actor()).await;
    assert!(ops.ok, "B ops failed: {:?}", ops.error);
    assert_eq!(
        ops.result.as_ref().unwrap()["ops"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "Generate from A must not append a checkpoint or generated op to B"
    );
}

fn assert_project_conflict(result: &VerbResult) {
    assert!(!result.ok, "Generate unexpectedly completed: {result:?}");
    let error = result
        .error
        .as_ref()
        .expect("conflict envelope includes error");
    assert_eq!(error.code, error_codes::CONFLICT);
    assert_eq!(
        error.message,
        "Generate target project changed before materialization"
    );
}

#[tokio::test]
async fn generate_prompt_insert_refuses_equal_revision_project_opened_during_adapter() {
    let _env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let (_a_path, b_path) = open_a_with_same_revision(&state, root.path()).await;
    let plan = r#"{"schema":"shellx-cut/generate-plan/1","status":"completed","backend":{"provider":"fixture"},"plan":{"template_id":"builtin.lower-third.clean","params":{"name":"A"}},"warnings":[]}"#;
    let (adapter, started, release) = write_blocking_adapter(root.path(), "prompt", plan);
    let _adapter = EnvRestore::set("CUTD_GENERATE_PROMPT_ADAPTER", &adapter);

    let pending_state = state.clone();
    let pending = tokio::spawn(async move {
        dispatch(
            &pending_state,
            "generate.from_prompt",
            json!({"prompt": "insert into A", "policy": "insert", "agent": "auto"}),
            test_actor(),
        )
        .await
    });
    let result = switch_to_b_then_release(&state, &b_path, &release, &started, pending).await;

    assert_project_conflict(&result);
    assert_b_is_unmodified(&state).await;
}

#[tokio::test]
async fn generate_storyboard_insert_refuses_equal_revision_project_opened_during_adapter() {
    let _env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let (_a_path, b_path) = open_a_with_same_revision(&state, root.path()).await;
    let storyboard = r#"{"schema":"shellx-cut/generate-storyboard-result/1","status":"completed","backend":{"provider":"fixture"},"questions":[],"warnings":[],"storyboard":{"schema":"shellx-cut/generate-storyboard/1","storyboard_id":"from-a","mode":"quick_prompt","status":"valid","scenes":[{"scene_id":"one","index":1,"role":"title","source":"generate_template","template_id":"builtin.title-card.episode","range_ms":[0,1000],"params":{"title":"A"}},{"scene_id":"two","index":2,"role":"lower-third","source":"generate_template","template_id":"builtin.lower-third.clean","range_ms":[1000,2000],"params":{"name":"A"}}]}}"#;
    let (adapter, started, release) = write_blocking_adapter(root.path(), "storyboard", storyboard);
    let _adapter = EnvRestore::set("CUTD_GENERATE_STORYBOARD_ADAPTER", &adapter);

    let pending_state = state.clone();
    let pending = tokio::spawn(async move {
        dispatch(
            &pending_state,
            "generate.storyboard",
            json!({"input": "insert two scenes into A", "mode": "quick_prompt", "policy": "insert", "agent": "auto"}),
            test_actor(),
        )
        .await
    });
    let result = switch_to_b_then_release(&state, &b_path, &release, &started, pending).await;

    assert_project_conflict(&result);
    assert_b_is_unmodified(&state).await;
}

#[tokio::test]
async fn generate_storyboard_insert_completes_nested_insert_while_project_is_pinned() {
    let _env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let (_a_path, _b_path) = open_a_with_same_revision(&state, root.path()).await;
    let storyboard = r#"{"schema":"shellx-cut/generate-storyboard-result/1","status":"completed","backend":{"provider":"fixture"},"questions":[],"warnings":[],"storyboard":{"schema":"shellx-cut/generate-storyboard/1","storyboard_id":"pinned","mode":"quick_prompt","status":"valid","scenes":[{"scene_id":"one","index":1,"role":"callout","source":"generate_template","template_id":"builtin.callout.arrow-label","range_ms":[0,250],"params":{"label":"A","duration_ms":250}}]}}"#;
    let (adapter, started, release) = write_blocking_adapter(root.path(), "pinned", storyboard);
    let _adapter = EnvRestore::set("CUTD_GENERATE_STORYBOARD_ADAPTER", &adapter);

    let pending_state = state.clone();
    let pending = tokio::spawn(async move {
        dispatch(
            &pending_state,
            "generate.storyboard",
            json!({"input": "insert a scene into A", "mode": "quick_prompt", "policy": "insert", "agent": "auto"}),
            test_actor(),
        )
        .await
    });
    let result = release_and_wait(&release, &started, pending).await;

    assert!(
        result.ok,
        "pinned storyboard insert failed: {:?}",
        result.error
    );
    assert_eq!(
        result.result.as_ref().unwrap()["insert"]["policy"],
        "insert"
    );
    let ops = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    assert!(ops.ok, "project ops failed: {:?}", ops.error);
    assert!(
        ops.result.as_ref().unwrap()["ops"]
            .as_array()
            .unwrap()
            .len()
            > 1,
        "nested Generate inserts must materialize after the adapter releases"
    );
    let inserted = &result.result.as_ref().unwrap()["insert"];
    assert!(!inserted["checkpoints"].as_array().unwrap().is_empty());
    assert!(!inserted["clips"].as_array().unwrap().is_empty());
    let project = state.project.read().await;
    let store = project.as_ref().unwrap();
    assert_eq!(store.project.name, "a");
    for asset in inserted["assets"].as_array().unwrap() {
        let source = &store.project.assets[asset.as_str().unwrap()].path;
        assert!(Path::new(source).is_file(), "generated overlay is missing");
    }
    assert!(!inserted["assets"].as_array().unwrap().is_empty());
}

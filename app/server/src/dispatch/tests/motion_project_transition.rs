//! Motion connector results must materialize into the project that started them.

use super::*;
use crate::motion_bridge::{install_motion_request_build_gate, MotionRequestBuildGate};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(2);
const MATERIALIZE: Duration = Duration::from_secs(20);

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
    let result = dispatch(
        state,
        "project.create",
        json!({"name":name,"dir":path}),
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
        b.log.current_revision().unwrap()
    );
    let opened = dispatch(state, "project.open", json!({"path":a_path}), test_actor()).await;
    assert!(opened.ok, "A open failed: {:?}", opened.error);
    (a_path, b_path)
}

fn shell_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn write_blocking_motion_connector(root: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let cli = root.join("motion-cli.sh");
    let templates = root.join("motion-templates");
    let package = templates.join("editable-lower-third");
    let started = root.join("motion.started");
    let release = root.join("motion.release");
    let render = root.join("motion-render.mp4");
    let plan = root.join("motion-plan.json");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("manifest.json"), b"{}\n").unwrap();
    std::fs::write(&render, b"fixture Motion output").unwrap();
    std::fs::write(&plan, r##"{
  "schema":"shellx-motion/cut-import-plan@1", "ok":true,
  "packageId":"pkg-fixture", "motionId":"motion-fixture", "targetId":"shellx-cut",
  "mode":"editable_lowering", "unsupported":[],
  "document":{"width":1920,"height":1080,"fps":30,"durationMs":200},
  "operations":[{"verb":"cut.shape.create","sourceLayerId":"panel","startMs":0,"durationMs":200,"payload":{"shape":"rounded-rect","fill":"#112233","transform":{"x":64,"y":108,"width":960,"height":216}}}],
  "receipt":{"schema":"shellx-motion/receipt@1","id":"fixture","operation":"cut.import.plan","status":"passed","packageId":"pkg-fixture","inputHashes":{},"createdAt":"2026-07-12T00:00:00Z","lane":"cut","output":{"mode":"editable_lowering"},"warnings":[]}
}"##).unwrap();
    let connector = json!({"ok":true,"render":{"outputPath":render},"cutPlanPath":plan});
    std::fs::write(
        &cli,
        format!(
            "#!/bin/sh\n: > {}\nwhile [ ! -f {} ]; do sleep 0.01; done\nprintf '%s\\n' {}\n",
            shell_literal(&started.display().to_string()),
            shell_literal(&release.display().to_string()),
            shell_literal(&connector.to_string()),
        ),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&cli).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&cli, permissions).unwrap();
    (cli, templates, started, release)
}

async fn wait_for(path: &Path) {
    tokio::time::timeout(WAIT, async {
        while !path.is_file() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fake Motion connector did not start");
}

struct MotionRequestBuildGateReset;

impl Drop for MotionRequestBuildGateReset {
    fn drop(&mut self) {
        install_motion_request_build_gate(None);
    }
}

async fn direct_motion_materialization_pins_a(
    verb: &'static str,
    args: serde_json::Value,
    pin_before_request_build: bool,
) {
    let _motion_env = crate::motion_bridge::MOTION_ENV_LOCK.lock().await;
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let (a_path, b_path) = open_a_with_same_revision(&state, root.path()).await;
    let (cli, templates, started, release) = write_blocking_motion_connector(root.path());
    let _cli = EnvRestore::set("SHELLX_MOTION_CLI", &cli);
    let _templates = EnvRestore::set("SHELLX_MOTION_TEMPLATE_ROOT", &templates);

    let request_build_gate = pin_before_request_build.then(MotionRequestBuildGate::new);
    if let Some(gate) = &request_build_gate {
        install_motion_request_build_gate(Some(gate.clone()));
    }
    let _request_build_gate_reset = MotionRequestBuildGateReset;
    let pinned_before_request_build = request_build_gate
        .as_ref()
        .map(|gate| gate.project_pinned.notified());
    let pending_state = state.clone();
    let mut pending =
        tokio::spawn(async move { dispatch(&pending_state, verb, args, test_actor()).await });
    if let Some(pinned) = pinned_before_request_build {
        if tokio::time::timeout(WAIT, pinned).await.is_err() {
            let result = tokio::time::timeout(WAIT, &mut pending).await;
            panic!("Motion did not pin A before deriving project paths: {result:?}");
        }
    } else {
        wait_for(&started).await;
    }
    assert!(
        state.project_transition.try_lock().is_err(),
        "{verb} must pin A before deriving project paths and through its connector wait"
    );

    let switch_state = state.clone();
    let mut switch = tokio::spawn(async move {
        dispatch(
            &switch_state,
            "project.open",
            json!({"path":b_path}),
            test_actor(),
        )
        .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut switch)
            .await
            .is_err(),
        "B must not replace A while {verb} derives project paths or waits for Motion"
    );
    if let Some(gate) = &request_build_gate {
        gate.continue_after_pin.notify_one();
        wait_for(&started).await;
    }
    std::fs::write(&release, b"release").unwrap();
    let inserted = tokio::time::timeout(MATERIALIZE, pending)
        .await
        .expect("Motion materialization did not finish")
        .expect("Motion task panicked");
    assert!(inserted.ok, "{verb} failed: {inserted:?}");
    assert!(
        switch.await.expect("B open task panicked").ok,
        "B opens after A materializes"
    );

    let b_ops = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    assert!(
        b_ops.ok
            && b_ops.result.as_ref().unwrap()["ops"]
                .as_array()
                .unwrap()
                .len()
                == 1,
        "{verb} must not append a checkpoint or import to B"
    );
    let reopened = dispatch(&state, "project.open", json!({"path":a_path}), test_actor()).await;
    assert!(reopened.ok, "A reopen failed: {:?}", reopened.error);
    let a_ops = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    let a_ops = a_ops
        .ok
        .then_some(a_ops.result)
        .flatten()
        .and_then(|result| result["ops"].as_array().cloned())
        .expect("A operation log must be readable");
    assert!(
        a_ops.len() > 1,
        "{verb} must materialize in A before B can open"
    );
    assert!(
        a_ops
            .iter()
            .any(|op| { op["verb"] == "project.checkpoint" && op["actor"]["name"] == "test" }),
        "{verb} must keep its checkpoint under the initiating actor"
    );
    assert!(
        a_ops.iter().any(|op| {
            op["verb"] == "motion.apply_import"
                && op["actor"]["name"] == "test"
                && op["args"]["idempotency_key"].as_str().is_some()
        }),
        "{verb} must preserve actor and idempotency through the raw Motion materialization"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn generate_insert_motion_holds_a_until_its_connector_materializes() {
    direct_motion_materialization_pins_a(
        "generate.insert",
        json!({"id":"builtin.motion.lower-third","params":{"title":"A"}}),
        false,
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn motion_template_to_cut_pins_a_before_building_paths_and_through_connector() {
    direct_motion_materialization_pins_a(
        "motion.template_to_cut",
        json!({"template":"editable-lower-third","policy":"insert"}),
        true,
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn motion_script_to_cut_pins_a_before_building_paths_and_through_connector() {
    direct_motion_materialization_pins_a(
        "motion.script_to_cut",
        json!({"policy":"insert","script":{"schema":"shellx-motion/scripted-video@1","id":"fixture","frames":[]}}),
        true,
    )
    .await;
}

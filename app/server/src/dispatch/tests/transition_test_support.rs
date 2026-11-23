//! Shared deterministic helpers for delayed project-materialization regressions.

use super::*;
use crate::dispatch::edit_tools::{
    install_assemble_broll_search_gate, install_assets_fetch_project_transition_gate,
};
use crate::state::AppState;
use std::time::Duration;

/// Upper bound for every wait in these tests. Each wait ends as soon as the
/// state it names is reached; the bound only catches a task that never gets
/// there, so it stays far above what a slow or busy machine needs.
pub(super) const TEST_TIMEOUT: Duration = Duration::from_secs(60);

/// These tests check what a project change does to work that is in flight, not
/// how fast background workers stop. Let the change wait for them for as long
/// as the other waits here, so a slow machine cannot turn a worker that is
/// still stopping into a refused change.
pub(super) fn allow_slow_worker_stop(state: &AppState) {
    state.jobs.set_drain_timeout_for_test(TEST_TIMEOUT);
}

pub(super) const ONE_BY_ONE_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240, 31, 0,
    5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

pub(super) fn transition_gate_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

pub(super) struct AssetsFetchTransitionGateReset;

impl Drop for AssetsFetchTransitionGateReset {
    fn drop(&mut self) {
        install_assets_fetch_project_transition_gate(None);
        install_assemble_broll_search_gate(None);
    }
}

pub(super) async fn create_project(state: &AppState, name: &str, path: &std::path::Path) {
    let created = dispatch(
        state,
        "project.create",
        json!({"name": name, "dir": path}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{name} create failed: {:?}", created.error);
}

pub(super) fn broll_actor() -> Actor {
    Actor {
        kind: cut_core::ActorKind::Agent,
        name: "broll-owner".into(),
        via: "test".into(),
        request: None,
    }
}

//! Test-only synchronization for relink ownership regression tests.

#[derive(Clone)]
pub(super) struct RelinkProjectTransitionGate {
    root: String,
    pub(super) project_pinned: std::sync::Arc<tokio::sync::Notify>,
    pub(super) continue_after_pin: std::sync::Arc<tokio::sync::Notify>,
    pub(super) relink_committed: std::sync::Arc<tokio::sync::Notify>,
    pub(super) continue_after_commit: std::sync::Arc<tokio::sync::Notify>,
}

impl RelinkProjectTransitionGate {
    pub(super) fn new(root: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            project_pinned: std::sync::Arc::new(tokio::sync::Notify::new()),
            continue_after_pin: std::sync::Arc::new(tokio::sync::Notify::new()),
            relink_committed: std::sync::Arc::new(tokio::sync::Notify::new()),
            continue_after_commit: std::sync::Arc::new(tokio::sync::Notify::new()),
        }
    }
}

static RELINK_PROJECT_TRANSITION_GATE: std::sync::OnceLock<
    std::sync::Mutex<Option<RelinkProjectTransitionGate>>,
> = std::sync::OnceLock::new();

fn gate() -> &'static std::sync::Mutex<Option<RelinkProjectTransitionGate>> {
    RELINK_PROJECT_TRANSITION_GATE.get_or_init(|| std::sync::Mutex::new(None))
}

pub(super) fn install_relink_project_transition_gate(value: Option<RelinkProjectTransitionGate>) {
    *gate()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = value;
}

fn current(root: &str) -> Option<RelinkProjectTransitionGate> {
    gate()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .filter(|value| value.root == root)
        .cloned()
}

pub(super) async fn wait_for_relink_project_transition_gate_after_pin(root: &str) {
    let Some(value) = current(root) else {
        return;
    };
    value.project_pinned.notify_one();
    value.continue_after_pin.notified().await;
}

pub(super) async fn wait_for_relink_project_transition_gate_after_commit(root: &str) {
    let Some(value) = current(root) else {
        return;
    };
    value.relink_committed.notify_one();
    value.continue_after_commit.notified().await;
}

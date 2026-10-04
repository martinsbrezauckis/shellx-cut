//! Test-only pause after a real first queue dry-run, before job admission.

#[derive(Clone)]
pub(in crate::dispatch) struct RenderQueueTransitionGate {
    rationale: String,
    pub(in crate::dispatch) reached: std::sync::Arc<tokio::sync::Notify>,
    pub(in crate::dispatch) release: std::sync::Arc<tokio::sync::Notify>,
}

impl RenderQueueTransitionGate {
    pub(in crate::dispatch) fn new(rationale: impl Into<String>) -> Self {
        Self {
            rationale: rationale.into(),
            reached: std::sync::Arc::new(tokio::sync::Notify::new()),
            release: std::sync::Arc::new(tokio::sync::Notify::new()),
        }
    }
}

static GATE: std::sync::OnceLock<std::sync::Mutex<Option<RenderQueueTransitionGate>>> =
    std::sync::OnceLock::new();

fn gate() -> &'static std::sync::Mutex<Option<RenderQueueTransitionGate>> {
    GATE.get_or_init(|| std::sync::Mutex::new(None))
}

pub(in crate::dispatch) fn install_render_queue_transition_gate(
    value: Option<RenderQueueTransitionGate>,
) {
    *gate()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = value;
}

pub(super) async fn wait_after_first_preflight(rationale: Option<&str>, index: usize) {
    if index != 0 {
        return;
    }
    let value = gate()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .filter(|gate| rationale == Some(gate.rationale.as_str()))
        .cloned();
    if let Some(value) = value {
        value.reached.notify_one();
        value.release.notified().await;
    }
}

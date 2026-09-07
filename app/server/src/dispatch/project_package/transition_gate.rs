//! Test-only synchronization for package ownership regression tests.

#[derive(Clone)]
pub(super) struct PackageProjectTransitionGate {
    package_name: String,
    pub(super) project_pinned: std::sync::Arc<tokio::sync::Notify>,
    pub(super) continue_after_pin: std::sync::Arc<tokio::sync::Notify>,
    pub(super) job_admitted: std::sync::Arc<tokio::sync::Notify>,
    pub(super) continue_after_admission: std::sync::Arc<tokio::sync::Notify>,
}

impl PackageProjectTransitionGate {
    pub(super) fn new(package_name: impl Into<String>) -> Self {
        Self {
            package_name: package_name.into(),
            project_pinned: std::sync::Arc::new(tokio::sync::Notify::new()),
            continue_after_pin: std::sync::Arc::new(tokio::sync::Notify::new()),
            job_admitted: std::sync::Arc::new(tokio::sync::Notify::new()),
            continue_after_admission: std::sync::Arc::new(tokio::sync::Notify::new()),
        }
    }
}

static PACKAGE_PROJECT_TRANSITION_GATE: std::sync::OnceLock<
    std::sync::Mutex<Option<PackageProjectTransitionGate>>,
> = std::sync::OnceLock::new();

fn gate() -> &'static std::sync::Mutex<Option<PackageProjectTransitionGate>> {
    PACKAGE_PROJECT_TRANSITION_GATE.get_or_init(|| std::sync::Mutex::new(None))
}

pub(super) fn install_package_project_transition_gate(value: Option<PackageProjectTransitionGate>) {
    *gate()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = value;
}

fn current(package_name: &str) -> Option<PackageProjectTransitionGate> {
    gate()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .filter(|value| value.package_name == package_name)
        .cloned()
}

pub(super) async fn wait_for_package_project_transition_gate_after_pin(package_name: &str) {
    let Some(value) = current(package_name) else {
        return;
    };
    value.project_pinned.notify_one();
    value.continue_after_pin.notified().await;
}

pub(super) async fn wait_for_package_project_transition_gate_after_admission(package_name: &str) {
    let Some(value) = current(package_name) else {
        return;
    };
    value.job_admitted.notify_one();
    value.continue_after_admission.notified().await;
}

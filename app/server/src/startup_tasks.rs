//! Background startup work that must never delay the loopback listener or UI.

use crate::state::AppState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

/// One-shot handoff from the mounted Cut app root to the startup doctor task.
///
/// The engine is loopback-only, and this signal travels only over the existing
/// loopback/origin-guarded UI WebSocket after that socket registered as a UI
/// client. It deliberately has no timer fallback: headless/API sessions must
/// not launch hardware probes merely because their listener became available.
#[derive(Clone, Default)]
pub(crate) struct UiMountReadiness {
    mounted: Arc<AtomicBool>,
    wake: Arc<Notify>,
}

impl UiMountReadiness {
    /// Mark the first actual Cut app-root mount. Returns true only for that
    /// first notice, so reconnects and React development remounts are harmless.
    pub(crate) fn notify_mounted(&self) -> bool {
        if self.mounted.swap(true, Ordering::AcqRel) {
            return false;
        }
        // `notify_one` retains a permit if the startup task has not registered
        // its waiter yet. `notify_waiters` would lose that exact boundary race.
        self.wake.notify_one();
        true
    }

    async fn wait_for_mount(&self) {
        while !self.mounted.load(Ordering::Acquire) {
            // A concurrent first mount either wakes this waiter or leaves a
            // retained permit for it, so this cannot strand an untriggered
            // startup task at the check-to-wait boundary.
            self.wake.notified().await;
        }
    }
}

pub(crate) fn spawn_post_ui_doctor(state: AppState) -> tokio::task::JoinHandle<()> {
    let readiness = state.ui_mount_readiness.clone();
    tokio::spawn(async move {
        readiness.wait_for_mount().await;
        // Reuse an explicit/UI lazy scan if it won the first post-mount race.
        // The first mount still warms a cold server, but never launches a
        // second automatic hardware probe for the same process lifetime.
        let report = state.doctor_cached().await;
        tracing::info!(
            "doctor: {} cards, ffmpeg {}",
            report.cards.len(),
            if report.essential_ok {
                "ok"
            } else {
                "MISSING (wizard will surface)"
            }
        );
    })
}

#[cfg(test)]
mod tests {
    use super::{spawn_post_ui_doctor, UiMountReadiness};
    use crate::state::AppState;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn first_mount_runs_the_waiting_warm_once_and_duplicates_are_idempotent() {
        let readiness = UiMountReadiness::default();
        let runs = Arc::new(AtomicUsize::new(0));
        let worker_readiness = readiness.clone();
        let worker_runs = runs.clone();
        let worker = tokio::spawn(async move {
            worker_readiness.wait_for_mount().await;
            worker_runs.fetch_add(1, Ordering::SeqCst);
        });

        tokio::task::yield_now().await;
        assert_eq!(
            runs.load(Ordering::SeqCst),
            0,
            "no mount means no startup warm"
        );
        assert!(readiness.notify_mounted());
        assert!(!readiness.notify_mounted());
        assert!(!readiness.notify_mounted());
        tokio::time::timeout(Duration::from_secs(1), worker)
            .await
            .expect("first mount must wake the startup task")
            .expect("startup task must not panic");
        assert_eq!(
            runs.load(Ordering::SeqCst),
            1,
            "duplicates must not rerun warm"
        );
    }

    #[tokio::test]
    async fn mount_notice_racing_wait_registration_never_strands_the_warm() {
        use tokio::sync::Barrier;

        for _ in 0..128 {
            let readiness = UiMountReadiness::default();
            let barrier = Arc::new(Barrier::new(2));
            let worker_readiness = readiness.clone();
            let worker_barrier = barrier.clone();
            let worker = tokio::spawn(async move {
                worker_barrier.wait().await;
                worker_readiness.wait_for_mount().await;
            });
            let notifier_barrier = barrier.clone();
            let notifier = tokio::spawn(async move {
                notifier_barrier.wait().await;
                readiness.notify_mounted();
            });

            tokio::time::timeout(Duration::from_secs(1), notifier)
                .await
                .expect("mount notifier must complete")
                .expect("mount notifier must not panic");
            tokio::time::timeout(Duration::from_secs(1), worker)
                .await
                .expect("racing mount notice must wake startup task")
                .expect("startup task must not panic");
        }
    }

    #[tokio::test]
    async fn unmounted_startup_doctor_is_immediately_abortable_for_shutdown() {
        let state = AppState::new();
        let doctor = spawn_post_ui_doctor(state.clone());
        tokio::task::yield_now().await;
        assert!(
            state.doctor.read().await.is_none(),
            "an unmounted UI must not start the doctor before shutdown"
        );
        doctor.abort();
        let result = tokio::time::timeout(Duration::from_secs(1), doctor)
            .await
            .expect("shutdown must not wait on an unmounted UI");
        assert!(
            result
                .expect_err("aborted doctor must not complete")
                .is_cancelled(),
            "the unmounted doctor must finish as a cancellation"
        );
    }

    #[test]
    fn startup_warm_has_no_time_based_fallback() {
        const SOURCE: &str = include_str!("startup_tasks.rs");
        let timer_sleep = ["tokio::time", "sleep"].join("::");
        let legacy_delay = ["UI_SETTLE_", "DELAY"].concat();
        assert!(
            !SOURCE.contains(&timer_sleep),
            "a fixed post-listener delay would probe before a real UI mount"
        );
        assert!(
            !SOURCE.contains(&legacy_delay),
            "startup readiness is a UI mount handshake, not a settle timeout"
        );
    }
}

use crate::state::AppState;

/// Mark foreground media work cancelled before accepting the process signal as
/// terminal server shutdown. On Unix the native qualification harness sends
/// SIGTERM, while interactive shells normally use Ctrl-C. An engine started by
/// the desktop shell also shuts down, through this same path, when that shell
/// disappears without closing it (see [`crate::desktop_parent`]).
///
/// The SIGTERM handler is installed when this is CALLED, not when the returned
/// future first runs: the engine calls it before it announces itself
/// (`engine.addr`). Installed later, a SIGTERM that arrived between the
/// announcement and the server's first poll (a loaded machine) took the
/// default action and ended the engine without its cleanup, leaving a stale
/// `engine.addr`.
// Windows has no handler to install at the call, so the body reads like an
// async fn there; the plain fn is what keeps the Unix installation eager.
#[cfg_attr(not(unix), allow(clippy::manual_async_fn))]
pub(crate) fn wait_for_server_shutdown(state: AppState) -> impl std::future::Future<Output = ()> {
    #[cfg(unix)]
    let terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
    async move {
        #[cfg(unix)]
        match terminate {
            Ok(terminate) => wait_for_unix_server_shutdown(state, terminate).await,
            Err(error) => {
                tracing::warn!(%error, "SIGTERM handler unavailable; waiting for Ctrl-C");
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = crate::desktop_parent::wait_for_parent_exit() => {}
                }
                state.request_server_shutdown();
            }
        }
        #[cfg(not(unix))]
        {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = crate::desktop_parent::wait_for_parent_exit() => {}
            }
            state.request_server_shutdown();
        }
        // A running recording is finished and saved before the server stops, so a
        // shutdown never cuts a take off. Bounded by Stop's own longest budget.
        crate::screen_record::active_capture::finish_for_shutdown(std::time::Duration::from_secs(
            15 * 60,
        ))
        .await;
        // The engine's private working folder goes with it (item 98).
        crate::output_paths::monitor_scratch::shutdown();
    }
}

/// How long the process waits, after the server stopped, for blocking work
/// that is still running (an export's encoder pipe reads) before it exits.
pub(crate) const BLOCKING_WORK_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// Run the engine, then drop its runtime with a bound. A plain drop waits for
/// every blocking worker: an engine asked to stop (SIGTERM, Ctrl-C, its shell
/// gone) during an export kept running until the export ended on its own
/// (28 s for a one-minute 1080p export). Exiting after the grace is the same
/// as the engine being killed: its helpers are stopped (lifeline watcher, or
/// the Windows job object), and the next start says which export was
/// interrupted and removes its unfinished files.
pub(crate) fn block_on_bounded<T>(
    runtime: tokio::runtime::Runtime,
    engine: impl std::future::Future<Output = T>,
) -> T {
    let output = runtime.block_on(engine);
    runtime.shutdown_timeout(BLOCKING_WORK_GRACE);
    output
}

#[cfg(unix)]
async fn wait_for_unix_server_shutdown(
    state: AppState,
    mut terminate: tokio::signal::unix::Signal,
) {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
        _ = crate::desktop_parent::wait_for_parent_exit() => {}
    }
    state.request_server_shutdown();
}

#[cfg(all(test, unix))]
mod shutdown_tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    #[tokio::test(flavor = "current_thread")]
    async fn sigterm_marks_foreground_media_shutdown_before_server_exit() {
        let state = AppState::new();
        let probe = state.server_shutdown_probe();
        let terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        let waiting = tokio::spawn(wait_for_unix_server_shutdown(state, terminate));
        unsafe { libc::kill(libc::getpid(), libc::SIGTERM) };
        tokio::time::timeout(Duration::from_secs(2), waiting)
            .await
            .expect("SIGTERM handler did not complete")
            .expect("SIGTERM handler task panicked");
        assert!(probe.load(Ordering::Acquire));
    }

    /// A SIGTERM that arrives after the call but before the future first runs
    /// is still handled (the handler is installed by the call).
    #[tokio::test(flavor = "current_thread")]
    async fn a_sigterm_before_the_first_poll_is_not_lost() {
        let state = AppState::new();
        let probe = state.server_shutdown_probe();
        let shutdown = wait_for_server_shutdown(state);
        // Not polled yet. Without an installed handler this would end the test
        // process.
        unsafe { libc::kill(libc::getpid(), libc::SIGTERM) };
        tokio::time::timeout(Duration::from_secs(5), shutdown)
            .await
            .expect("the early SIGTERM was handled");
        assert!(probe.load(Ordering::Acquire));
    }
}

#[cfg(test)]
mod bounded_exit_tests {
    use super::*;

    /// Row 211: a blocking worker that keeps running (an export's encoder
    /// pipe) no longer holds the engine's exit past the grace.
    #[test]
    fn a_running_blocking_worker_does_not_hold_the_exit() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let started = std::time::Instant::now();
        let output = block_on_bounded(runtime, async {
            tokio::task::spawn_blocking(|| std::thread::sleep(std::time::Duration::from_secs(30)));
            7
        });
        assert_eq!(output, 7);
        let waited = started.elapsed();
        assert!(waited >= BLOCKING_WORK_GRACE, "{waited:?}");
        assert!(
            waited < BLOCKING_WORK_GRACE + std::time::Duration::from_secs(3),
            "{waited:?}"
        );
    }
}

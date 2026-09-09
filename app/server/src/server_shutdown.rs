use crate::state::AppState;

/// Mark foreground media work cancelled before accepting the process signal as
/// terminal server shutdown. On Unix the native qualification harness sends
/// SIGTERM, while interactive shells normally use Ctrl-C.
pub(crate) async fn wait_for_server_shutdown(state: AppState) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        match signal(SignalKind::terminate()) {
            Ok(terminate) => wait_for_unix_server_shutdown(state, terminate).await,
            Err(error) => {
                tracing::warn!(%error, "SIGTERM handler unavailable; waiting for Ctrl-C");
                let _ = tokio::signal::ctrl_c().await;
                state.request_server_shutdown();
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        state.request_server_shutdown();
    }
}

#[cfg(unix)]
async fn wait_for_unix_server_shutdown(
    state: AppState,
    mut terminate: tokio::signal::unix::Signal,
) {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
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
}

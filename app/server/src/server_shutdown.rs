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
        _ = wait_for_desktop_parent_exit() => {}
    }
    state.request_server_shutdown();
}

/// A macOS LaunchServices Quit can terminate the Tauri shell without sending
/// the normal Tauri run events. The desktop sets this variable only for its
/// own spawned child; a standalone `cutd serve` has no watchdog.
#[cfg(target_os = "macos")]
async fn wait_for_desktop_parent_exit() {
    if std::env::var_os("SHELLX_CUT_PARENT_PID").is_none() {
        std::future::pending::<()>().await;
        return;
    }
    loop {
        if unsafe { libc::getppid() } == 1 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

#[cfg(not(target_os = "macos"))]
async fn wait_for_desktop_parent_exit() {
    std::future::pending::<()>().await;
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

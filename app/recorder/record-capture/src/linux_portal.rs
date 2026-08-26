//! Bounded, stop-aware pre-first-frame XDG portal waits.
//!
//! The ScreenCast portal is user-owned, so normal source selection continues to
//! use the portal's existing request flow. These waits only put a finite bound
//! around setup before Cut has received a stream/frame, and let the external
//! `screen_record.stop` flag abandon an unanswered request promptly.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ashpd::desktop::{screencast::Screencast, Session};
use record_core::{error_codes, RecordError, Result};
use tokio::time::Instant;

/// One total budget prevents a sequence of individually bounded portal calls
/// from becoming an effectively unbounded pre-capture handshake.
pub(crate) const PRE_FIRST_FRAME_PORTAL_TIMEOUT: Duration = Duration::from_secs(120);
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(25);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) fn pre_first_frame_deadline() -> Instant {
    Instant::now() + PRE_FIRST_FRAME_PORTAL_TIMEOUT
}

/// Await one portal/stream setup stage until it succeeds, its shared setup
/// budget expires, or the external capture stop is requested.
pub(crate) async fn await_pre_first_frame<T>(
    stage: &str,
    stop: &AtomicBool,
    deadline: Instant,
    wait: impl Future<Output = Result<T>>,
) -> Result<T> {
    if stop.load(Ordering::Relaxed) {
        return Err(stopped(stage));
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(timed_out(stage));
    }

    tokio::select! {
        result = tokio::time::timeout(remaining, wait) => match result {
            Ok(result) => result,
            Err(_) => Err(timed_out(stage)),
        },
        _ = wait_for_stop(stop) => Err(stopped(stage)),
    }
}

/// `Session` does not close on drop. Closing after an interrupted setup removes
/// the portal picker/session before a later capture can ask for another stream.
pub(crate) async fn close_session(session: &Session<Screencast>) {
    match bounded_close(CLOSE_TIMEOUT, session.close()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => eprintln!("warning: portal session close failed (non-fatal): {error}"),
        Err(_) => eprintln!("warning: portal session close timed out (non-fatal)"),
    }
}

async fn bounded_close<E>(
    timeout: Duration,
    close: impl Future<Output = std::result::Result<(), E>>,
) -> std::result::Result<std::result::Result<(), E>, ()> {
    tokio::time::timeout(timeout, close).await.map_err(|_| ())
}

async fn wait_for_stop(stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        tokio::time::sleep(STOP_POLL_INTERVAL).await;
    }
}

fn stopped(stage: &str) -> RecordError {
    RecordError::new(
        error_codes::CAPTURE,
        "screen capture stopped before the first frame",
        format!("screen_record.stop interrupted portal setup at {stage}"),
    )
    .with_action("dismiss the portal picker if it remains visible, then retry recording")
}

fn timed_out(stage: &str) -> RecordError {
    RecordError::new(
        error_codes::CAPTURE,
        "ScreenCast portal did not deliver a stream before the setup deadline",
        format!(
            "portal setup at {stage} exceeded {} seconds before the first frame",
            PRE_FIRST_FRAME_PORTAL_TIMEOUT.as_secs()
        ),
    )
    .with_action("dismiss the portal picker, verify the desktop session, then retry recording")
}

#[cfg(test)]
mod tests {
    use std::future::pending;
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn fake_blocked_portal_wait_stops_with_a_typed_capture_error() {
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            signal.store(true, Ordering::Relaxed);
        });

        let error = await_pre_first_frame(
            "select portal sources",
            &stop,
            Instant::now() + Duration::from_secs(1),
            pending::<Result<()>>(),
        )
        .await
        .expect_err("stop must cancel a blocked portal request");

        assert_eq!(error.code, error_codes::CAPTURE);
        assert_eq!(
            error.message,
            "screen capture stopped before the first frame"
        );
        assert!(error.cause.contains("select portal sources"));
    }

    #[tokio::test]
    async fn successful_portal_wait_preserves_the_stage_result() {
        let stop = AtomicBool::new(false);
        let result = await_pre_first_frame(
            "start portal cast",
            &stop,
            Instant::now() + Duration::from_secs(1),
            async { Ok::<_, RecordError>("granted-stream") },
        )
        .await
        .expect("a granted portal stream must still pass through");

        assert_eq!(result, "granted-stream");
    }

    #[tokio::test]
    async fn portal_session_close_is_bounded_when_the_portal_ignores_it() {
        let result = bounded_close(
            Duration::from_millis(5),
            pending::<std::result::Result<(), RecordError>>(),
        )
        .await;

        assert_eq!(result, Err(()));
    }
}

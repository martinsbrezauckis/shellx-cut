use super::*;

/// Translate a panic payload into a structured error. `todo!()` panics carry
/// "not yet implemented"; other panics remain explicit job failures.
fn error_from_panic(what: &str, payload: Box<dyn std::any::Any + Send>) -> CutError {
    let msg = payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".into());
    if msg.contains("not yet implemented") || msg.contains("not implemented") {
        CutError::new(
            error_codes::UNIMPLEMENTED,
            format!("{what} is not available in this build"),
            msg,
        )
        .with_suggested_action("update ShellX Cut or use one of the supported alternatives")
    } else {
        CutError::new(error_codes::JOB_FAILED, format!("{what} panicked"), msg)
    }
}

/// Catch synchronous dependency panics before they can cross the verb boundary.
pub(crate) fn guard_call<T>(
    what: &str,
    f: impl FnOnce() -> Result<T, CutError>,
) -> Result<T, CutError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(payload) => Err(error_from_panic(what, payload)),
    }
}

/// Run blocking media/perception work away from the async runtime and map task
/// panics or cancellation to the same structured error surface.
pub(crate) async fn run_blocking<T: Send + 'static>(
    what: &str,
    f: impl FnOnce() -> Result<T, CutError> + Send + 'static,
) -> Result<T, CutError> {
    // The guard lives inside the blocking closure. If its awaiting job task is
    // aborted, JobManager still sees the real worker until the closure returns
    // and any synchronous child process has been waited/reaped.
    let worker = crate::jobs::begin_current_blocking_worker();
    match tokio::task::spawn_blocking(move || {
        let _worker = worker;
        f()
    })
    .await
    {
        Ok(result) => result,
        Err(error) if error.is_panic() => Err(error_from_panic(what, error.into_panic())),
        Err(error) => Err(CutError::new(
            error_codes::JOB_FAILED,
            format!("{what} task was cancelled"),
            error.to_string(),
        )),
    }
}

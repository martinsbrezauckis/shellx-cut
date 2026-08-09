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
    // The guard lives inside the blocking closure. If a job requests
    // cancellation, JobManager still sees the real worker until the closure
    // returns and any synchronous child process has been waited/reaped.
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

/// Run a finite foreground helper under the shared owned-process contract.
/// The caller waits synchronously, but a wedged helper cannot retain a process
/// tree or its pipes past `timeout`.
pub(crate) fn run_bounded_foreground_command(
    command: &mut std::process::Command,
    what: &str,
) -> Result<std::process::Output, CutError> {
    run_bounded_foreground_command_with_timeout(
        command,
        std::time::Duration::from_secs(30 * 60),
        what,
    )
}

fn run_bounded_foreground_command_with_timeout(
    command: &mut std::process::Command,
    timeout: std::time::Duration,
    what: &str,
) -> Result<std::process::Output, CutError> {
    let control = cut_media::ffmpeg::OwnedProcessControl::bounded(timeout, || false);
    cut_media::ffmpeg::run_owned_command(command, &control, what)
}

/// Build the operation owner for a job-side Python helper. The caller captures
/// the job signal before entering `spawn_blocking`, then passes this control to
/// every sidecar stage in that operation. Sidecar reports can legitimately be
/// larger than normal tool diagnostics, but retention remains bounded.
pub(crate) fn owned_job_process_control(
    cancellation: crate::jobs::JobCancellation,
) -> cut_media::ffmpeg::OwnedProcessControl {
    let cancellation_probe = cancellation.clone();
    cut_media::ffmpeg::OwnedProcessControl::bounded(
        std::time::Duration::from_secs(30 * 60),
        move || cancellation_probe.is_cancelled(),
    )
    .with_output_cap(8 * 1024 * 1024)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn foreground_owner_times_out_and_reaps_its_child() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 60"]);
        let started = std::time::Instant::now();
        let error = run_bounded_foreground_command_with_timeout(
            &mut command,
            std::time::Duration::from_millis(80),
            "foreground fixture",
        )
        .unwrap_err();
        assert_eq!(error.code, error_codes::FFMPEG);
        assert_eq!(error.message, "operation timed out");
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
}

/// The cancellable variant of [`run_blocking`]. A managed job first flips the
/// supplied signal; the blocking worker can stop and reap a child process
/// before the job manager reports cancellation. Foreground callers receive an
/// inert signal.
pub(crate) async fn run_blocking_cancellable<T: Send + 'static>(
    what: &str,
    f: impl FnOnce(crate::jobs::JobCancellation) -> Result<T, CutError> + Send + 'static,
) -> Result<T, CutError> {
    struct CancelWorkerOnDrop(crate::jobs::JobCancellation, bool);

    impl CancelWorkerOnDrop {
        fn disarm(&mut self) {
            self.1 = false;
        }
    }

    impl Drop for CancelWorkerOnDrop {
        fn drop(&mut self) {
            if self.1 {
                self.0.request_cancel();
            }
        }
    }

    let worker = crate::jobs::begin_current_blocking_worker();
    let cancellation = crate::jobs::current_job_cancellation();
    // If an outer timeout drops this future, request cooperative shutdown before
    // its still-running closure can be detached from the async task.
    let mut cancel_on_drop = CancelWorkerOnDrop(cancellation.clone(), true);
    let joined = tokio::task::spawn_blocking(move || {
        let _worker = worker;
        f(cancellation)
    })
    .await;
    cancel_on_drop.disarm();
    match joined {
        Ok(result) => result,
        Err(error) if error.is_panic() => Err(error_from_panic(what, error.into_panic())),
        Err(error) => Err(CutError::new(
            error_codes::JOB_FAILED,
            format!("{what} task was cancelled"),
            error.to_string(),
        )),
    }
}

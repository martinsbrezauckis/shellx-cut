//! Bounded ownership for ffmpeg processes started by one render operation.
//!
//! The server passes one [`RenderProcessControl`] across the `spawn_blocking`
//! boundary. Every render child shares its deadline and cancellation signal;
//! the owner reaps the direct child and closes its owned tree before a job can
//! report a terminal result.

mod child;
mod pipes;
#[cfg(test)]
mod tests;
mod tree;

use child::{exit_error, spawn_error, ManagedChild};
use cut_core::{error_codes, CutError};
use pipes::{
    finish_progress, finish_reader, finish_writer, read_capped, read_capped_lines, read_progress,
    start_writer, LineObserver,
};
use std::cell::RefCell;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

const PROCESS_POLL: Duration = Duration::from_millis(20);
const STOP_GRACE: Duration = Duration::from_millis(250);
const OUTPUT_CAP_BYTES: usize = 512 * 1024;
const INPUT_CAP_BYTES: usize = 1024 * 1024;
const DEFAULT_FOREGROUND_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// One deadline and cancellation probe shared by every external child in one
/// operation. It lives in `cut-media`, so server jobs, renders, and bounded
/// foreground helpers agree on the ownership boundary without a crate cycle.
#[derive(Clone)]
pub struct RenderProcessControl {
    deadline: Instant,
    cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
    output_cap_bytes: usize,
}

/// Generic name for the same cross-platform owner when a caller is not a
/// final render (for example a foreground perception or archive helper).
pub type OwnedProcessControl = RenderProcessControl;
/// Callback for a bounded child stderr protocol line (for example sidecar
/// progress). The owner still drains and caps retained output itself.
pub type ProcessStderrLineObserver = LineObserver;

impl RenderProcessControl {
    pub fn bounded(
        timeout: Duration,
        cancelled: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            deadline: Instant::now() + timeout,
            cancelled: Arc::new(cancelled),
            output_cap_bytes: OUTPUT_CAP_BYTES,
        }
    }

    /// Keep draining output after this retained-diagnostic cap. Callers with a
    /// structured sidecar response may raise the cap deliberately, but cannot
    /// make it unbounded.
    pub fn with_output_cap(mut self, output_cap_bytes: usize) -> Self {
        self.output_cap_bytes = output_cap_bytes.clamp(1, 8 * 1024 * 1024);
        self
    }

    pub(super) fn check(&self, context: &str) -> Result<(), CutError> {
        if (self.cancelled)() {
            return Err(CutError::new(
                error_codes::RENDER_CANCELLED,
                "render cancelled",
                format!("{context}: the owning job requested cancellation"),
            ));
        }
        if Instant::now() >= self.deadline {
            return Err(CutError::new(
                error_codes::FFMPEG,
                "operation timed out",
                format!("{context}: the operation-wide deadline elapsed"),
            )
            .with_suggested_action("reduce the render scope or retry"));
        }
        Ok(())
    }
}

thread_local! {
    static CURRENT_CONTROL: RefCell<Option<RenderProcessControl>> = const { RefCell::new(None) };
}

/// Run an operation with process ownership. Direct library callers preserve
/// their established foreground behavior unless they opt into this boundary.
pub fn with_render_process_control<T>(
    control: &RenderProcessControl,
    run: impl FnOnce() -> T,
) -> T {
    let restore = scoped_render_process_control(Some(control));
    let value = run();
    drop(restore);
    value
}

pub(crate) struct RenderProcessControlScope(Option<RenderProcessControl>);

impl Drop for RenderProcessControlScope {
    fn drop(&mut self) {
        CURRENT_CONTROL.with(|slot| {
            slot.replace(self.0.take());
        });
    }
}

pub(crate) fn scoped_render_process_control(
    control: Option<&RenderProcessControl>,
) -> RenderProcessControlScope {
    RenderProcessControlScope(CURRENT_CONTROL.with(|slot| slot.replace(control.cloned())))
}

pub(crate) fn current_render_process_control() -> Option<RenderProcessControl> {
    CURRENT_CONTROL.with(|slot| slot.borrow().clone())
}

pub(crate) fn command_output_with_current_control(
    command: &mut Command,
    context: &str,
) -> Result<Output, CutError> {
    match current_render_process_control() {
        Some(control) => command_output_with_control(command, &control, context),
        None => run_default_bounded_command(command, context),
    }
}

pub(crate) fn command_output_with_control(
    command: &mut Command,
    control: &RenderProcessControl,
    context: &str,
) -> Result<Output, CutError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ManagedChild::spawn(command, control.clone(), context)?;
    let stdout = read_capped(child.take_stdout(context)?, control.output_cap_bytes);
    let stderr = read_capped(child.take_stderr(context)?, control.output_cap_bytes);
    let status = child.wait(context);
    let stdout = finish_reader(stdout, &mut child, context);
    let stderr = finish_reader(stderr, &mut child, context);
    match (status, stdout, stderr) {
        (Ok(status), Ok(stdout), Ok(stderr)) => Ok(Output {
            status,
            stdout,
            stderr,
        }),
        (Err(error), _, _) => Err(error),
        (_, Err(error), _) | (_, _, Err(error)) => Err(spawn_error(context, error)),
    }
}

/// Own one finite foreground command with the caller's deadline. This uses the
/// same process-group/Job-Object claim, capped pipes, and final wait/reap path
/// as a render; callers that need an operation-wide budget reuse one control.
pub fn run_owned_command(
    command: &mut Command,
    control: &OwnedProcessControl,
    context: &str,
) -> Result<Output, CutError> {
    command_output_with_control(command, control, context)
}

/// Own a finite command that consumes one bounded stdin request. The request
/// write is moved off the calling thread so a child that never reads stdin
/// cannot bypass cancellation or the operation deadline. Its writer, direct
/// child, pipes, and descendants all complete before this function returns.
///
/// The optional line observer receives stderr lines while the command is
/// running. It is intended for bounded progress protocol lines, not for
/// retaining diagnostics; output retention remains capped by `control`.
pub fn run_owned_command_with_input(
    command: &mut Command,
    input: &[u8],
    control: &OwnedProcessControl,
    context: &str,
    stderr_line: Option<ProcessStderrLineObserver>,
) -> Result<Output, CutError> {
    if input.len() > INPUT_CAP_BYTES {
        return Err(CutError::new(
            error_codes::FFMPEG,
            "owned command stdin request is too large",
            format!(
                "{context}: {} bytes exceeds {INPUT_CAP_BYTES} byte limit",
                input.len()
            ),
        ));
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ManagedChild::spawn(command, control.clone(), context)?;
    let writer = start_writer(child.take_stdin(context)?, input.to_vec());
    let stdout = read_capped(child.take_stdout(context)?, control.output_cap_bytes);
    let stderr = read_capped_lines(
        child.take_stderr(context)?,
        control.output_cap_bytes,
        stderr_line,
    );
    let status = child.wait(context);
    let writer = finish_writer(writer, &mut child, context);
    let stdout = finish_reader(stdout, &mut child, context);
    let stderr = finish_reader(stderr, &mut child, context);
    match (status, writer, stdout, stderr) {
        (Ok(status), Ok(()), Ok(stdout), Ok(stderr)) => Ok(Output {
            status,
            stdout,
            stderr,
        }),
        (Err(error), _, _, _) => Err(error),
        (_, Err(error), _, _) | (_, _, Err(error), _) | (_, _, _, Err(error)) => {
            Err(spawn_error(context, error))
        }
    }
}

/// Run one finite foreground command under a bounded, owned process tree.
///
/// This is the safe default for media probes and transformations that are not
/// part of a server job. Job callers should pass their render-wide control via
/// [`run_owned_command`] instead, so sibling children share one deadline and
/// cancellation signal.
pub fn run_bounded_command(command: &mut Command, context: &str) -> Result<Output, CutError> {
    command_output_with_current_control(command, context)
}

fn run_default_bounded_command(command: &mut Command, context: &str) -> Result<Output, CutError> {
    let control = OwnedProcessControl::bounded(DEFAULT_FOREGROUND_TIMEOUT, || false);
    run_owned_command(command, &control, context)
}

pub(crate) fn drive_with_current_control(
    command: &mut Command,
    total_ms: u64,
    on_progress: &dyn Fn(f32),
) -> Result<(), CutError> {
    match current_render_process_control() {
        Some(control) => drive_with_control(command, total_ms, on_progress, &control),
        None => {
            let control = RenderProcessControl::bounded(DEFAULT_FOREGROUND_TIMEOUT, || false);
            drive_with_control(command, total_ms, on_progress, &control)
        }
    }
}

fn drive_with_control(
    command: &mut Command,
    total_ms: u64,
    on_progress: &dyn Fn(f32),
    control: &RenderProcessControl,
) -> Result<(), CutError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ManagedChild::spawn(command, control.clone(), "ffmpeg render")?;
    let progress = read_progress(child.take_stdout("ffmpeg render")?);
    let stderr = read_capped(
        child.take_stderr("ffmpeg render")?,
        control.output_cap_bytes,
    );
    on_progress(0.0);
    let status = loop {
        if let Err(error) = control.check("ffmpeg render") {
            let mut error = error;
            if let Err(cleanup) = child.stop_and_reap("ffmpeg render") {
                error.cause = format!("{}; cleanup failed: {cleanup}", error.cause);
            }
            break Err(error);
        }
        match child.try_wait("ffmpeg render")? {
            Some(status) => break Ok(status),
            None => match progress.done.recv_timeout(PROCESS_POLL) {
                Ok(Some(us)) if total_ms > 0 => {
                    on_progress(((us / 1000) as f32 / total_ms as f32).clamp(0.0, 1.0));
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break Ok(child.wait("ffmpeg render")?),
            },
        }
    };
    let progress_error = finish_progress(progress, &mut child, total_ms, on_progress);
    let stderr = finish_reader(stderr, &mut child, "ffmpeg render");
    match (status, progress_error, stderr) {
        (Ok(status), Ok(()), Ok(_stderr)) if status.success() => {
            // FFmpeg's final out_time may stop one encoded-frame shy of the
            // requested timeline duration. A successful owned process is the
            // authoritative completion boundary for the progress contract.
            on_progress(1.0);
            Ok(())
        }
        (Ok(status), Ok(()), Ok(stderr)) => Err(exit_error(status, &stderr)),
        (Err(error), _, _) => Err(error),
        (_, Err(error), _) | (_, _, Err(error)) => Err(spawn_error("ffmpeg render", error)),
    }
}

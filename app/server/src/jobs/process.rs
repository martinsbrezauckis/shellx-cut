//! Bounded ownership for external workers started by a managed job.
//!
//! `tokio::process::Child::kill` only addresses the direct process.  This
//! module owns the entire worker lifetime: it caps diagnostic pipes, observes
//! the job cancellation reason and a caller-owned operation deadline, stops a
//! process tree, then waits for the direct child before returning.

use super::{
    begin_current_process_worker, current_job_cancellation, JobCancellation, JobCancellationReason,
};
use std::fmt;
use std::io;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

mod cleanup;
mod tree;
use tree::ProcessTree;

const PROCESS_POLL: Duration = Duration::from_millis(20);
const GRACEFUL_STOP: Duration = Duration::from_millis(250);
const OUTPUT_CAP_BYTES: usize = 512 * 1024;

/// Why an owned process was stopped before it could exit normally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessTermination {
    Cancelled(JobCancellationReason),
    DeadlineExceeded,
}

/// Error returned after an owned child has definitely been waited/reaped.
#[derive(Debug)]
pub(crate) struct ManagedProcessError {
    termination: Option<ProcessTermination>,
    io_kind: Option<io::ErrorKind>,
    detail: String,
}

impl ManagedProcessError {
    fn io(context: &str, error: io::Error) -> Self {
        Self {
            termination: None,
            io_kind: Some(error.kind()),
            detail: format!("{context}: {error}"),
        }
    }

    fn terminated(termination: ProcessTermination) -> Self {
        let detail = match termination {
            ProcessTermination::Cancelled(reason) => {
                format!("external worker cancelled ({})", reason.label())
            }
            ProcessTermination::DeadlineExceeded => "external worker timed out".to_string(),
        };
        Self {
            termination: Some(termination),
            io_kind: None,
            detail,
        }
    }

    pub(crate) fn termination(&self) -> Option<ProcessTermination> {
        self.termination
    }

    pub(crate) fn io_kind(&self) -> Option<io::ErrorKind> {
        self.io_kind
    }

    fn with_cleanup(mut self, cleanup: impl fmt::Display) -> Self {
        self.detail.push_str("; cleanup: ");
        self.detail.push_str(&cleanup.to_string());
        self
    }
}

impl fmt::Display for ManagedProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for ManagedProcessError {}

/// One deadline shared across every child in an operation, plus the job's
/// cooperative cancellation signal.  Create once and pass it to every stage
/// of a multi-process operation; do not reset the timeout for each child.
#[derive(Clone)]
pub(crate) struct ProcessControl {
    deadline: tokio::time::Instant,
    cancellation: JobCancellation,
    output_cap_bytes: usize,
}

impl ProcessControl {
    pub(crate) fn for_operation(timeout: Duration) -> Self {
        Self {
            deadline: tokio::time::Instant::now() + timeout,
            cancellation: current_job_cancellation(),
            output_cap_bytes: OUTPUT_CAP_BYTES,
        }
    }

    pub(crate) fn with_output_cap(mut self, output_cap_bytes: usize) -> Self {
        self.output_cap_bytes = output_cap_bytes.max(1);
        self
    }

    #[cfg(test)]
    fn with_cancellation(timeout: Duration, cancellation: JobCancellation) -> Self {
        Self {
            deadline: tokio::time::Instant::now() + timeout,
            cancellation,
            output_cap_bytes: OUTPUT_CAP_BYTES,
        }
    }

    fn termination(&self) -> Option<ProcessTermination> {
        self.cancellation
            .reason()
            .map(ProcessTermination::Cancelled)
            .or_else(|| {
                (tokio::time::Instant::now() >= self.deadline)
                    .then_some(ProcessTermination::DeadlineExceeded)
            })
    }
}

/// Captured output.  The pipes are always drained; only retained diagnostics
/// are capped so an untrusted worker cannot consume unbounded server memory.
#[derive(Debug)]
pub(crate) struct ManagedOutput {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) stdout_truncated: bool,
    pub(crate) stderr_truncated: bool,
}

impl ManagedOutput {
    pub(crate) fn diagnostics_truncated(&self) -> bool {
        self.stdout_truncated || self.stderr_truncated
    }
}

/// Spawn, optionally write stdin, and own the child until it is reaped.
///
/// On Unix a new process group contains descendants.  On Windows each child
/// is assigned to a kill-on-close Job Object; inability to establish that
/// ownership fails closed after killing and reaping the direct child.
pub(crate) async fn run_owned(
    command: &mut Command,
    stdin: Option<&[u8]>,
    control: &ProcessControl,
) -> Result<ManagedOutput, ManagedProcessError> {
    let _worker = begin_current_process_worker();
    if let Some(termination) = control.termination() {
        return Err(ManagedProcessError::terminated(termination));
    }
    tree::configure(command).map_err(|error| ManagedProcessError::io("configure worker", error))?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Drop is not a substitute for ownership.  We explicitly stop and
        // wait below, including descendants.
        .kill_on_drop(false);
    let mut child = command
        .spawn()
        .map_err(|error| ManagedProcessError::io("start external worker", error))?;
    let mut tree = match ProcessTree::establish(&child) {
        Ok(tree) => tree,
        Err(error) => {
            let mut claimed = ManagedProcessError::io("claim external worker", error);
            if let Err(cleanup) = cleanup::direct_kill_and_wait(&mut child).await {
                claimed = claimed.with_cleanup(cleanup);
            }
            return Err(claimed);
        }
    };
    let stdout = child.stdout.take().expect("owned stdout is piped");
    let stderr = child.stderr.take().expect("owned stderr is piped");
    let mut stdout_reader = tokio::spawn(read_capped(stdout, control.output_cap_bytes));
    let mut stderr_reader = tokio::spawn(read_capped(stderr, control.output_cap_bytes));

    if let Some(input) = stdin {
        let input_result = if let Some(mut pipe) = child.stdin.take() {
            write_input(&mut pipe, input, control).await
        } else {
            Err(ManagedProcessError::io(
                "write external worker stdin",
                io::Error::new(io::ErrorKind::BrokenPipe, "stdin pipe unavailable"),
            ))
        };
        if let Err(error) = input_result {
            return Err(cleanup::cleanup_error(
                error,
                &mut child,
                &mut tree,
                &mut stdout_reader,
                &mut stderr_reader,
            )
            .await);
        }
    } else {
        drop(child.stdin.take());
    }

    let status = loop {
        if let Some(termination) = control.termination() {
            return Err(cleanup::cleanup_error(
                ManagedProcessError::terminated(termination),
                &mut child,
                &mut tree,
                &mut stdout_reader,
                &mut stderr_reader,
            )
            .await);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => tokio::time::sleep(PROCESS_POLL).await,
            Err(error) => {
                return Err(cleanup::cleanup_error(
                    ManagedProcessError::io("wait for external worker", error),
                    &mut child,
                    &mut tree,
                    &mut stdout_reader,
                    &mut stderr_reader,
                )
                .await);
            }
        }
    };
    // A successful leader must not silently leave a helper behind. The worker
    // group/Job is operation-owned, not a detached service lifecycle.
    if let Err(error) = tree.hard_stop() {
        let mut failure = ManagedProcessError::io("close external worker tree", error);
        if let Err(readers) =
            cleanup::abort_and_join_readers(&mut stdout_reader, &mut stderr_reader).await
        {
            failure = failure.with_cleanup(readers);
        }
        return Err(failure);
    }
    let ((stdout, stdout_truncated), (stderr, stderr_truncated)) =
        cleanup::finish_readers(&mut tree, &mut stdout_reader, &mut stderr_reader)
            .await
            .map_err(|error| ManagedProcessError::io("read external worker output", error))?;
    Ok(ManagedOutput {
        status,
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
    })
}

async fn write_input(
    pipe: &mut tokio::process::ChildStdin,
    input: &[u8],
    control: &ProcessControl,
) -> Result<(), ManagedProcessError> {
    let write = async {
        pipe.write_all(input).await?;
        pipe.shutdown().await
    };
    tokio::pin!(write);
    loop {
        if let Some(termination) = control.termination() {
            return Err(ManagedProcessError::terminated(termination));
        }
        tokio::select! {
            result = &mut write => {
                return result.map_err(|error| ManagedProcessError::io("write external worker stdin", error));
            }
            _ = tokio::time::sleep(PROCESS_POLL) => {}
        }
    }
}

async fn read_capped<R: AsyncRead + Unpin>(
    mut reader: R,
    output_cap_bytes: usize,
) -> io::Result<(Vec<u8>, bool)> {
    let mut retained = Vec::new();
    let mut truncated = false;
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            return Ok((retained, truncated));
        }
        let available = output_cap_bytes.saturating_sub(retained.len());
        let keep = available.min(count);
        retained.extend_from_slice(&buffer[..keep]);
        truncated |= keep != count;
    }
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;

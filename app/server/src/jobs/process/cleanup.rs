use super::{ManagedProcessError, ProcessTree, GRACEFUL_STOP};
use std::io;
use std::process::ExitStatus;
use tokio::process::Child;

pub(super) type Reader = tokio::task::JoinHandle<io::Result<(Vec<u8>, bool)>>;

pub(super) async fn cleanup_error(
    error: ManagedProcessError,
    child: &mut Child,
    tree: &mut ProcessTree,
    stdout: &mut Reader,
    stderr: &mut Reader,
) -> ManagedProcessError {
    match stop_and_drain(child, tree, stdout, stderr).await {
        Ok(()) => error,
        Err(cleanup) => error.with_cleanup(cleanup),
    }
}

pub(super) async fn finish_readers(
    tree: &mut ProcessTree,
    stdout: &mut Reader,
    stderr: &mut Reader,
) -> io::Result<((Vec<u8>, bool), (Vec<u8>, bool))> {
    match tokio::time::timeout(GRACEFUL_STOP, join_readers(stdout, stderr)).await {
        Ok(result) => result,
        Err(_) => {
            // A leader can exit while an inherited pipe remains open in a
            // descendant. Stop the owned tree before a reader can hang forever.
            if let Err(tree_error) = tree.hard_stop() {
                let readers = abort_and_join_readers(stdout, stderr).await;
                return match readers {
                    Ok(()) => Err(tree_error),
                    Err(read_error) => Err(io::Error::other(format!(
                        "hard-stop owned worker tree failed: {tree_error}; reader cleanup failed: {read_error}"
                    ))),
                };
            }
            match tokio::time::timeout(GRACEFUL_STOP, join_readers(stdout, stderr)).await {
                Ok(result) => result,
                Err(_) => {
                    let readers = abort_and_join_readers(stdout, stderr).await;
                    let detail =
                        "owned worker descendants kept output pipes open after termination";
                    if let Err(error) = readers {
                        return Err(io::Error::other(format!(
                            "{detail}; reader cleanup failed: {error}"
                        )));
                    }
                    Err(io::Error::new(io::ErrorKind::TimedOut, detail))
                }
            }
        }
    }
}

/// Abort both cancellable pipe readers and wait until neither task remains.
/// Returning before this join would detach a reader that can still retain a
/// pipe/worker reference after the process owner has reported its outcome.
pub(super) async fn abort_and_join_readers(
    stdout: &mut Reader,
    stderr: &mut Reader,
) -> io::Result<()> {
    stdout.abort();
    stderr.abort();
    join_aborted_reader(stdout).await?;
    join_aborted_reader(stderr).await
}

pub(super) async fn stop_and_wait(
    child: &mut Child,
    tree: &mut ProcessTree,
) -> io::Result<ExitStatus> {
    // Closing stdin is the cross-platform graceful request used by JSON workers.
    drop(child.stdin.take());
    let _ = tokio::time::timeout(GRACEFUL_STOP, child.wait()).await;
    let soft_error = tree.soft_stop().err();
    let _ = tokio::time::timeout(GRACEFUL_STOP, child.wait()).await;
    let hard_error = tree.hard_stop().err();
    let direct_error = child
        .start_kill()
        .err()
        .filter(|error| error.kind() != io::ErrorKind::InvalidInput);
    let waited = child.wait().await;
    match (waited, soft_error, hard_error, direct_error) {
        (Ok(status), None, None, None) => Ok(status),
        (Ok(_), soft, hard, direct) => Err(cleanup_failures(soft, hard, direct, None)),
        (Err(wait), soft, hard, direct) => Err(cleanup_failures(soft, hard, direct, Some(wait))),
    }
}

pub(super) async fn direct_kill_and_wait(child: &mut Child) -> io::Result<ExitStatus> {
    let kill = child.start_kill().err();
    let waited = child.wait().await;
    match (waited, kill) {
        (Ok(status), None) => Ok(status),
        (Ok(_), Some(kill)) => Err(io::Error::other(format!(
            "direct child kill failed: {kill}"
        ))),
        (Err(wait), _) => Err(wait),
    }
}

async fn stop_and_drain(
    child: &mut Child,
    tree: &mut ProcessTree,
    stdout: &mut Reader,
    stderr: &mut Reader,
) -> io::Result<()> {
    let stopped = stop_and_wait(child, tree).await;
    let drained = finish_readers(tree, stdout, stderr).await;
    match (stopped, drained) {
        (Ok(_), Ok(_)) => Ok(()),
        (Err(stop), Ok(_)) => Err(stop),
        (Ok(_), Err(drain)) => Err(drain),
        (Err(stop), Err(drain)) => Err(io::Error::other(format!(
            "worker stop failed: {stop}; pipe drain failed: {drain}"
        ))),
    }
}

async fn join_readers(
    stdout: &mut Reader,
    stderr: &mut Reader,
) -> io::Result<((Vec<u8>, bool), (Vec<u8>, bool))> {
    Ok((join_reader(stdout).await?, join_reader(stderr).await?))
}

async fn join_reader(reader: &mut Reader) -> io::Result<(Vec<u8>, bool)> {
    reader
        .await
        .map_err(|error| io::Error::other(format!("pipe reader task failed: {error}")))?
}

async fn join_aborted_reader(reader: &mut Reader) -> io::Result<()> {
    match reader.await {
        Ok(_) => Ok(()),
        Err(error) if error.is_cancelled() => Ok(()),
        Err(error) => Err(io::Error::other(format!(
            "pipe reader task failed during abort: {error}"
        ))),
    }
}

fn cleanup_failures(
    soft: Option<io::Error>,
    hard: Option<io::Error>,
    direct: Option<io::Error>,
    wait: Option<io::Error>,
) -> io::Error {
    let mut parts = Vec::new();
    for (label, error) in [
        ("graceful tree stop", soft),
        ("hard tree stop", hard),
        ("direct child kill", direct),
        ("direct child wait", wait),
    ] {
        if let Some(error) = error {
            parts.push(format!("{label} failed: {error}"));
        }
    }
    io::Error::other(parts.join("; "))
}

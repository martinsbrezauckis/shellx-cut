//! Bounded stdin/stdout/stderr workers for an owned media child.

use super::child::ManagedChild;
use super::{OUTPUT_CAP_BYTES, PIPE_CLOSE_GRACE, STOP_GRACE};
use std::io::{self, Read, Write};
use std::process::ChildStdin;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;

pub type LineObserver = Arc<dyn Fn(&str) + Send + Sync>;

pub(super) struct Reader {
    done: Receiver<io::Result<Vec<u8>>>,
    handle: Option<JoinHandle<()>>,
}

pub(super) struct Writer {
    done: Receiver<io::Result<()>>,
    handle: Option<JoinHandle<()>>,
}

/// Write the request away from the operation thread. A child that does not
/// read its pipe can block this worker, but cancellation/deadline cleanup
/// closes the owned tree and unblocks it before `finish_writer` returns.
pub(super) fn start_writer(mut stdin: ChildStdin, input: Vec<u8>) -> Writer {
    let (tx, done) = mpsc::sync_channel(1);
    let handle = thread::spawn(move || {
        let result = match stdin.write_all(&input).and_then(|_| stdin.flush()) {
            // The child closed its input without reading all of it (it exited
            // early, e.g. a stand-in interpreter that only prints a hint). That
            // is not a failure to run it: its exit status and output say what
            // happened, and the caller reports those.
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
            result => result,
        };
        drop(stdin);
        let _ = tx.send(result);
    });
    Writer {
        done,
        handle: Some(handle),
    }
}

pub(super) fn read_capped<R: Read + Send + 'static>(
    mut input: R,
    output_cap_bytes: usize,
) -> Reader {
    let (tx, done) = mpsc::sync_channel(1);
    let handle = thread::spawn(move || {
        let mut kept = Vec::with_capacity(output_cap_bytes);
        let mut buf = [0u8; 8192];
        let result = loop {
            match input.read(&mut buf) {
                Ok(0) => break Ok(kept),
                Ok(n) => {
                    let room = output_cap_bytes.saturating_sub(kept.len());
                    kept.extend_from_slice(&buf[..n.min(room)]);
                }
                Err(error) => break Err(error),
            }
        };
        let _ = tx.send(result);
    });
    Reader {
        done,
        handle: Some(handle),
    }
}

/// Drain stderr while forwarding complete, bounded lines to a progress
/// observer. This preserves pipe backpressure safety even when a sidecar emits
/// progress faster than its consumer can retain diagnostics.
pub(super) fn read_capped_lines<R: Read + Send + 'static>(
    mut input: R,
    output_cap_bytes: usize,
    observer: Option<LineObserver>,
) -> Reader {
    const LINE_CAP_BYTES: usize = 16 * 1024;
    let (tx, done) = mpsc::sync_channel(1);
    let handle = thread::spawn(move || {
        let mut kept = Vec::with_capacity(output_cap_bytes);
        let mut line = Vec::with_capacity(256);
        let mut buf = [0u8; 8192];
        let result = loop {
            match input.read(&mut buf) {
                Ok(0) => {
                    if !line.is_empty() {
                        if let Some(observer) = &observer {
                            observer(&String::from_utf8_lossy(&line));
                        }
                    }
                    break Ok(kept);
                }
                Ok(n) => {
                    let room = output_cap_bytes.saturating_sub(kept.len());
                    kept.extend_from_slice(&buf[..n.min(room)]);
                    if let Some(observer) = &observer {
                        for byte in &buf[..n] {
                            if *byte == b'\n' {
                                observer(&String::from_utf8_lossy(&line));
                                line.clear();
                            } else if line.len() < LINE_CAP_BYTES {
                                line.push(*byte);
                            }
                        }
                    }
                }
                Err(error) => break Err(error),
            }
        };
        let _ = tx.send(result);
    });
    Reader {
        done,
        handle: Some(handle),
    }
}

pub(super) fn finish_reader(
    mut reader: Reader,
    child: &mut ManagedChild,
    context: &str,
) -> io::Result<Vec<u8>> {
    finish_worker(&mut reader.done, &mut reader.handle, child, context, "pipe")
}

pub(super) fn finish_writer(
    mut writer: Writer,
    child: &mut ManagedChild,
    context: &str,
) -> io::Result<()> {
    finish_worker(
        &mut writer.done,
        &mut writer.handle,
        child,
        context,
        "stdin writer",
    )
}

fn finish_worker<T>(
    done: &mut Receiver<io::Result<T>>,
    handle: &mut Option<JoinHandle<()>>,
    child: &mut ManagedChild,
    context: &str,
    worker: &str,
) -> io::Result<T> {
    let (result, timed_out, cleanup) = match done.recv_timeout(STOP_GRACE) {
        Ok(result) => (Some(result), false, None),
        Err(_) => {
            let cleanup = child.stop_and_reap(context).err();
            let grace = if cleanup.is_none() {
                PIPE_CLOSE_GRACE
            } else {
                STOP_GRACE
            };
            (done.recv_timeout(grace).ok(), true, cleanup)
        }
    };
    if result.is_some() {
        join_completed_worker(handle, context, worker)?;
    }
    if let Some(cleanup) = cleanup {
        return Err(io::Error::other(format!(
            "{context}: {worker} cleanup failed: {cleanup}"
        )));
    }
    match result {
        Some(result) => result,
        None if timed_out => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("{context}: owned ffmpeg {worker} did not close"),
        )),
        None => Err(io::Error::other(format!(
            "{context}: owned ffmpeg {worker} disconnected"
        ))),
    }
}

// Join only after the worker completion channel has closed or yielded its result.
// JoinHandle::join has no timeout, so joining before that proof could make a
// cancelled job wait forever on an unread inherited pipe.
fn join_completed_worker(
    handle: &mut Option<JoinHandle<()>>,
    context: &str,
    worker: &str,
) -> io::Result<()> {
    if let Some(handle) = handle.take() {
        handle.join().map_err(|_| {
            io::Error::other(format!("{context}: owned ffmpeg {worker} thread panicked"))
        })?;
    }
    Ok(())
}

pub(super) struct ProgressReader {
    pub(super) done: Receiver<Option<u64>>,
    handle: Option<JoinHandle<()>>,
}

pub(super) fn read_progress<R: Read + Send + 'static>(mut input: R) -> ProgressReader {
    let (tx, rx) = mpsc::sync_channel(32);
    let handle = thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut line = Vec::new();
        while let Ok(n) = input.read(&mut buf) {
            if n == 0 {
                break;
            }
            for byte in &buf[..n] {
                if *byte == b'\n' {
                    // A line may end in CR (a Windows writer): not part of the value.
                    let value = std::str::from_utf8(&line)
                        .ok()
                        .map(|line| line.trim_end_matches('\r'))
                        .and_then(|line| line.strip_prefix("out_time_us="))
                        .and_then(|value| value.parse().ok());
                    let _ = tx.try_send(value);
                    line.clear();
                } else if line.len() < OUTPUT_CAP_BYTES {
                    line.push(*byte);
                }
            }
        }
    });
    ProgressReader {
        done: rx,
        handle: Some(handle),
    }
}

pub(super) fn finish_progress(
    mut progress: ProgressReader,
    child: &mut ManagedChild,
    total_ms: u64,
    on_progress: &dyn Fn(f32),
) -> io::Result<()> {
    let mut cleanup = None;
    let mut close_deadline = None;
    loop {
        let wait = close_deadline
            .map(|deadline: Instant| deadline.saturating_duration_since(Instant::now()))
            .unwrap_or(STOP_GRACE);
        let received = if wait.is_zero() {
            Err(mpsc::RecvTimeoutError::Timeout)
        } else {
            progress.done.recv_timeout(wait)
        };
        match received {
            Ok(Some(us)) if total_ms > 0 => {
                on_progress(((us / 1000) as f32 / total_ms as f32).clamp(0.0, 1.0))
            }
            Ok(_) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                join_completed_worker(&mut progress.handle, "ffmpeg progress", "reader")?;
                return cleanup.map_or(Ok(()), Err);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if close_deadline.is_none() {
                    cleanup = child.stop_and_reap("ffmpeg progress").err();
                    let grace = if cleanup.is_none() {
                        PIPE_CLOSE_GRACE
                    } else {
                        STOP_GRACE
                    };
                    close_deadline = Some(Instant::now() + grace);
                } else {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "ffmpeg progress: owned reader did not close after process cleanup",
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "pipe_tests.rs"]
mod pipe_tests;

#[cfg(all(test, unix))]
mod early_exit_tests {
    /// A child that exits without reading its input is reported with its own
    /// exit status and output, not as a broken pipe (the input is larger than
    /// a pipe buffer, so the write always meets the closed pipe).
    #[test]
    fn a_child_that_exits_before_reading_its_input_keeps_its_status() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "echo 'not installed' >&2; exit 3"]);
        let control =
            super::super::OwnedProcessControl::bounded(std::time::Duration::from_secs(30), || {
                false
            });
        let input = vec![b'x'; 512 * 1024];
        let output = super::super::run_owned_command_with_input(
            &mut command,
            &input,
            &control,
            "early exit",
            None,
        )
        .expect("the child ran; its status is the result");
        assert_eq!(output.status.code(), Some(3));
        assert!(String::from_utf8_lossy(&output.stderr).contains("not installed"));
    }
}

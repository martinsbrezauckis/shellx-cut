//! Bounded stdin/stdout/stderr workers for an owned media child.

use super::child::ManagedChild;
use super::{OUTPUT_CAP_BYTES, STOP_GRACE};
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
        let result = stdin.write_all(&input).and_then(|_| stdin.flush());
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
            (done.recv_timeout(STOP_GRACE).ok(), true, cleanup)
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
                    let value = std::str::from_utf8(&line)
                        .ok()
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
        match progress.done.recv_timeout(wait) {
            Ok(Some(us)) if total_ms > 0 => {
                on_progress(((us / 1000) as f32 / total_ms as f32).clamp(0.0, 1.0))
            }
            Ok(_) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                join_completed_worker(&mut progress.handle, "ffmpeg progress", "reader")?;
                return cleanup.map_or(Ok(()), Err);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if cleanup.is_none() {
                    cleanup = child.stop_and_reap("ffmpeg progress").err();
                    close_deadline = Some(Instant::now() + STOP_GRACE);
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

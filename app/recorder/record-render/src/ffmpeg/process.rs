//! Bounded, cancellable Screen Record ffmpeg process ownership.

mod process_tree;

use std::io::{self, Read};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use record_core::{error_codes, RecordError, Result};

use super::ff_err;

const PROCESS_POLL: Duration = Duration::from_millis(20);
const STOP_GRACE: Duration = Duration::from_millis(250);
pub(super) const OUTPUT_CAP_BYTES: usize = 512 * 1024;

/// One deadline shared by every child in a recording export, plus its job's
/// cooperative cancellation probe.
#[derive(Clone)]
pub struct ProcessControl {
    deadline: Instant,
    cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl ProcessControl {
    pub fn bounded(
        timeout: Duration,
        cancelled: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            deadline: Instant::now() + timeout,
            cancelled: Arc::new(cancelled),
        }
    }

    pub fn check(&self, context: &str) -> Result<()> {
        if (self.cancelled)() {
            return Err(RecordError::new(
                "render_cancelled",
                "screen-record render cancelled",
                context,
            ));
        }
        if Instant::now() >= self.deadline {
            return Err(RecordError::new(
                error_codes::FFMPEG,
                "screen-record render timed out",
                context,
            )
            .with_action("reduce the recording length or retry the export"));
        }
        Ok(())
    }
}

/// Owns the leader and its process group/Job Object until the leader is reaped.
pub(super) struct ManagedChild {
    child: Arc<Mutex<Child>>,
    tree: Arc<Mutex<process_tree::ProcessTree>>,
    stopped: Arc<AtomicBool>,
    watcher: Option<JoinHandle<()>>,
    control: ProcessControl,
    context: &'static str,
    reaped: bool,
    tree_closed: bool,
}

impl ManagedChild {
    pub(super) fn spawn(
        command: &mut Command,
        control: ProcessControl,
        context: &'static str,
    ) -> Result<Self> {
        control.check(context)?;
        process_tree::configure(command)
            .map_err(|error| ff_err("configure ffmpeg owner", error))?;
        let mut direct = command.spawn().map_err(|error| ff_err(context, error))?;
        let process_tree = match process_tree::ProcessTree::establish(&direct) {
            Ok(tree) => tree,
            Err(error) => {
                let _ = direct.kill();
                let _ = direct.wait();
                return Err(ff_err("claim ffmpeg process tree", error));
            }
        };
        let child = Arc::new(Mutex::new(direct));
        let tree = Arc::new(Mutex::new(process_tree));
        let stopped = Arc::new(AtomicBool::new(false));
        let watcher = spawn_watcher(
            child.clone(),
            tree.clone(),
            stopped.clone(),
            control.clone(),
            context,
        );
        Ok(Self {
            child,
            tree,
            stopped,
            watcher: Some(watcher),
            control,
            context,
            reaped: false,
            tree_closed: false,
        })
    }

    pub(super) fn take_stdin(&self) -> Result<ChildStdin> {
        self.child
            .lock()
            .expect("ffmpeg child lock")
            .stdin
            .take()
            .ok_or_else(|| ff_err(self.context, "ffmpeg stdin was not piped"))
    }

    pub(super) fn take_stdout(&self) -> Result<ChildStdout> {
        self.child
            .lock()
            .expect("ffmpeg child lock")
            .stdout
            .take()
            .ok_or_else(|| ff_err(self.context, "ffmpeg stdout was not piped"))
    }

    pub(super) fn take_stderr(&self) -> Result<ChildStderr> {
        self.child
            .lock()
            .expect("ffmpeg child lock")
            .stderr
            .take()
            .ok_or_else(|| ff_err(self.context, "ffmpeg stderr was not piped"))
    }

    pub(super) fn wait(&mut self) -> Result<ExitStatus> {
        loop {
            if let Err(mut error) = self.control.check(self.context) {
                if let Err(cleanup) = self.stop_and_reap() {
                    error.cause = format!("{}; cleanup: {}", error.cause, cleanup.cause);
                }
                return Err(error);
            }
            let status = self
                .child
                .lock()
                .expect("ffmpeg child lock")
                .try_wait()
                .map_err(|error| ff_err(self.context, error))?;
            if let Some(status) = status {
                self.reaped = true;
                let tree_result = match self.tree.lock().expect("ffmpeg tree lock").hard_stop() {
                    Ok(()) => {
                        self.tree_closed = true;
                        Ok(())
                    }
                    Err(error) => Err(ff_err("close ffmpeg process tree", error)),
                };
                self.stop_watcher();
                tree_result?;
                return Ok(status);
            }
            thread::sleep(PROCESS_POLL);
        }
    }

    pub(super) fn kill_and_reap(&mut self) {
        let _ = self.stop_and_reap();
    }

    pub(super) fn stop_and_reap(&mut self) -> Result<()> {
        if self.reaped && self.tree_closed {
            return Ok(());
        }
        let soft = self
            .tree
            .lock()
            .expect("ffmpeg tree lock")
            .soft_stop()
            .err();
        let _ = self.wait_for_grace();
        let hard = self
            .tree
            .lock()
            .expect("ffmpeg tree lock")
            .hard_stop()
            .err();
        self.tree_closed = hard.is_none();
        let mut child = self.child.lock().expect("ffmpeg child lock");
        let kill = child
            .kill()
            .err()
            .filter(|error| error.kind() != io::ErrorKind::InvalidInput);
        let wait = child.wait().err();
        self.reaped = wait.is_none();
        drop(child);
        self.stop_watcher();
        cleanup_result(soft, hard, kill, wait)
    }

    fn wait_for_grace(&mut self) -> io::Result<()> {
        let until = Instant::now() + STOP_GRACE;
        while Instant::now() < until {
            if self
                .child
                .lock()
                .expect("ffmpeg child lock")
                .try_wait()?
                .is_some()
            {
                self.reaped = true;
                return Ok(());
            }
            thread::sleep(PROCESS_POLL);
        }
        Ok(())
    }

    fn stop_watcher(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        self.kill_and_reap();
    }
}

fn spawn_watcher(
    child: Arc<Mutex<Child>>,
    tree: Arc<Mutex<process_tree::ProcessTree>>,
    stopped: Arc<AtomicBool>,
    control: ProcessControl,
    context: &'static str,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while !stopped.load(Ordering::Acquire) {
            if control.check(context).is_err() {
                let _ = tree.lock().expect("ffmpeg tree lock").hard_stop();
                let _ = child.lock().expect("ffmpeg child lock").kill();
                return;
            }
            thread::sleep(PROCESS_POLL);
        }
    })
}

fn cleanup_result(
    soft: Option<io::Error>,
    hard: Option<io::Error>,
    kill: Option<io::Error>,
    wait: Option<io::Error>,
) -> Result<()> {
    let errors: Vec<_> = [
        ("graceful tree stop", soft),
        ("hard tree stop", hard),
        ("direct child kill", kill),
        ("direct child wait", wait),
    ]
    .into_iter()
    .filter_map(|(label, error)| error.map(|error| format!("{label}: {error}")))
    .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ff_err("clean up ffmpeg process", errors.join("; ")))
    }
}

pub(super) struct Reader {
    done: Receiver<io::Result<Vec<u8>>>,
    handle: Option<JoinHandle<()>>,
}

pub(super) fn read_capped<R: Read + Send + 'static>(mut input: R) -> Reader {
    let (tx, done) = mpsc::sync_channel(1);
    let handle = thread::spawn(move || {
        let mut kept = Vec::with_capacity(OUTPUT_CAP_BYTES);
        let mut buffer = [0_u8; 8192];
        let result = loop {
            match input.read(&mut buffer) {
                Ok(0) => break Ok(kept),
                Ok(count) => {
                    let room = OUTPUT_CAP_BYTES.saturating_sub(kept.len());
                    kept.extend_from_slice(&buffer[..count.min(room)]);
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
) -> Result<Vec<u8>> {
    let (result, timed_out, cleanup) = match reader.done.recv_timeout(STOP_GRACE) {
        Ok(result) => (Some(result), false, None),
        Err(_) => {
            let cleanup = child.stop_and_reap().err();
            (reader.done.recv_timeout(STOP_GRACE).ok(), true, cleanup)
        }
    };
    let joined = join_reader(&mut reader, context);
    if let Some(error) = cleanup {
        return Err(error);
    }
    joined?;
    match result {
        Some(result) => result.map_err(|error| ff_err(context, error)),
        None if timed_out => Err(ff_err(
            context,
            "owned ffmpeg pipe did not close after process-tree cleanup",
        )),
        None => Err(ff_err(context, "owned ffmpeg pipe reader disconnected")),
    }
}

fn join_reader(reader: &mut Reader, context: &str) -> Result<()> {
    if let Some(handle) = reader.handle.take() {
        handle
            .join()
            .map_err(|_| ff_err(context, "ffmpeg pipe reader panicked"))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "process/tests.rs"]
mod tests;

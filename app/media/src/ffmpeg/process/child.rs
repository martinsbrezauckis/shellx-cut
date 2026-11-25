//! Direct-child lifetime and cleanup diagnostics.

use super::{tree, RenderProcessControl, PROCESS_POLL, STOP_GRACE};
use cut_core::{error_codes, CutError};
use std::io;
use std::process::{Child, ChildStdin, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

// A killed process can remain stuck in kernel exit (seen on virtual macOS).
const REAP_GRACE: Duration = Duration::from_secs(5);

#[cfg(all(test, unix))]
#[path = "child_tests.rs"]
mod tests;

pub(super) struct ManagedChild {
    // None means ownership was transferred to the detached reaper, not reaped.
    child: Option<Child>,
    #[cfg(test)]
    skip_stop: bool,
    pub(super) tree: tree::ProcessTree,
    control: RenderProcessControl,
    reaped: bool,
    tree_closed: bool,
}

impl ManagedChild {
    pub(super) fn spawn(
        command: &mut Command,
        control: RenderProcessControl,
        context: &str,
    ) -> Result<Self, CutError> {
        control.check(context)?;
        tree::configure(command).map_err(|error| spawn_error(context, error))?;
        // A just-written program can be briefly busy (see process_start).
        let mut child =
            crate::process_start::retry_busy_start(|| command.spawn()).map_err(|error| {
                super::tool_missing::spawn_error(command, &error)
                    .unwrap_or_else(|| spawn_error(context, error))
            })?;
        let tree = match tree::ProcessTree::establish(&child) {
            Ok(tree) => tree,
            Err(error) => {
                let _ = child.kill();
                let _ = reap_stopped_child(&mut Some(child));
                return Err(spawn_error("claim ffmpeg process tree", error));
            }
        };
        Ok(Self {
            child: Some(child),
            #[cfg(test)]
            skip_stop: false,
            tree,
            control,
            reaped: false,
            tree_closed: false,
        })
    }

    pub(super) fn take_stdout(
        &mut self,
        context: &str,
    ) -> Result<std::process::ChildStdout, CutError> {
        self.child
            .as_mut()
            .and_then(|child| child.stdout.take())
            .ok_or_else(|| CutError::new(error_codes::FFMPEG, "ffmpeg stdout unavailable", context))
    }

    pub(super) fn take_stdin(&mut self, context: &str) -> Result<ChildStdin, CutError> {
        self.child
            .as_mut()
            .and_then(|child| child.stdin.take())
            .ok_or_else(|| CutError::new(error_codes::FFMPEG, "ffmpeg stdin unavailable", context))
    }

    pub(super) fn take_stderr(
        &mut self,
        context: &str,
    ) -> Result<std::process::ChildStderr, CutError> {
        self.child
            .as_mut()
            .and_then(|child| child.stderr.take())
            .ok_or_else(|| CutError::new(error_codes::FFMPEG, "ffmpeg stderr unavailable", context))
    }

    pub(super) fn try_wait(&mut self, context: &str) -> Result<Option<ExitStatus>, CutError> {
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| spawn_error(context, handed_off_error()))?;
        if !self
            .tree
            .exit_ready(child)
            .map_err(|error| spawn_error(context, error))?
        {
            return Ok(None);
        }
        self.tree
            .close_after_exit()
            .map_err(|error| spawn_error("close ffmpeg process tree", error))?;
        self.tree_closed = true;
        let status = child
            .try_wait()
            .map_err(|error| spawn_error(context, error))?
            .ok_or_else(|| {
                spawn_error(
                    context,
                    io::Error::other("exited ffmpeg child was unavailable for reap"),
                )
            })?;
        self.reaped = true;
        Ok(Some(status))
    }

    pub(super) fn wait(&mut self, context: &str) -> Result<ExitStatus, CutError> {
        loop {
            if let Err(error) = self.control.check(context) {
                let mut error = error;
                if let Err(cleanup) = self.stop_and_reap(context) {
                    error.cause = format!("{}; cleanup failed: {cleanup}", error.cause);
                }
                return Err(error);
            }
            if let Some(status) = self.try_wait(context)? {
                return Ok(status);
            }
            thread::sleep(PROCESS_POLL);
        }
    }

    pub(super) fn stop_and_reap(&mut self, context: &str) -> io::Result<()> {
        let child = self.child.as_mut().ok_or_else(handed_off_error)?;
        let _ = child.stdin.take();
        #[cfg(not(test))]
        let skip_stop = false;
        #[cfg(test)]
        let skip_stop = self.skip_stop;
        let exited_before_soft_stop = self.wait_for_grace().unwrap_or(false);
        let soft = if exited_before_soft_stop || skip_stop {
            None
        } else {
            self.tree.soft_stop().err()
        };
        let exited_after_soft_stop =
            exited_before_soft_stop || self.wait_for_grace().unwrap_or(false);
        let hard = if skip_stop {
            None
        } else if exited_after_soft_stop {
            self.tree.close_after_exit().err()
        } else {
            self.tree.hard_stop().err()
        };
        self.tree_closed = hard.is_none();
        let kill = if skip_stop {
            None
        } else {
            self.child
                .as_mut()
                .ok_or_else(handed_off_error)?
                .kill()
                .err()
                .filter(|error| error.kind() != io::ErrorKind::InvalidInput)
        };
        let wait = reap_stopped_child(&mut self.child).err();
        self.reaped = wait.is_none();
        combine_cleanup(context, soft, hard, kill, wait)
    }

    fn wait_for_grace(&mut self) -> io::Result<bool> {
        let until = Instant::now() + STOP_GRACE;
        while Instant::now() < until {
            let child = self.child.as_mut().ok_or_else(handed_off_error)?;
            if self.tree.exit_ready(child)? {
                return Ok(true);
            }
            thread::sleep(PROCESS_POLL);
        }
        Ok(false)
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        if self.child.is_some() && (!self.reaped || !self.tree_closed) {
            let _ = self.stop_and_reap("drop ffmpeg process owner");
        }
    }
}

fn handed_off_error() -> io::Error {
    io::Error::other("ffmpeg child is owned by the background reaper")
}

/// Never block the owner on a child stuck in exit, even during failed spawn cleanup.
fn reap_stopped_child(slot: &mut Option<Child>) -> io::Result<()> {
    let child = slot.as_mut().ok_or_else(handed_off_error)?;
    let until = Instant::now() + REAP_GRACE;
    let error = loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) if Instant::now() < until => thread::sleep(PROCESS_POLL),
            Ok(None) => {
                break io::Error::other(format!(
                    "program with pid {} did not finish exiting after it was stopped",
                    child.id()
                ));
            }
            Err(error) => break error,
        }
    };
    if let Some(mut child) = slot.take() {
        // Only this detached thread may wait without a deadline. Drop and later
        // calls see an empty slot and cannot double-wait or retry tree signals.
        thread::Builder::new()
            .name("ffmpeg-child-reaper".into())
            .spawn(move || {
                let _ = child.wait();
            })
            .map_err(|spawn| io::Error::other(format!("{error}; start child reaper: {spawn}")))?;
    }
    Err(error)
}

pub(super) fn spawn_error(context: &str, error: impl std::fmt::Display) -> CutError {
    CutError::new(
        error_codes::FFMPEG,
        format!("{context} failed"),
        error.to_string(),
    )
}

pub(super) fn exit_error(status: ExitStatus, stderr: &[u8]) -> CutError {
    let text = String::from_utf8_lossy(stderr);
    let tail: String = text
        .chars()
        .skip(text.chars().count().saturating_sub(2000))
        .collect();
    CutError::new(
        error_codes::FFMPEG,
        format!("ffmpeg exited with {status}"),
        tail,
    )
}

fn combine_cleanup(
    context: &str,
    soft: Option<io::Error>,
    hard: Option<io::Error>,
    kill: Option<io::Error>,
    wait: Option<io::Error>,
) -> io::Result<()> {
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
        Err(io::Error::other(format!(
            "{context}: {}",
            errors.join("; ")
        )))
    }
}

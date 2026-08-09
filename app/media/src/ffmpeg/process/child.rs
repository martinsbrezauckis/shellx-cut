//! Direct-child lifetime and cleanup diagnostics.

use super::{tree, RenderProcessControl, PROCESS_POLL, STOP_GRACE};
use cut_core::{error_codes, CutError};
use std::io;
use std::process::{Child, ChildStdin, Command, ExitStatus};
use std::thread;
use std::time::Instant;

pub(super) struct ManagedChild {
    child: Child,
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
        let mut child = command
            .spawn()
            .map_err(|error| spawn_error(context, error))?;
        let tree = match tree::ProcessTree::establish(&child) {
            Ok(tree) => tree,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(spawn_error("claim ffmpeg process tree", error));
            }
        };
        Ok(Self {
            child,
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
            .stdout
            .take()
            .ok_or_else(|| CutError::new(error_codes::FFMPEG, "ffmpeg stdout unavailable", context))
    }

    pub(super) fn take_stdin(&mut self, context: &str) -> Result<ChildStdin, CutError> {
        self.child
            .stdin
            .take()
            .ok_or_else(|| CutError::new(error_codes::FFMPEG, "ffmpeg stdin unavailable", context))
    }

    pub(super) fn take_stderr(
        &mut self,
        context: &str,
    ) -> Result<std::process::ChildStderr, CutError> {
        self.child
            .stderr
            .take()
            .ok_or_else(|| CutError::new(error_codes::FFMPEG, "ffmpeg stderr unavailable", context))
    }

    pub(super) fn try_wait(&mut self, context: &str) -> Result<Option<ExitStatus>, CutError> {
        let status = self
            .child
            .try_wait()
            .map_err(|error| spawn_error(context, error))?;
        if status.is_some() {
            self.reaped = true;
            match self.tree.hard_stop() {
                Ok(()) => self.tree_closed = true,
                Err(error) => return Err(spawn_error("close ffmpeg process tree", error)),
            }
        }
        Ok(status)
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
        let _ = self.child.stdin.take();
        let _ = self.wait_for_grace();
        let soft = self.tree.soft_stop().err();
        let _ = self.wait_for_grace();
        let hard = self.tree.hard_stop().err();
        self.tree_closed = hard.is_none();
        let kill = self
            .child
            .kill()
            .err()
            .filter(|error| error.kind() != io::ErrorKind::InvalidInput);
        let wait = self.child.wait().err();
        self.reaped = wait.is_none();
        combine_cleanup(context, soft, hard, kill, wait)
    }

    fn wait_for_grace(&mut self) -> io::Result<()> {
        let until = Instant::now() + STOP_GRACE;
        while Instant::now() < until {
            if self.child.try_wait()?.is_some() {
                self.reaped = true;
                return Ok(());
            }
            thread::sleep(PROCESS_POLL);
        }
        Ok(())
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        if !self.reaped || !self.tree_closed {
            let _ = self.stop_and_reap("drop ffmpeg process owner");
        }
    }
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

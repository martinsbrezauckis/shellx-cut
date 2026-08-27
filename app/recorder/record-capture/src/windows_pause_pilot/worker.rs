//! WGC-owned private worker behavior, isolated from the Cut server.

use std::time::{Duration, Instant};

use super::logical_run::ActiveLogicalRun;
use super::worker_commands::drain_stop_dominant;
use super::worker_facts::{failed, pilot_started};
use super::{
    WindowsPausePilotChannelError, WindowsPausePilotCommand, WindowsPausePilotCommandReceiver,
    WindowsPausePilotEvent, WindowsPausePilotEventSender, WindowsPausePilotProfile,
    WindowsPausePilotRefusal, WindowsPausePilotStartError, WindowsSealedScreenRun,
};
use crate::windows_wgc_run::{
    SealedScreenRun, WgcCheckpointPublisher, WgcControlFactory, WgcRunOwner, WgcStartObservation,
};

/// WGC native ownership for one newly started pause-pilot session. `resolve`
/// is called again on every Resume and has no fallback input. Physical WGC
/// checkpoint numbers are independent from server Pause/Resume generations.
pub(crate) struct WindowsPausePilotWorker<T, F, P, R>
where
    F: WgcControlFactory<T>,
    P: WgcCheckpointPublisher,
    R: FnMut(&str) -> Option<T>,
{
    profile: WindowsPausePilotProfile,
    owner: WgcRunOwner<T, F, P>,
    resolve: R,
    logical_run: Option<ActiveLogicalRun>,
    stopped: bool,
}

impl<T, F, P, R> WindowsPausePilotWorker<T, F, P, R>
where
    F: WgcControlFactory<T>,
    P: WgcCheckpointPublisher,
    R: FnMut(&str) -> Option<T>,
{
    /// Started is emitted only after exact resolution, WGC acceptance, and one
    /// paired native monotonic/Unix observation have all succeeded.
    pub(crate) fn start<B>(
        profile: WindowsPausePilotProfile,
        mut resolve: R,
        make_owner: B,
        reserve_start_ms: impl FnOnce() -> u64,
        observe_started: impl FnOnce() -> record_core::Result<WgcStartObservation>,
    ) -> Result<(Self, WindowsPausePilotEvent), WindowsPausePilotStartError>
    where
        B: FnOnce(T) -> record_core::Result<WgcRunOwner<T, F, P>>,
    {
        let target = resolve(profile.exact_monitor_id()).ok_or(
            WindowsPausePilotStartError::Refused(WindowsPausePilotRefusal::TargetUnavailable),
        )?;
        let mut owner =
            make_owner(target).map_err(|_| WindowsPausePilotStartError::NativeStartFailed)?;
        let identity = owner
            .begin(reserve_start_ms(), observe_started)
            .map_err(|_| WindowsPausePilotStartError::NativeStartFailed)?;
        let started = match pilot_started(&profile, identity) {
            Ok(started) => started,
            Err(()) => {
                let _ = owner.abort_unpublished_start();
                return Err(WindowsPausePilotStartError::NativeStartFailed);
            }
        };
        Ok((
            Self {
                profile,
                owner,
                resolve,
                logical_run: Some(ActiveLogicalRun::with_observed_start(&started)),
                stopped: false,
            },
            WindowsPausePilotEvent::Started { started },
        ))
    }

    /// Drain one non-blocking command burst. A queued Stop clears earlier
    /// Pause/Resume work before it enters WGC. Channel loss always closes and
    /// joins the owned worker before this function returns an error.
    pub(crate) fn process_available(
        &mut self,
        commands: &WindowsPausePilotCommandReceiver,
        events: &WindowsPausePilotEventSender,
        mut reserve_start_ms: impl FnMut() -> u64,
        mut observe_started: impl FnMut() -> record_core::Result<WgcStartObservation>,
        mut observe_boundary: impl FnMut() -> (u64, Instant),
    ) -> Result<(), WindowsPausePilotChannelError> {
        let queued = match drain_stop_dominant(commands, None) {
            Ok(queued) => queued,
            Err(error) => {
                self.close_and_reap(&mut observe_boundary);
                return Err(error);
            }
        };
        for command in queued {
            let terminal = matches!(command, WindowsPausePilotCommand::Stop { .. });
            let event = self.handle(
                command,
                &mut reserve_start_ms,
                &mut observe_started,
                &mut observe_boundary,
            );
            if let Err(error) = events.send(event) {
                self.close_and_reap(&mut observe_boundary);
                return Err(error);
            }
            if terminal {
                break;
            }
        }
        Ok(())
    }

    /// Run the owned worker with the configured checkpoint rollover cadence.
    /// Every elapsed cadence is an ordinary close→verify/publish→reopen WGC
    /// checkpoint rollover inside the current logical run. Stop wakes this wait
    /// immediately; a lost peer is terminal. This is not the server lifecycle
    /// hook.
    pub(crate) fn run_until_terminal(
        &mut self,
        commands: &WindowsPausePilotCommandReceiver,
        events: &WindowsPausePilotEventSender,
        rollover_interval: Duration,
        mut reserve_start_ms: impl FnMut() -> u64,
        mut observe_started: impl FnMut() -> record_core::Result<WgcStartObservation>,
        mut observe_boundary: impl FnMut() -> (u64, Instant),
    ) -> Result<(), WindowsPausePilotChannelError> {
        loop {
            let first = match commands.recv_timeout(rollover_interval) {
                Ok(Some(command)) => command,
                Ok(None) => {
                    if self
                        .rollover_checkpoint(
                            &mut reserve_start_ms,
                            &mut observe_started,
                            &mut observe_boundary,
                        )
                        .is_err()
                    {
                        self.close_and_reap(&mut observe_boundary);
                        return Err(WindowsPausePilotChannelError::NativeTerminal);
                    }
                    continue;
                }
                Err(error) => {
                    self.close_and_reap(&mut observe_boundary);
                    return Err(error);
                }
            };
            let queued = match drain_stop_dominant(commands, Some(first)) {
                Ok(queued) => queued,
                Err(error) => {
                    self.close_and_reap(&mut observe_boundary);
                    return Err(error);
                }
            };
            for command in queued {
                let terminal = matches!(command, WindowsPausePilotCommand::Stop { .. });
                let event = self.handle(
                    command,
                    &mut reserve_start_ms,
                    &mut observe_started,
                    &mut observe_boundary,
                );
                if let Err(error) = events.send(event) {
                    self.close_and_reap(&mut observe_boundary);
                    return Err(error);
                }
                if terminal || self.stopped {
                    return Ok(());
                }
            }
        }
    }

    fn handle(
        &mut self,
        command: WindowsPausePilotCommand,
        reserve_start_ms: &mut impl FnMut() -> u64,
        observe_started: &mut impl FnMut() -> record_core::Result<WgcStartObservation>,
        observe_boundary: &mut impl FnMut() -> (u64, Instant),
    ) -> WindowsPausePilotEvent {
        if self.stopped {
            return failed(command, observe_boundary().1);
        }
        match command {
            WindowsPausePilotCommand::Pause { generation, epoch } => {
                let mut observed_at = None;
                match self.owner.seal_active_checkpoint(|| {
                    let (observed_ms, observed) = observe_boundary();
                    observed_at = Some(observed);
                    observed_ms
                }) {
                    Ok(run) => match self.seal_logical_run(run) {
                        Ok(run) => WindowsPausePilotEvent::PauseSealed {
                            generation,
                            epoch,
                            run,
                            observed_at: observed_at.expect("sealed run observes after close"),
                        },
                        Err(()) => {
                            self.close_and_reap(observe_boundary);
                            failed(command, observe_boundary().1)
                        }
                    },
                    Err(_) => {
                        self.close_and_reap(observe_boundary);
                        failed(command, observe_boundary().1)
                    }
                }
            }
            WindowsPausePilotCommand::Resume { generation, epoch } => {
                let Some(target) = (self.resolve)(self.profile.exact_monitor_id()) else {
                    return WindowsPausePilotEvent::ResumeRefused {
                        generation,
                        epoch,
                        refusal: WindowsPausePilotRefusal::TargetUnavailable,
                        observed_at: observe_boundary().1,
                    };
                };
                match self
                    .owner
                    .resume_with_target(target, reserve_start_ms(), observe_started)
                {
                    Ok(identity) => match pilot_started(&self.profile, identity) {
                        Ok(started) if self.logical_run.is_none() => {
                            self.logical_run =
                                Some(ActiveLogicalRun::with_observed_start(&started));
                            WindowsPausePilotEvent::ResumeReady {
                                generation,
                                epoch,
                                started,
                            }
                        }
                        Ok(_) => {
                            let _ = self.owner.abort_unpublished_start();
                            self.close_and_reap(observe_boundary);
                            failed(command, observe_boundary().1)
                        }
                        Err(()) => {
                            let _ = self.owner.abort_unpublished_start();
                            self.close_and_reap(observe_boundary);
                            failed(command, observe_boundary().1)
                        }
                    },
                    Err(_) => {
                        self.close_and_reap(observe_boundary);
                        failed(command, observe_boundary().1)
                    }
                }
            }
            WindowsPausePilotCommand::Stop { epoch } => {
                self.stopped = true;
                let mut observed_at = None;
                match self.owner.stop(|| {
                    let (observed_ms, observed) = observe_boundary();
                    observed_at = Some(observed);
                    observed_ms
                }) {
                    Ok(Some(run)) => match self.seal_logical_run(run) {
                        Ok(run) => WindowsPausePilotEvent::StopSealed {
                            epoch,
                            run: Some(run),
                            observed_at: observed_at.expect("sealed run observes after close"),
                        },
                        Err(()) => {
                            self.close_and_reap(observe_boundary);
                            failed(command, observe_boundary().1)
                        }
                    },
                    Ok(None) if self.logical_run.is_none() => WindowsPausePilotEvent::StopSealed {
                        epoch,
                        run: None,
                        observed_at: observed_at.unwrap_or_else(|| observe_boundary().1),
                    },
                    Ok(None) => {
                        self.close_and_reap(observe_boundary);
                        failed(command, observe_boundary().1)
                    }
                    Err(_) => {
                        self.close_and_reap(observe_boundary);
                        failed(command, observe_boundary().1)
                    }
                }
            }
        }
    }

    pub(super) fn close_and_reap(&mut self, observe_boundary: &mut impl FnMut() -> (u64, Instant)) {
        self.stopped = true;
        // `WgcNativeControl::close` joins the encoder worker. Its error cannot
        // be sent to a disappeared peer, but the terminal owner state prevents
        // every later Resume from replacing this native result.
        let _ = self.owner.stop(|| observe_boundary().0);
        self.logical_run = None;
    }

    fn rollover_checkpoint(
        &mut self,
        reserve_start_ms: &mut impl FnMut() -> u64,
        observe_started: &mut impl FnMut() -> record_core::Result<WgcStartObservation>,
        observe_boundary: &mut impl FnMut() -> (u64, Instant),
    ) -> Result<(), ()> {
        let reserved_start_ms = reserve_start_ms();
        let (sealed, next) = self
            .owner
            .rollover_checkpoint(|| observe_boundary().0, reserved_start_ms, observe_started)
            .map_err(|_| ())?;
        let sealed_end_ms = sealed.boundary.end_ms;
        let started = pilot_started(&self.profile, next.clone())?;
        let logical = self.logical_run.as_mut().ok_or(())?;
        logical.append(sealed)?;
        if !logical.accepts(&started.accepted) || next.start_ms < sealed_end_ms {
            return Err(());
        }
        Ok(())
    }

    fn seal_logical_run(&mut self, run: SealedScreenRun) -> Result<WindowsSealedScreenRun, ()> {
        let logical = self.logical_run.as_mut().ok_or(())?;
        logical.append(run)?;
        self.logical_run.take().ok_or(())?.seal()
    }
}

#[cfg(test)]
#[path = "worker_test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "worker_rotation_tests.rs"]
mod rotation_tests;

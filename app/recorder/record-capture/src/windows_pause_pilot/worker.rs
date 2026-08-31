//! WGC-owned private worker behavior, isolated from the Cut server.

use std::time::{Duration, Instant};

use super::audio_run::{start_for_profile as start_audio_for_profile, WindowsPauseAudioFactory};
use super::input_run::{start_for_profile, WindowsPauseInputFactory};
use super::logical_run::ActiveLogicalRun;
use super::worker_commands::drain_stop_dominant;
use super::worker_facts::pilot_started;
use super::worker_rollover::rollover_checkpoint;
use super::{
    WindowsPausePilotChannelError, WindowsPausePilotCommand, WindowsPausePilotCommandReceiver,
    WindowsPausePilotEvent, WindowsPausePilotEventSender, WindowsPausePilotProfile,
    WindowsPausePilotRefusal, WindowsPausePilotStartError,
};
use crate::windows_wgc_run::{
    WgcCheckpointPublisher, WgcControlFactory, WgcRunOwner, WgcStartObservation,
};

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
    input_factory: Box<dyn WindowsPauseInputFactory>,
    audio_factory: Box<dyn WindowsPauseAudioFactory>,
    stopped: bool,
}

impl<T, F, P, R> WindowsPausePilotWorker<T, F, P, R>
where
    F: WgcControlFactory<T>,
    P: WgcCheckpointPublisher,
    R: FnMut(&str) -> Option<T>,
{
    pub(crate) fn start<B>(
        profile: WindowsPausePilotProfile,
        mut resolve: R,
        make_owner: B,
        mut input_factory: Box<dyn WindowsPauseInputFactory>,
        mut audio_factory: Box<dyn WindowsPauseAudioFactory>,
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
        let input = match start_for_profile(&profile, input_factory.as_mut(), &started) {
            Ok(input) => input,
            Err(()) => {
                let _ = owner.abort_unpublished_start();
                return Err(WindowsPausePilotStartError::NativeStartFailed);
            }
        };
        let audio = match start_audio_for_profile(&profile, audio_factory.as_mut(), &started) {
            Ok(audio) => audio,
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
                logical_run: Some(ActiveLogicalRun::with_observed_start(
                    &started, input, audio,
                )),
                input_factory,
                audio_factory,
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
                    if rollover_checkpoint(
                        &mut self.owner,
                        &self.profile,
                        &mut self.logical_run,
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
}

mod control;

#[cfg(test)]
#[path = "worker_test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "worker_rotation_tests.rs"]
mod rotation_tests;

#[cfg(test)]
#[path = "worker_input_tests.rs"]
mod input_tests;

#[cfg(test)]
#[path = "worker_audio_tests.rs"]
mod audio_tests;

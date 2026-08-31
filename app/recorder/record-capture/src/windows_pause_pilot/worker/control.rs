use std::time::Instant;

use super::super::audio_run::start_for_profile as start_audio_for_profile;
use super::super::input_run::start_for_profile;
use super::super::logical_run::{seal_logical_run, ActiveLogicalRun};
use super::super::worker_facts::{failed, pilot_started};
use super::super::{
    WindowsPausePilotCommand, WindowsPausePilotEvent, WindowsPausePilotRefusal,
    WindowsSealedScreenRun,
};
use super::WindowsPausePilotWorker;
use crate::windows_wgc_run::{
    SealedScreenRun, WgcCheckpointPublisher, WgcControlFactory, WgcStartObservation,
};

impl<T, F, P, R> WindowsPausePilotWorker<T, F, P, R>
where
    F: WgcControlFactory<T>,
    P: WgcCheckpointPublisher,
    R: FnMut(&str) -> Option<T>,
{
    pub(super) fn handle(
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
                        Ok((run, input, audio)) => WindowsPausePilotEvent::PauseSealed {
                            generation,
                            epoch,
                            run,
                            input,
                            audio,
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
                            match start_for_profile(
                                &self.profile,
                                self.input_factory.as_mut(),
                                &started,
                            ) {
                                Ok(input) => match start_audio_for_profile(
                                    &self.profile,
                                    self.audio_factory.as_mut(),
                                    &started,
                                ) {
                                    Ok(audio) => {
                                        self.logical_run =
                                            Some(ActiveLogicalRun::with_observed_start(
                                                &started, input, audio,
                                            ));
                                        WindowsPausePilotEvent::ResumeReady {
                                            generation,
                                            epoch,
                                            started,
                                        }
                                    }
                                    Err(()) => {
                                        let _ = self.owner.abort_unpublished_start();
                                        self.close_and_reap(observe_boundary);
                                        failed(command, observe_boundary().1)
                                    }
                                },
                                Err(()) => {
                                    let _ = self.owner.abort_unpublished_start();
                                    self.close_and_reap(observe_boundary);
                                    failed(command, observe_boundary().1)
                                }
                            }
                        }
                        Ok(_) | Err(()) => {
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
                        Ok((run, input, audio)) => WindowsPausePilotEvent::StopSealed {
                            epoch,
                            run: Some(run),
                            input,
                            audio,
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
                        input: None,
                        audio: Vec::new(),
                        observed_at: observed_at.unwrap_or_else(|| observe_boundary().1),
                    },
                    Ok(None) | Err(_) => {
                        self.close_and_reap(observe_boundary);
                        failed(command, observe_boundary().1)
                    }
                }
            }
        }
    }

    pub(in super::super) fn close_and_reap(
        &mut self,
        observe_boundary: &mut impl FnMut() -> (u64, Instant),
    ) {
        self.stopped = true;
        let _ = self.owner.stop(|| observe_boundary().0);
        self.logical_run = None;
    }

    fn seal_logical_run(
        &mut self,
        run: SealedScreenRun,
    ) -> Result<
        (
            WindowsSealedScreenRun,
            Option<super::super::WindowsSealedInputRun>,
            Vec<super::super::WindowsSealedAudioRun>,
        ),
        (),
    > {
        let logical = self.logical_run.as_mut().ok_or(())?;
        logical.append(run)?;
        seal_logical_run(
            self.logical_run.take().ok_or(())?,
            self.profile.captures_input(),
            &self.profile.selected_audio_streams(),
        )
    }
}

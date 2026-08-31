//! Stop-dominant private ownership over one exact macOS screen run.

use super::control::CommandState;
use super::validation::{valid_sealed_run, valid_started};
use super::{
    abort_selected, seal_selected_after_screen, start_for_profile, MacosPauseAudioFactory,
    MacosPauseAudioOwner, MacosPauseCommand, MacosPausePilotEvent, MacosPausePilotProfile,
    MacosPausePilotStarted, MacosPauseStartError, MacosSealedAudioRun, MacosSealedScreenRun,
};
use std::time::Instant;

/// Native ScreenCaptureKit boundary for the crate-private pause seam.
///
/// Implementations must freshly resolve `profile.exact_monitor_id()` at each
/// start, bind that exact opaque identity into `MacosPausePilotStarted`, and
/// retain partial terminal-close state until `abort_and_join` reports success.
pub(crate) trait MacosPauseScreenOwner {
    fn start(
        &mut self,
        profile: &MacosPausePilotProfile,
    ) -> Result<MacosPausePilotStarted, MacosPauseStartError>;

    fn seal_active(&mut self) -> Result<MacosSealedScreenRun, ()>;

    /// Terminal cleanup: close the active ScreenCaptureKit output and join/reap
    /// every owner thread before returning `Ok`. On `Err`, retain its partial
    /// close state so the owning run can make its bounded retry.
    fn abort_and_join(&mut self) -> Result<(), ()>;
}

/// Owns exactly one active native screen generation and its selected audio
/// siblings. It is crate-private and has no server journal/projection caller
/// or ordinary recorder API admission.
pub(crate) struct MacosPauseRunOwner<S, A>
where
    S: MacosPauseScreenOwner,
    A: MacosPauseAudioFactory,
{
    profile: MacosPausePilotProfile,
    screen: S,
    audio_factory: A,
    active: Option<ActiveRun>,
    commands: CommandState,
    stopped: bool,
    teardown_complete: bool,
    teardown_error: Option<TerminalTeardownError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalTeardownError {
    NativeCloseUnresolved,
}

struct ActiveRun {
    started: MacosPausePilotStarted,
    audio: Vec<Box<dyn MacosPauseAudioOwner>>,
}

impl ActiveRun {
    fn physical_generation(&self) -> u64 {
        self.started.physical_generation
    }

    fn abort_and_join(self) {
        abort_selected(self.audio);
    }
}

impl<S, A> MacosPauseRunOwner<S, A>
where
    S: MacosPauseScreenOwner,
    A: MacosPauseAudioFactory,
{
    pub(crate) fn start(
        profile: MacosPausePilotProfile,
        mut screen: S,
        mut audio_factory: A,
    ) -> Result<(Self, MacosPausePilotEvent), MacosPauseStartError> {
        let started = screen.start(&profile)?;
        if !valid_started(&profile, &started) {
            abort_rejected_start(&mut screen);
            return Err(MacosPauseStartError::NativeStartFailed);
        }
        let audio = match start_for_profile(&profile, &mut audio_factory, &started) {
            Ok(audio) => audio,
            Err(()) => {
                abort_rejected_start(&mut screen);
                return Err(MacosPauseStartError::NativeStartFailed);
            }
        };
        Ok((
            Self {
                profile,
                screen,
                audio_factory,
                active: Some(ActiveRun {
                    started: started.clone(),
                    audio,
                }),
                commands: CommandState::default(),
                stopped: false,
                teardown_complete: false,
                teardown_error: None,
            },
            MacosPausePilotEvent::Started { started },
        ))
    }

    /// Process a burst in delivery order, except a queued `Stop` discards every
    /// earlier Pause/Resume before the screen or audio owner is touched.
    pub(crate) fn process_commands(
        &mut self,
        commands: impl IntoIterator<Item = MacosPauseCommand>,
        observed_at: Instant,
    ) -> Vec<MacosPausePilotEvent> {
        let (commands, discarded_epochs) = stop_dominant(commands);
        let mut events = Vec::with_capacity(commands.len());
        for command in commands {
            let terminal = matches!(command, MacosPauseCommand::Stop { .. });
            events.push(self.handle(command, observed_at, &discarded_epochs));
            if terminal {
                break;
            }
        }
        events
    }

    fn handle(
        &mut self,
        command: MacosPauseCommand,
        observed_at: Instant,
        discarded_epochs: &[u64],
    ) -> MacosPausePilotEvent {
        let active_generation = self.active.as_ref().map(ActiveRun::physical_generation);
        if let Err(rejection) =
            self.commands
                .validate(command, active_generation, self.stopped, discarded_epochs)
        {
            return MacosPausePilotEvent::Rejected { command, rejection };
        }
        // A command is consumed before it drives native state. A delayed retry
        // therefore cannot become valid after a refusal or failed boundary.
        self.commands.accept(command);
        match command {
            MacosPauseCommand::Pause {
                generation, epoch, ..
            } => {
                let Ok(Some((run, audio))) = self.seal_active() else {
                    return self.failed(command, observed_at);
                };
                MacosPausePilotEvent::PauseSealed {
                    generation,
                    epoch,
                    run,
                    audio,
                }
            }
            MacosPauseCommand::Resume {
                generation,
                epoch,
                resume_from_physical_generation,
            } => {
                let started = match self.screen.start(&self.profile) {
                    Ok(started)
                        if valid_started(&self.profile, &started)
                            && resume_from_physical_generation.checked_add(1)
                                == Some(started.physical_generation) =>
                    {
                        started
                    }
                    _ => {
                        if self.screen.abort_and_join().is_err() {
                            return self.failed(command, observed_at);
                        }
                        return MacosPausePilotEvent::ResumeRefused { generation, epoch };
                    }
                };
                let audio =
                    match start_for_profile(&self.profile, &mut self.audio_factory, &started) {
                        Ok(audio) => audio,
                        Err(()) => return self.failed(command, observed_at),
                    };
                self.active = Some(ActiveRun {
                    started: started.clone(),
                    audio,
                });
                MacosPausePilotEvent::ResumeReady {
                    generation,
                    epoch,
                    started,
                }
            }
            MacosPauseCommand::Stop { epoch, .. } => {
                // Terminalize before close: a nested or delayed command cannot
                // reopen a run after this Stop was admitted.
                self.stopped = true;
                match self.seal_active() {
                    Ok(Some((run, audio))) => {
                        self.teardown_complete = true;
                        MacosPausePilotEvent::StopSealed {
                            observed_at: run.observed_at,
                            epoch,
                            run: Some(run),
                            audio,
                        }
                    }
                    Ok(None) if self.commands.last_sealed_physical_generation().is_some() => {
                        self.teardown_complete = true;
                        MacosPausePilotEvent::StopSealed {
                            epoch,
                            run: None,
                            audio: Vec::new(),
                            observed_at,
                        }
                    }
                    Ok(None) | Err(()) => self.failed(command, observed_at),
                }
            }
        }
    }

    fn seal_active(
        &mut self,
    ) -> Result<Option<(MacosSealedScreenRun, Vec<MacosSealedAudioRun>)>, ()> {
        let Some(active) = self.active.take() else {
            return Ok(None);
        };
        // ScreenCaptureKit must close and publish first; only then may selected
        // Mic/Core Audio stop and publish their sidecars.
        let run = match self.screen.seal_active() {
            Ok(run) => run,
            Err(()) => {
                // Keep the selected audio beside the resumable screen close.
                // `failed` retries that exact owner and marks teardown complete
                // only after every native close acknowledgement is known.
                self.active = Some(active);
                return Err(());
            }
        };
        if !valid_sealed_run(&active.started, &run) {
            self.active = Some(active);
            return Err(());
        }
        let audio = seal_selected_after_screen(
            active.audio,
            &self.profile.selected_audio_streams(),
            run.observed_end_ms,
        )?;
        self.commands
            .note_sealed_physical_generation(active.started.physical_generation);
        Ok(Some((run, audio)))
    }

    fn failed(&mut self, command: MacosPauseCommand, observed_at: Instant) -> MacosPausePilotEvent {
        let teardown_unresolved = self.terminal_abort_and_join().is_err();
        MacosPausePilotEvent::Failed {
            command,
            observed_at,
            teardown_unresolved,
        }
    }

    /// Keep teardown incomplete until ScreenCaptureKit has closed its output
    /// through every resumable phase. The stored error is intentionally kept
    /// for the later bounded retry (including the owner Drop path).
    fn terminal_abort_and_join(&mut self) -> Result<(), TerminalTeardownError> {
        if self.teardown_complete {
            return Ok(());
        }
        self.stopped = true;
        if self.screen.abort_and_join().is_err() {
            self.teardown_error = Some(TerminalTeardownError::NativeCloseUnresolved);
            return Err(TerminalTeardownError::NativeCloseUnresolved);
        }
        if let Some(active) = self.active.take() {
            active.abort_and_join();
        }
        self.teardown_complete = true;
        self.teardown_error = None;
        Ok(())
    }
}

/// Startup has no owner value to retain after it returns an error. Spend the
/// same bounded native close retry before that return and report an unresolved
/// close rather than pretending a local destructor completed it.
fn abort_rejected_start<S: MacosPauseScreenOwner>(screen: &mut S) {
    if screen.abort_and_join().is_err() && screen.abort_and_join().is_err() {
        eprintln!("private macOS pause start rejected with unresolved native close state");
    }
}

impl<S, A> Drop for MacosPauseRunOwner<S, A>
where
    S: MacosPauseScreenOwner,
    A: MacosPauseAudioFactory,
{
    fn drop(&mut self) {
        let first = self.terminal_abort_and_join();
        let result = if first.is_err() {
            // Consume the one retained retry while this owner still exists.
            // `NativeCloseState` refuses a third native invocation, so Drop
            // cannot turn an unresolved close into an unbounded loop.
            self.terminal_abort_and_join()
        } else {
            first
        };
        if let Err(error) = result {
            // Drop cannot return the retained terminal error. Do not claim
            // cleanup completed or hide it behind an implicit destructor.
            debug_assert_eq!(self.teardown_error, Some(error));
            eprintln!("private macOS pause owner dropped with unresolved native close state");
        }
    }
}

fn stop_dominant(
    commands: impl IntoIterator<Item = MacosPauseCommand>,
) -> (Vec<MacosPauseCommand>, Vec<u64>) {
    let commands = commands.into_iter().collect::<Vec<_>>();
    if let Some(stop_index) = commands
        .iter()
        .position(|command| matches!(command, MacosPauseCommand::Stop { .. }))
    {
        // The shared owner may already have assigned epochs to commands that
        // this native boundary must discard because Stop is dominant. Retain
        // their exact prefix so command correlation can prove the terminal
        // epoch accounts for every skipped edge; trailing commands are not
        // evidence and cannot authorize a Stop-first epoch gap.
        let discarded_epochs = commands[..stop_index]
            .iter()
            .map(|command| command.epoch())
            .collect();
        return (vec![commands[stop_index]], discarded_epochs);
    }
    (commands, Vec::new())
}

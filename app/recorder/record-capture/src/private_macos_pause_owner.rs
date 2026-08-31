//! Server-only lifecycle adapter for the private macOS pause owner.
//!
//! The server's durable pause coordinator consumes the existing neutral worker
//! protocol. This adapter translates ScreenCaptureKit facts into that protocol
//! without exposing monitor labels, native paths, or a second public recorder.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::macos_pause_pilot::{
    MacosPauseCommand, MacosPausePilotEvent, MacosPausePilotProfile, MacosPausePilotRequest,
    MacosPauseRunOwner, MacosPauseStartError, RequiredMacosPauseAudioFactory,
    RequiredMacosPauseScreenOwner,
};
use crate::windows_pause_pilot::{
    channel, WindowsPausePilotChannelError, WindowsPausePilotCommand,
    WindowsPausePilotCommandReceiver, WindowsPausePilotCommandSender, WindowsPausePilotEvent,
    WindowsPausePilotEventReceiver, WindowsPausePilotEventSender, WindowsPausePilotOperation,
};
use crate::{CheckpointConfig, MicrophoneSource};

mod conversion;
use conversion::translate_event;

const STARTUP_WAIT: Duration = Duration::from_secs(15);
const COMMAND_POLL: Duration = Duration::from_millis(50);

pub struct MacosPausePilotThread {
    commands: Option<WindowsPausePilotCommandSender>,
    events: Option<WindowsPausePilotEventReceiver>,
    shutdown: Arc<AtomicBool>,
    join: Option<JoinHandle<Result<(), WindowsPausePilotChannelError>>>,
}

impl MacosPausePilotThread {
    pub fn command_sender(&self) -> WindowsPausePilotCommandSender {
        self.commands
            .as_ref()
            .expect("macOS pause command owner remains live until join")
            .clone()
    }

    pub fn take_event_receiver(&mut self) -> Option<WindowsPausePilotEventReceiver> {
        self.events.take()
    }

    pub fn shutdown_and_join(&mut self) -> Result<(), WindowsPausePilotChannelError> {
        self.shutdown.store(true, Ordering::Release);
        self.commands.take();
        self.join_owned().unwrap_or(Ok(()))
    }

    pub fn join_after_terminal(&mut self) -> Result<(), WindowsPausePilotChannelError> {
        self.commands.take();
        self.join_owned().unwrap_or(Ok(()))
    }

    fn join_owned(&mut self) -> Option<Result<(), WindowsPausePilotChannelError>> {
        self.join.take().map(|join| match join.join() {
            Ok(result) => result,
            Err(_) => Err(WindowsPausePilotChannelError::NativeTerminal),
        })
    }
}

impl Drop for MacosPausePilotThread {
    fn drop(&mut self) {
        let _ = self.shutdown_and_join();
    }
}

pub fn start_private(
    exact_monitor_id: String,
    fps: f64,
    microphone: bool,
    system_audio: bool,
    microphone_source: MicrophoneSource,
    checkpoint: CheckpointConfig,
) -> record_core::Result<MacosPausePilotThread> {
    if checkpoint.interval_ms == 0 {
        return Err(start_error());
    }
    let profile = MacosPausePilotProfile::admit(MacosPausePilotRequest::screen_with_audio(
        exact_monitor_id,
        fps,
        microphone,
        system_audio,
    ))
    .map_err(|_| start_error())?;
    spawn_owner(profile, microphone_source, checkpoint).map_err(|_| start_error())
}

fn spawn_owner(
    profile: MacosPausePilotProfile,
    microphone_source: MicrophoneSource,
    checkpoint: CheckpointConfig,
) -> Result<MacosPausePilotThread, MacosPauseStartError> {
    let (commands, receiver, sender, events) = channel();
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
    let join = thread::Builder::new()
        .name("shellx-cut-macos-pause".into())
        .spawn(move || {
            let screen = match RequiredMacosPauseScreenOwner::new(checkpoint.clone()) {
                Ok(screen) => screen,
                Err(error) => {
                    let _ = startup_sender.send(Err(error));
                    return Err(WindowsPausePilotChannelError::NativeTerminal);
                }
            };
            let audio = RequiredMacosPauseAudioFactory::new(
                checkpoint.manifest_dir.clone().into(),
                microphone_source,
            );
            let (owner, started) = match MacosPauseRunOwner::start(profile, screen, audio) {
                Ok(started) => started,
                Err(error) => {
                    let _ = startup_sender.send(Err(error));
                    return Err(WindowsPausePilotChannelError::NativeTerminal);
                }
            };
            run_owner(
                owner,
                started,
                receiver,
                sender,
                worker_shutdown,
                startup_sender,
            )
        })
        .map_err(|_| MacosPauseStartError::NativeStartFailed)?;
    let mut lifecycle = MacosPausePilotThread {
        commands: Some(commands),
        events: Some(events),
        shutdown,
        join: Some(join),
    };
    match startup_receiver.recv_timeout(STARTUP_WAIT) {
        Ok(Ok(())) => Ok(lifecycle),
        _ => {
            let _ = lifecycle.shutdown_and_join();
            Err(MacosPauseStartError::NativeStartFailed)
        }
    }
}

fn start_error() -> record_core::RecordError {
    record_core::RecordError::new(
        record_core::error_codes::CAPTURE,
        "start private macOS pause capture",
        "ScreenCaptureKit did not accept the exact selected display and streams",
    )
}

fn run_owner(
    mut owner: MacosPauseRunOwner<RequiredMacosPauseScreenOwner, RequiredMacosPauseAudioFactory>,
    started: MacosPausePilotEvent,
    commands: WindowsPausePilotCommandReceiver,
    events: WindowsPausePilotEventSender,
    shutdown: Arc<AtomicBool>,
    startup: mpsc::SyncSender<Result<(), MacosPauseStartError>>,
) -> Result<(), WindowsPausePilotChannelError> {
    let mut active_physical = None;
    let mut last_sealed_physical = None;
    let mut last_epoch = 0_u64;
    let event = translate_event(
        started,
        Instant::now(),
        &mut active_physical,
        &mut last_sealed_physical,
    )?;
    events.send(event)?;
    let _ = startup.send(Ok(()));

    loop {
        if shutdown.load(Ordering::Acquire) {
            if let Some(physical_generation) = active_physical.or(last_sealed_physical) {
                let epoch = last_epoch.saturating_add(1);
                let native = MacosPauseCommand::Stop {
                    epoch,
                    physical_generation,
                };
                let _ = owner.process_commands([native], Instant::now());
            }
            return Ok(());
        }
        let Some(command) = commands.recv_timeout(COMMAND_POLL)? else {
            continue;
        };
        let mut batch = vec![command];
        while let Some(next) = commands.try_recv()? {
            batch.push(next);
        }
        let observed_at = Instant::now();
        let mut native = Vec::with_capacity(batch.len());
        for command in batch {
            last_epoch = last_epoch.max(command.epoch());
            native.push(native_command(
                command,
                active_physical,
                last_sealed_physical,
            )?);
        }
        let terminal = native
            .iter()
            .any(|command| matches!(command, MacosPauseCommand::Stop { .. }));
        for event in owner.process_commands(native, observed_at) {
            events.send(translate_event(
                event,
                observed_at,
                &mut active_physical,
                &mut last_sealed_physical,
            )?)?;
        }
        if terminal {
            return Ok(());
        }
    }
}

fn native_command(
    command: WindowsPausePilotCommand,
    active: Option<u64>,
    last_sealed: Option<u64>,
) -> Result<MacosPauseCommand, WindowsPausePilotChannelError> {
    let physical = active
        .or(last_sealed)
        .ok_or(WindowsPausePilotChannelError::NativeTerminal)?;
    Ok(match command {
        WindowsPausePilotCommand::Pause { generation, epoch } => MacosPauseCommand::Pause {
            generation,
            epoch,
            physical_generation: physical,
        },
        WindowsPausePilotCommand::Resume { generation, epoch } => MacosPauseCommand::Resume {
            generation,
            epoch,
            resume_from_physical_generation: physical,
        },
        WindowsPausePilotCommand::Stop { epoch } => MacosPauseCommand::Stop {
            epoch,
            physical_generation: physical,
        },
    })
}

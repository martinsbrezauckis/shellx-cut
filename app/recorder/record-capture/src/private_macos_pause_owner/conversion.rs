use std::time::Instant;

use record_recovery::Checkpoint;

use crate::macos_pause_pilot::{
    MacosPauseCommand, MacosPausePilotEvent, MacosSealedAudioRun, MacosSealedScreenRun,
};
use crate::windows_pause_pilot::{
    WindowsPausePilotAcceptedCapture, WindowsPausePilotCaptureRange, WindowsPausePilotChannelError,
    WindowsPausePilotCheckpointRange, WindowsPausePilotEvent, WindowsPausePilotOperation,
    WindowsPausePilotRefusal, WindowsPausePilotStarted, WindowsSealedAudioRun,
    WindowsSealedScreenRun, WindowsSealedWgcCheckpoint,
};

pub(super) fn translate_event(
    event: MacosPausePilotEvent,
    batch_observed_at: Instant,
    active: &mut Option<u64>,
    last_sealed: &mut Option<u64>,
) -> Result<WindowsPausePilotEvent, WindowsPausePilotChannelError> {
    Ok(match event {
        MacosPausePilotEvent::Started { started } => {
            *active = Some(started.physical_generation);
            WindowsPausePilotEvent::Started {
                started: convert_started(started),
            }
        }
        MacosPausePilotEvent::PauseSealed {
            generation,
            epoch,
            run,
            audio,
        } => {
            *active = None;
            *last_sealed = Some(run.physical_generation);
            WindowsPausePilotEvent::PauseSealed {
                generation,
                epoch,
                observed_at: run.observed_at,
                run: convert_run(run)?,
                input: None,
                audio: convert_audio(audio),
            }
        }
        MacosPausePilotEvent::ResumeReady {
            generation,
            epoch,
            started,
        } => {
            *active = Some(started.physical_generation);
            WindowsPausePilotEvent::ResumeReady {
                generation,
                epoch,
                started: convert_started(started),
            }
        }
        MacosPausePilotEvent::ResumeRefused { generation, epoch } => {
            WindowsPausePilotEvent::ResumeRefused {
                generation,
                epoch,
                refusal: WindowsPausePilotRefusal::TargetUnavailable,
                observed_at: batch_observed_at,
            }
        }
        MacosPausePilotEvent::StopSealed {
            epoch,
            run,
            audio,
            observed_at,
        } => {
            if let Some(run) = run.as_ref() {
                *last_sealed = Some(run.physical_generation);
            }
            *active = None;
            WindowsPausePilotEvent::StopSealed {
                epoch,
                run: run.map(convert_run).transpose()?,
                input: None,
                audio: convert_audio(audio),
                observed_at,
            }
        }
        MacosPausePilotEvent::Failed {
            command,
            observed_at,
            ..
        } => {
            let (operation, generation, epoch) = command_parts(command);
            WindowsPausePilotEvent::Failed {
                operation,
                generation,
                epoch,
                observed_at,
            }
        }
        MacosPausePilotEvent::Rejected { command, .. } => {
            let (operation, generation, epoch) = command_parts(command);
            WindowsPausePilotEvent::Failed {
                operation,
                generation,
                epoch,
                observed_at: batch_observed_at,
            }
        }
    })
}

fn convert_started(
    value: crate::macos_pause_pilot::MacosPausePilotStarted,
) -> WindowsPausePilotStarted {
    WindowsPausePilotStarted {
        physical_generation: value.physical_generation,
        observed_start_ms: value.observed_start_ms,
        monotonic_at: value.monotonic_at,
        unix_ms: value.unix_ms,
        accepted: WindowsPausePilotAcceptedCapture {
            settings: value.accepted.settings,
            range: WindowsPausePilotCaptureRange {
                origin_x: value.accepted.range.origin_x,
                origin_y: value.accepted.range.origin_y,
                width: value.accepted.range.width,
                height: value.accepted.range.height,
            },
        },
    }
}

fn convert_run(
    value: MacosSealedScreenRun,
) -> Result<WindowsSealedScreenRun, WindowsPausePilotChannelError> {
    let first_sequence = value
        .checkpoints
        .first()
        .map(|checkpoint| checkpoint.sequence)
        .ok_or(WindowsPausePilotChannelError::NativeTerminal)?;
    let last_sequence = value
        .checkpoints
        .last()
        .map(|checkpoint| checkpoint.sequence)
        .ok_or(WindowsPausePilotChannelError::NativeTerminal)?;
    let physical_generation = value.physical_generation;
    let checkpoints = value
        .checkpoints
        .into_iter()
        .map(|checkpoint: Checkpoint| WindowsSealedWgcCheckpoint {
            physical_generation,
            start_ms: checkpoint.facts.start_ms,
            end_ms: checkpoint.facts.end_ms,
            checkpoint,
        })
        .collect();
    Ok(WindowsSealedScreenRun {
        observed_start_ms: value.observed_start_ms,
        observed_end_ms: value.observed_end_ms,
        accepted: WindowsPausePilotAcceptedCapture {
            settings: value.accepted.settings,
            range: WindowsPausePilotCaptureRange {
                origin_x: value.accepted.range.origin_x,
                origin_y: value.accepted.range.origin_y,
                width: value.accepted.range.width,
                height: value.accepted.range.height,
            },
        },
        range: WindowsPausePilotCheckpointRange {
            first_physical_generation: physical_generation,
            last_physical_generation: physical_generation,
            first_checkpoint_sequence: first_sequence,
            last_checkpoint_sequence: last_sequence,
        },
        checkpoints,
    })
}

fn convert_audio(values: Vec<MacosSealedAudioRun>) -> Vec<WindowsSealedAudioRun> {
    values
        .into_iter()
        .map(|value| WindowsSealedAudioRun {
            stream: value.stream,
            source_generation: value.source_generation,
            artifact: value.artifact,
            bytes: value.bytes,
            sha256: value.sha256,
            media_duration_ms: value.media_duration_ms,
            native_ready_unix_ms: value.native_ready_unix_ms,
            native_ready_raw_ms: value.native_ready_raw_ms,
            raw_start_ms: value.raw_start_ms,
            raw_end_ms: value.raw_end_ms,
        })
        .collect()
}

fn command_parts(command: MacosPauseCommand) -> (WindowsPausePilotOperation, Option<u64>, u64) {
    match command {
        MacosPauseCommand::Pause {
            generation, epoch, ..
        } => (WindowsPausePilotOperation::Pause, Some(generation), epoch),
        MacosPauseCommand::Resume {
            generation, epoch, ..
        } => (WindowsPausePilotOperation::Resume, Some(generation), epoch),
        MacosPauseCommand::Stop { epoch, .. } => (WindowsPausePilotOperation::Stop, None, epoch),
    }
}

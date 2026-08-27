//! Native boundary facts shared by the private pause-pilot worker paths.

use std::time::Instant;

use super::{
    WindowsPausePilotAcceptedCapture, WindowsPausePilotCaptureRange, WindowsPausePilotCommand,
    WindowsPausePilotEvent, WindowsPausePilotOperation, WindowsPausePilotProfile,
    WindowsPausePilotStarted,
};
use crate::windows_wgc_run::ScreenRunIdentity;

/// Preserve only facts the native WGC owner actually accepted after opening.
pub(super) fn pilot_started(
    profile: &WindowsPausePilotProfile,
    identity: ScreenRunIdentity,
) -> Result<WindowsPausePilotStarted, ()> {
    let range = identity.accepted.range.ok_or(())?;
    if identity.accepted.settings.fps != profile.fps() as f32
        || identity.accepted.settings.width != range.width
        || identity.accepted.settings.height != range.height
    {
        return Err(());
    }
    Ok(WindowsPausePilotStarted {
        physical_generation: identity.physical_generation,
        observed_start_ms: identity.start_ms,
        monotonic_at: identity.started.monotonic_at,
        unix_ms: identity.started.unix_ms,
        accepted: WindowsPausePilotAcceptedCapture {
            settings: identity.accepted.settings,
            range: WindowsPausePilotCaptureRange {
                origin_x: range.origin_x,
                origin_y: range.origin_y,
                width: range.width,
                height: range.height,
            },
        },
    })
}

pub(super) fn failed(
    command: WindowsPausePilotCommand,
    observed_at: Instant,
) -> WindowsPausePilotEvent {
    let (operation, generation, epoch) = match command {
        WindowsPausePilotCommand::Pause { generation, epoch } => {
            (WindowsPausePilotOperation::Pause, Some(generation), epoch)
        }
        WindowsPausePilotCommand::Resume { generation, epoch } => {
            (WindowsPausePilotOperation::Resume, Some(generation), epoch)
        }
        WindowsPausePilotCommand::Stop { epoch } => (WindowsPausePilotOperation::Stop, None, epoch),
    };
    WindowsPausePilotEvent::Failed {
        operation,
        generation,
        epoch,
        observed_at,
    }
}

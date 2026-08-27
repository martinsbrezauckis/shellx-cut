//! Stop-dominant private command draining for the owned WGC worker.

use super::{
    WindowsPausePilotChannelError, WindowsPausePilotCommand, WindowsPausePilotCommandReceiver,
};

/// Keep command ordering unless a queued Stop is present. A terminal Stop
/// discards earlier Pause/Resume commands before they can touch native WGC.
pub(super) fn drain_stop_dominant(
    receiver: &WindowsPausePilotCommandReceiver,
    first: Option<WindowsPausePilotCommand>,
) -> Result<Vec<WindowsPausePilotCommand>, WindowsPausePilotChannelError> {
    let mut queued = first.into_iter().collect::<Vec<_>>();
    if matches!(queued.first(), Some(WindowsPausePilotCommand::Stop { .. })) {
        return Ok(queued);
    }
    loop {
        match receiver.try_recv()? {
            Some(command @ WindowsPausePilotCommand::Stop { .. }) => {
                queued.clear();
                queued.push(command);
                return Ok(queued);
            }
            Some(command) => queued.push(command),
            None => return Ok(queued),
        }
    }
}

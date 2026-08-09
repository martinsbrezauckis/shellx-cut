//! Bounded finite-command owner for passive recorder doctor probes.

use std::process::{Command, Output};
use std::time::Duration;

const DOCTOR_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// A doctor must remain best-effort, but a missing/blocked desktop service may
/// not leave its probe running. Tool absence, nonzero status, and ownership
/// failure all stay non-green evidence at the caller; this helper only owns
/// the finite process tree and its pipes.
pub(super) fn output(command: &mut Command, context: &str) -> Option<Output> {
    let control = cut_media::ffmpeg::OwnedProcessControl::bounded(DOCTOR_PROBE_TIMEOUT, || false);
    cut_media::ffmpeg::run_owned_command(command, &control, context).ok()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn doctor_probe_timeout_reaps_the_command() {
        let started = std::time::Instant::now();
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 60"]);
        assert!(output(&mut command, "test doctor probe").is_none());
        assert!(started.elapsed() < Duration::from_secs(4));
    }
}

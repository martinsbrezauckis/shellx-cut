//! A failed render whose stderr shows the HARDWARE encoder's own failure
//! (item 133).
//!
//! The software fallback (server `hw_fallback`) decides from the error text.
//! The out-of-memory error keeps only the last 600 characters of stderr and
//! the plain error the last 2000, so the encoder's own lines, which ffmpeg
//! prints early, could be cut away and the fallback missed its cause. This
//! classification runs first, on the FULL stderr, and puts the encoder's lines
//! at the front of the error cause, before the usual tail. A process the
//! system killed (a signal) is still the memory classifier's case.

use crate::hwencode_runtime::{classify_runtime_failure, hardware_lines};
use cut_core::{error_codes, CutError};
use std::process::ExitStatus;

/// Characters of ffmpeg's closing output kept after the encoder's lines.
const TAIL_CHARS: usize = 2000;

pub(super) fn hardware_failure_error(status: ExitStatus, stderr: &[u8]) -> Option<CutError> {
    if killed_by_signal(status) {
        return None;
    }
    let text = String::from_utf8_lossy(stderr);
    classify_runtime_failure(&text)?;
    let own = hardware_lines(&text, 6).join("\n");
    let tail: String = text
        .chars()
        .skip(text.chars().count().saturating_sub(TAIL_CHARS))
        .collect();
    Some(CutError::new(
        error_codes::FFMPEG,
        format!("ffmpeg exited with {status}"),
        format!("{own}\n...\n{tail}"),
    ))
}

#[cfg(unix)]
fn killed_by_signal(status: ExitStatus) -> bool {
    use std::os::unix::process::ExitStatusExt;
    status.signal().is_some()
}

#[cfg(not(unix))]
fn killed_by_signal(_status: ExitStatus) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exit_1() -> ExitStatus {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw(1 << 8)
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;
            ExitStatus::from_raw(1)
        }
    }

    /// The NVENC lines come first, then more than 2000 characters of other
    /// output ending in "Cannot allocate memory": the cause still carries the
    /// encoder's lines, so the fallback finds its reason.
    #[test]
    fn encoder_lines_survive_a_long_tail() {
        let mut stderr = String::from(
            "[h264_nvenc @ 0x5d1] OpenEncodeSessionEx failed: out of memory (10)\n\
             [h264_nvenc @ 0x5d1] InitializeEncoder failed: out of memory (10)\n",
        );
        for i in 0..80 {
            stderr.push_str(&format!(
                "[aac @ 0x9] queue input is backward in time {i}\n"
            ));
        }
        stderr.push_str("Error: Cannot allocate memory\nConversion failed!\n");
        let err = hardware_failure_error(exit_1(), stderr.as_bytes()).expect("hardware");
        assert!(
            err.cause
                .starts_with("[h264_nvenc @ 0x5d1] OpenEncodeSessionEx failed"),
            "{}",
            err.cause
        );
        assert_eq!(
            classify_runtime_failure(&err.cause),
            Some(crate::hwencode_runtime::HwRuntimeFailure::DeviceOutOfMemory)
        );
        assert!(
            hardware_failure_error(exit_1(), b"[fc#0 @ 0x1] Error reinitializing filters!")
                .is_none()
        );
    }

    /// A process the system killed stays with the memory classifier.
    #[cfg(unix)]
    #[test]
    fn a_signal_kill_is_not_a_hardware_failure() {
        use std::os::unix::process::ExitStatusExt;
        let killed = ExitStatus::from_raw(9);
        assert!(hardware_failure_error(
            killed,
            b"[h264_nvenc @ 0x1] InitializeEncoder failed: out of memory (10)"
        )
        .is_none());
    }
}

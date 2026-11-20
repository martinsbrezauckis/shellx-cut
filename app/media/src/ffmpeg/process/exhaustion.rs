//! Tell a render that ran out of memory or disk apart from any other ffmpeg
//! failure, so the job error says what happened and what to do instead of a
//! bare "ffmpeg exited with signal: 9".
//!
//! How each system shows it:
//! - Linux: the kernel's OOM killer (globally or inside the render's own
//!   cgroup scope) and systemd-oomd both stop ffmpeg with SIGKILL. The kernel
//!   counts its kills in `/proc/vmstat` `oom_kill`; systemd-oomd leaves no
//!   counter, so an unrequested SIGKILL while the host is nearly out of memory
//!   is reported as out of memory too, and any other unrequested SIGKILL as
//!   "stopped by the system", naming memory as the usual cause.
//! - macOS: the memory-pressure killer (jetsam) also sends SIGKILL and leaves
//!   nothing a process can read, so the same unrequested-SIGKILL rule applies
//!   and the message says the cause is not certain.
//! - Windows: there is no killer; an allocation fails inside ffmpeg
//!   ("Not enough space" is the C runtime's ENOMEM text) or the process ends
//!   with STATUS_NO_MEMORY / STATUS_COMMITMENT_LIMIT.
//!
//! Cancellation is not seen here: a kill this engine sends ends the poll loop
//! with the cancellation error before any exit status is classified.

use cut_core::{error_codes, CutError};
use std::process::ExitStatus;

/// Snapshot taken when the render starts, compared when it fails.
pub(crate) struct Watch {
    kernel_oom_kills: Option<u64>,
}

/// Why a render failed for lack of a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exhaustion {
    /// The kernel recorded an OOM kill while the render ran.
    KilledOutOfMemory,
    /// An unrequested SIGKILL while the host was almost out of memory/swap.
    KilledLowMemory,
    /// An unrequested SIGKILL with no memory evidence (macOS jetsam, or a
    /// user-space killer on Linux that leaves no counter).
    KilledBySystem,
    /// ffmpeg itself could not allocate memory (any system).
    AllocationFailed,
    /// The output volume filled up while writing.
    DiskFull,
}

impl Watch {
    pub(crate) fn start() -> Self {
        Self {
            kernel_oom_kills: kernel_oom_kills(),
        }
    }

    /// The resource error for a failed render, or None for an ordinary failure.
    pub(crate) fn classify(&self, status: ExitStatus, stderr: &[u8]) -> Option<CutError> {
        let text = String::from_utf8_lossy(stderr);
        let kind = classify(
            killed_by_sigkill(status),
            windows_memory_status(status),
            &text,
            self.kernel_oom_kills
                .zip(kernel_oom_kills())
                .is_some_and(|(before, after)| after > before),
            host_memory_nearly_exhausted(),
        )?;
        Some(exhaustion_error(kind, status, &text))
    }
}

/// Pure decision, separated from the readings so it can be tested anywhere.
pub(crate) fn classify(
    sigkill: bool,
    windows_no_memory: bool,
    stderr: &str,
    kernel_oom_kill_seen: bool,
    host_low_memory: bool,
) -> Option<Exhaustion> {
    let low = stderr.to_ascii_lowercase();
    if low.contains("no space left on device") || low.contains("disk quota exceeded") {
        return Some(Exhaustion::DiskFull);
    }
    if sigkill {
        return Some(if kernel_oom_kill_seen {
            Exhaustion::KilledOutOfMemory
        } else if host_low_memory {
            Exhaustion::KilledLowMemory
        } else {
            Exhaustion::KilledBySystem
        });
    }
    // "Cannot allocate memory" is ENOMEM on Linux and macOS, "Not enough
    // space" on the Windows C runtime. GPU encoders print their own "out of
    // memory" for video memory; that is a hardware encoder failure and is
    // sorted elsewhere, so it is not matched here.
    if windows_no_memory
        || low.contains("cannot allocate memory")
        || low.contains("not enough space")
    {
        return Some(Exhaustion::AllocationFailed);
    }
    None
}

/// The ways to make a render lighter, in the order they help most.
pub(crate) const LIGHTER_RENDER: &str = "close other programs that use a lot of memory, \
     or make the render lighter: keep Faster exports on (hardware encoder), choose HEVC \
     instead of H.264 for 8K, or render at a lower resolution";

fn exhaustion_error(kind: Exhaustion, status: ExitStatus, stderr: &str) -> CutError {
    let tail: String = stderr
        .chars()
        .skip(stderr.chars().count().saturating_sub(600))
        .collect();
    let (code, message, action) = match kind {
        Exhaustion::KilledOutOfMemory => (
            error_codes::OUT_OF_MEMORY,
            "the render ran out of memory: the system stopped ffmpeg (out-of-memory kill)",
            format!("Free memory, then render again: {LIGHTER_RENDER}."),
        ),
        Exhaustion::KilledLowMemory => (
            error_codes::OUT_OF_MEMORY,
            "the render ran out of memory: the system stopped ffmpeg while memory and swap \
             were almost full",
            format!("Free memory, then render again: {LIGHTER_RENDER}."),
        ),
        Exhaustion::KilledBySystem => (
            error_codes::OUT_OF_MEMORY,
            "the system stopped ffmpeg (signal 9) without a request from ShellX Cut; the usual \
             cause is that the computer ran out of memory",
            format!(
                "If memory was short, {LIGHTER_RENDER}. On macOS the system does not say why it \
                 stopped a program, so this cause is likely but not certain."
            ),
        ),
        Exhaustion::AllocationFailed => (
            error_codes::OUT_OF_MEMORY,
            "the render ran out of memory: ffmpeg could not allocate memory",
            format!("Free memory, then render again: {LIGHTER_RENDER}."),
        ),
        Exhaustion::DiskFull => (
            error_codes::NOT_ENOUGH_SPACE,
            "the render stopped because the disk it writes to is full",
            "Free space on the output drive or choose another output folder, then render again."
                .to_string(),
        ),
    };
    CutError::new(
        code,
        message,
        format!("ffmpeg exited with {status}; {tail}"),
    )
    .with_suggested_action(action)
}

#[cfg(unix)]
fn killed_by_sigkill(status: ExitStatus) -> bool {
    use std::os::unix::process::ExitStatusExt;
    // 137 = 128 + 9 when a shell wrapper reports the kill as an exit code.
    status.signal() == Some(9) || status.code() == Some(137)
}

#[cfg(not(unix))]
fn killed_by_sigkill(_status: ExitStatus) -> bool {
    false
}

/// STATUS_NO_MEMORY (0xC0000017) and STATUS_COMMITMENT_LIMIT (0xC000012D).
fn windows_memory_status(status: ExitStatus) -> bool {
    cfg!(windows)
        && matches!(
            status.code().map(|c| c as u32),
            Some(0xC000_0017) | Some(0xC000_012D)
        )
}

#[cfg(target_os = "linux")]
fn kernel_oom_kills() -> Option<u64> {
    let vmstat = std::fs::read_to_string("/proc/vmstat").ok()?;
    vmstat.lines().find_map(|l| {
        l.strip_prefix("oom_kill ")
            .and_then(|n| n.trim().parse().ok())
    })
}

#[cfg(not(target_os = "linux"))]
fn kernel_oom_kills() -> Option<u64> {
    None
}

/// True when available memory is under 3% of RAM and swap is over 90% used:
/// the state in which the kernel or systemd-oomd stops the biggest process.
#[cfg(target_os = "linux")]
fn host_memory_nearly_exhausted() -> bool {
    let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") else {
        return false;
    };
    let field = |name: &str| {
        meminfo.lines().find_map(|l| {
            l.strip_prefix(name)
                .and_then(|r| r.split_whitespace().next())
                .and_then(|n| n.parse::<u64>().ok())
        })
    };
    let (Some(total), Some(avail)) = (field("MemTotal:"), field("MemAvailable:")) else {
        return false;
    };
    let swap_total = field("SwapTotal:").unwrap_or(0);
    let swap_free = field("SwapFree:").unwrap_or(0);
    let swap_full = swap_total == 0 || swap_free * 10 < swap_total;
    avail * 100 < total * 3 && swap_full
}

#[cfg(not(target_os = "linux"))]
fn host_memory_nearly_exhausted() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Item 95: the record's two kills (SIGKILL, kernel cgroup OOM line, swap
    /// full) become out-of-memory errors; a plain encoder error stays None.
    #[test]
    fn sigkill_and_allocation_failures_are_memory() {
        assert_eq!(
            classify(true, false, "frame= 10", true, false),
            Some(Exhaustion::KilledOutOfMemory)
        );
        assert_eq!(
            classify(true, false, "frame= 10", false, true),
            Some(Exhaustion::KilledLowMemory)
        );
        assert_eq!(
            classify(true, false, "", false, false),
            Some(Exhaustion::KilledBySystem)
        );
        assert_eq!(
            classify(
                false,
                false,
                "[libx264 @ 0x1] malloc of size 1 failed\nError: Cannot allocate memory",
                false,
                false
            ),
            Some(Exhaustion::AllocationFailed)
        );
        assert_eq!(
            classify(false, true, "", false, false),
            Some(Exhaustion::AllocationFailed)
        );
        assert_eq!(
            classify(
                false,
                false,
                "av_interleaved_write_frame(): No space left on device",
                false,
                false
            ),
            Some(Exhaustion::DiskFull)
        );
        // NVENC's own "out of memory" is video memory: not this classifier's.
        assert_eq!(
            classify(
                false,
                false,
                "[h264_nvenc @ 0x1] InitializeEncoder failed: out of memory (10)",
                false,
                false
            ),
            None
        );
        assert_eq!(
            classify(false, false, "Invalid data found", false, false),
            None
        );
    }

    /// The message an agent and a person see names memory and the way out.
    #[cfg(unix)]
    #[test]
    fn sigkill_error_names_memory_and_the_way_out() {
        use std::os::unix::process::ExitStatusExt;
        let status = ExitStatus::from_raw(9); // terminated by signal 9
        let err = exhaustion_error(Exhaustion::KilledOutOfMemory, status, "frame=  3");
        assert_eq!(err.code, error_codes::OUT_OF_MEMORY);
        assert!(err.message.contains("ran out of memory"), "{}", err.message);
        let action = err.suggested_action.unwrap();
        assert!(
            action.contains("hardware encoder") && action.contains("HEVC"),
            "{action}"
        );
        assert!(err.cause.contains("signal: 9"), "{}", err.cause);
    }
}

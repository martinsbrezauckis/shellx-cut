//! process_start.rs — start an external program, retrying only the start
//! failure that is a known, short-lived race: the program file is busy.
//!
//! Unix (ETXTBSY, "Text file busy"): the kernel refuses to run a file that
//! any process still has open for writing. A program Cut (or a test) has just
//! written and closed can still have such a writer for a moment: another
//! thread's fork copied the descriptor table while the file was open, and that
//! child keeps the copy until its own exec closes it (close-on-exec). The race
//! ends by itself as soon as that child execs, so the start is retried briefly.
//!
//! Windows (ERROR_SHARING_VIOLATION): there is no fork and Rust's handles are
//! not inherited, so that race does not exist; the analogue is a just-written
//! .exe that another process (typically an antivirus scan) has open without
//! read/execute sharing. The same bounded retry covers it.
//!
//! Every other start error is returned at once, unchanged. After the last
//! retry the original busy error is returned, so a file that is really held
//! open for writing still fails plainly, in well under a second.

use std::io;
use std::time::Duration;

/// Waits between attempts after a busy start: 6 retries, 630 ms at most.
///
/// Go's `cmd/go` used 100 + 200 + 400 ms (0.7 s) for this race (go1.9
/// `cmd/go/internal/work/build.go`, golang.org/issue/3001); Go's `os/exec`
/// itself never retries (golang.org/issue/22315 is open). The window normally
/// lasts microseconds (until the other child's exec), longer only on a loaded
/// machine, so the first retry comes after 10 ms and the total stays below
/// Go's 0.7 s.
pub const BUSY_RETRY_DELAYS: [Duration; 6] = [
    Duration::from_millis(10),
    Duration::from_millis(20),
    Duration::from_millis(40),
    Duration::from_millis(80),
    Duration::from_millis(160),
    Duration::from_millis(320),
];

/// Whether a start failed only because the program file is busy (see the
/// module comment); every other error is a real failure.
pub fn is_program_busy(error: &io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(libc::ETXTBSY)
    }
    #[cfg(windows)]
    {
        error.raw_os_error() == Some(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = error;
        false
    }
}

/// Run `start` (a `Command::spawn`, `output` or `status` call), retrying it
/// after [`BUSY_RETRY_DELAYS`] while it fails with a busy program file. A
/// failed exec runs nothing, so repeating the start is safe.
pub fn retry_busy_start<T>(start: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    retry_busy_start_with(&BUSY_RETRY_DELAYS, std::thread::sleep, start)
}

/// [`retry_busy_start`] with explicit waits and sleep (tests).
pub(crate) fn retry_busy_start_with<T>(
    delays: &[Duration],
    mut sleep: impl FnMut(Duration),
    mut start: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    let mut delays = delays.iter();
    loop {
        match start() {
            Err(error) if is_program_busy(&error) => match delays.next() {
                Some(delay) => sleep(*delay),
                None => return Err(error),
            },
            result => return result,
        }
    }
}

#[cfg(test)]
#[path = "process_start_tests.rs"]
mod tests;

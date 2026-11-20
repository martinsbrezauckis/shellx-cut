//! Notice that the desktop shell which started this engine is gone.
//!
//! The shell stops its engine on a normal close, but a termination signal, a
//! crash or a forced kill of the shell gives it no chance to. Without this the
//! engine keeps running with nobody to close it, and the next shell finds it and
//! reuses it. The shell therefore names itself in `SHELLX_CUT_PARENT_PID`, and the
//! engine shuts down through its ordinary graceful path (which also removes the
//! `engine.addr` discovery file) when that process disappears.
//!
//! A standalone `cutd serve` has no such variable and is never stopped by this.

use std::time::Duration;

/// Set by the desktop shell, on every system, for the engine it spawns.
pub(crate) const ENV_PARENT_PID: &str = "SHELLX_CUT_PARENT_PID";

/// How often the shell is checked. Short enough that a closed shell takes its
/// engine with it within a moment, long enough to cost nothing.
const POLL: Duration = Duration::from_millis(250);

/// The shell to watch: the pid in the environment, accepted only when that
/// process really is this engine's parent. A value inherited from some other
/// terminal or script (a stale export) must not shut a standalone engine down.
fn watched_parent(env_value: Option<&str>, actual_parent: Option<u32>) -> Option<u32> {
    let named: u32 = env_value?.trim().parse().ok()?;
    (named != 0 && actual_parent == Some(named)).then_some(named)
}

/// True when `named` is no longer this process's parent.
#[cfg(any(unix, test))]
fn parent_changed(named: u32, actual_parent: Option<u32>) -> bool {
    actual_parent != Some(named)
}

/// The pid of this process's parent, as the platform reports it.
#[cfg(unix)]
fn actual_parent_pid() -> Option<u32> {
    // SAFETY: getppid has no preconditions and cannot fail.
    let ppid = unsafe { libc::getppid() };
    u32::try_from(ppid).ok()
}

/// The parent of this process from a process snapshot (the same snapshot the job
/// tree code takes), or None when the entry cannot be found.
#[cfg(windows)]
fn actual_parent_pid() -> Option<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let me = std::process::id();
    // SAFETY: the snapshot handle is closed on every path; the entry struct is
    // zeroed with its size field set as the API requires.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = None;
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more {
            if entry.th32ProcessID == me {
                found = Some(entry.th32ParentProcessID);
                break;
            }
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        found
    }
}

/// Windows keeps the parent id in a process's record after the parent exits, so
/// the pid alone cannot say the parent is gone: ask whether that process still
/// runs.
#[cfg(windows)]
fn process_is_running(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // SAFETY: the handle is closed before returning; the exit-code out-pointer is
    // a local.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            // "No such process" means gone; any other refusal (access denied)
            // means it exists.
            return GetLastError() != ERROR_INVALID_PARAMETER;
        }
        let mut exit_code = 0u32;
        let running =
            GetExitCodeProcess(process, &mut exit_code) != 0 && exit_code == STILL_ACTIVE as u32;
        CloseHandle(process);
        running
    }
}

#[cfg(unix)]
fn parent_gone(named: u32) -> bool {
    parent_changed(named, actual_parent_pid())
}

#[cfg(windows)]
fn parent_gone(named: u32) -> bool {
    !process_is_running(named)
}

#[cfg(not(any(unix, windows)))]
fn parent_gone(_named: u32) -> bool {
    false
}

#[cfg(not(any(unix, windows)))]
fn actual_parent_pid() -> Option<u32> {
    None
}

/// The shell this engine follows, decided once, at start. Deciding it later
/// (when the server's shutdown future is first polled) lost a shell that died
/// in between: on a loaded machine the engine had already been reparented,
/// its parent no longer matched the named one, and it then never stopped.
fn followed_shell() -> Option<u32> {
    static FOLLOWED: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *FOLLOWED.get_or_init(|| {
        let env_value = std::env::var(ENV_PARENT_PID).ok();
        watched_parent(env_value.as_deref(), actual_parent_pid())
    })
}

/// Record which shell this engine follows. Call first thing at start.
pub(crate) fn capture() {
    let _ = followed_shell();
}

/// Resolves when the desktop shell that started this engine is gone. Pending
/// forever for an engine that was not started by a shell.
pub(crate) async fn wait_for_parent_exit() {
    let Some(named) = followed_shell() else {
        std::future::pending::<()>().await;
        return;
    };
    while !parent_gone(named) {
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_real_parent_is_watched() {
        assert_eq!(watched_parent(Some("42"), Some(42)), Some(42));
        assert_eq!(watched_parent(Some(" 42\n"), Some(42)), Some(42));
        // A stale export naming some other process is ignored, not obeyed.
        assert_eq!(watched_parent(Some("42"), Some(7)), None);
        assert_eq!(watched_parent(Some("42"), None), None);
        assert_eq!(watched_parent(None, Some(42)), None);
        assert_eq!(watched_parent(Some("not-a-pid"), Some(42)), None);
        assert_eq!(watched_parent(Some("0"), Some(0)), None);
    }

    #[test]
    fn a_changed_parent_means_the_shell_is_gone() {
        assert!(!parent_changed(42, Some(42)));
        // Reparented to init or a subreaper.
        assert!(parent_changed(42, Some(1)));
        assert!(parent_changed(42, None));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_engine_without_a_shell_never_stops_itself() {
        // No SHELLX_CUT_PARENT_PID in this process: the wait must stay pending.
        if std::env::var_os(ENV_PARENT_PID).is_some() {
            return;
        }
        let waited = tokio::time::timeout(Duration::from_millis(600), wait_for_parent_exit()).await;
        assert!(waited.is_err(), "a standalone engine must keep running");
    }
}

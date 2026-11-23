//! No helper outlives the engine that started it.
//!
//! The engine starts long-running helpers (ffmpeg for exports and proxies,
//! recorder helpers, the captions Python, installers). Most run in their own
//! process group so a cancel can stop the whole tree. When the engine is
//! killed (kill -9, a crash, Force Quit) no engine code runs, and nothing on
//! Linux or macOS stopped those groups: an export's ffmpeg kept writing its
//! hidden staging file after the next start had said the files were removed.
//!
//! - Every helper carries this run's mark in its environment
//!   (`SHELLX_CUT_ENGINE_RUN=<pid>-<start>-<nonce>`), inherited by every
//!   descendant. A mark is never a bare pid or a program name.
//! - Linux and macOS: at start the engine also starts a small watcher, itself
//!   (`cutd __lifeline`), a session leader of its own (setsid, no controlling
//!   terminal), holding the read end of a pipe
//!   whose only writer is the engine. The kernel closes that pipe when the
//!   engine dies however it dies, the watcher reads end-of-file, and it stops
//!   every process that carries the engine's mark (TERM, then KILL after 2 s).
//!   This is the same for every helper, whatever its process group.
//! - Windows: the engine puts itself in a Job Object that kills every process
//!   in it when its last handle closes, which the kernel does when the engine
//!   dies. Every child is in the job from its creation.
//! - The next start stops any survivor of a dead run by its mark before it
//!   removes that run's unfinished files (see `jobs::interrupted`), and then
//!   whatever else still carries a dead run's mark (`runs`), in case the
//!   watcher was killed too.

use std::time::Duration;

pub(crate) const ENV_ENGINE_RUN: &str = "SHELLX_CUT_ENGINE_RUN";
const LIFELINE_ARG: &str = "__lifeline";
const GRACE: Duration = Duration::from_secs(2);

mod runs;
pub(crate) use runs::stop_dead_runs;

#[cfg(unix)]
#[path = "engine_lifeline/scan_unix.rs"]
mod scan;

/// This run's mark, when the engine set one (a serving engine does).
pub(crate) fn this_run_mark() -> Option<String> {
    std::env::var(ENV_ENGINE_RUN)
        .ok()
        .filter(|mark| valid_mark(mark))
}

pub(crate) fn valid_mark(mark: &str) -> bool {
    !mark.is_empty()
        && mark.len() <= 96
        && mark
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == ':')
}

/// Run as the watcher when started as one; never returns then.
pub(crate) fn run_if_requested() {
    #[cfg(target_os = "linux")]
    crate::linux_microphone_probe::run_if_requested();
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new(LIFELINE_ARG)) {
        return;
    }
    #[cfg(unix)]
    {
        use std::io::Read;
        let mut sink = [0u8; 64];
        // Blocks until the engine's write end closes: its exit or its death.
        while matches!(std::io::stdin().read(&mut sink), Ok(n) if n > 0) {}
        if let Some(mark) = this_run_mark() {
            stop_marked(&mark);
        }
    }
    std::process::exit(0);
}

/// Mark this run and make sure its helpers die with it. Call once, first
/// thing in `serve`, before any thread or child exists.
pub(crate) fn begin(serving: bool) {
    if !serving {
        return;
    }
    // Which desktop shell this engine follows is decided now, before anything
    // can be slow (see desktop_parent).
    crate::desktop_parent::capture();
    let identity = record_recovery::CaptureStart::new("engine-run", 0).owner_identity;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let mark: String = format!("{}-{identity}-{nonce:08x}", std::process::id())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == ':')
        .take(96)
        .collect();
    // SAFETY: called before the Tokio runtime or any other thread starts.
    std::env::set_var(ENV_ENGINE_RUN, &mark);
    runs::record(&mark, &identity);
    if let Err(error) = arm() {
        eprintln!("[cutd] helpers may outlive a killed engine: {error}");
    }
}

#[cfg(unix)]
fn arm() -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg(LIFELINE_ARG)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // A real session leader with no controlling terminal: closing the terminal
    // an engine was started from (its hangup goes to that terminal's session)
    // or ending the engine's session or process group never reaches the
    // watcher, which then still stops the helpers. setsid cannot fail here:
    // the forked child is never a group leader.
    // SAFETY: setsid is async-signal-safe and runs between fork and exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    // The write end stays open for the engine's whole life. std creates it
    // close-on-exec, so no helper inherits it and keeps the watcher waiting.
    let writer = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("no lifeline pipe"))?;
    std::mem::forget(writer);
    std::mem::forget(child);
    Ok(())
}

#[cfg(windows)]
fn arm() -> std::io::Result<()> {
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: plain Win32 calls on a handle this function owns; the job
    // handle is deliberately kept open until the process ends.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(std::io::Error::from_raw_os_error(GetLastError() as i32));
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const _,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == 0
            || AssignProcessToJobObject(job, GetCurrentProcess()) == 0
        {
            return Err(std::io::Error::from_raw_os_error(GetLastError() as i32));
        }
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn arm() -> std::io::Result<()> {
    Ok(())
}

/// Stop every process that carries `mark` (never this process): TERM, then
/// KILL whatever is still there after a short grace. Returns how many there
/// were. On Windows the engine's Job Object already ended them; nothing to do.
pub(crate) fn stop_marked(mark: &str) -> usize {
    if !valid_mark(mark) {
        return 0;
    }
    #[cfg(unix)]
    {
        let found = scan::marked(ENV_ENGINE_RUN, mark);
        for process in &found {
            scan::signal_if_marked(process, ENV_ENGINE_RUN, mark, libc::SIGTERM);
        }
        let deadline = std::time::Instant::now() + GRACE;
        while std::time::Instant::now() < deadline
            && found
                .iter()
                .any(|process| scan::still_marked(process, ENV_ENGINE_RUN, mark))
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        for process in &found {
            scan::signal_if_marked(process, ENV_ENGINE_RUN, mark, libc::SIGKILL);
        }
        found.len()
    }
    #[cfg(not(unix))]
    {
        let _ = GRACE;
        0
    }
}

#[cfg(all(test, unix))]
#[path = "engine_lifeline/tests.rs"]
mod tests;

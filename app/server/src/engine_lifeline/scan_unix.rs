//! Find this user's processes by an environment entry, on Linux (`/proc`)
//! and macOS (`KERN_PROCARGS2`). Another user's processes cannot be read and
//! are never touched. macOS does not show the environment of Apple's own
//! tools (`/bin/sh`, `/usr/bin/caffeinate`), so those are never found here:
//! the engine's one long-running Apple tool, `caffeinate`, ends by itself
//! with the engine (`-w`), and an `sh` wrapper ends with the helper it runs.

/// A marked process. Linux holds a pidfd across both cleanup signals.
pub(super) struct MarkedProcess {
    pub(super) pid: i32,
    #[cfg(target_os = "linux")]
    pidfd: Option<std::os::fd::OwnedFd>,
    #[cfg(target_os = "macos")]
    start_time: Option<(u64, u64)>,
}

/// Processes (not this process) whose environment holds `name=value`.
pub(super) fn marked(name: &str, value: &str) -> Vec<MarkedProcess> {
    let me = std::process::id() as i32;
    all_pids()
        .into_iter()
        .filter(|pid| *pid > 1 && *pid != me)
        .filter_map(|pid| hold_if_marked(pid, name, value))
        .collect()
}

/// Whether `pid` is alive and its environment holds `name=value`.
pub(super) fn carries(pid: i32, name: &str, value: &str) -> bool {
    let wanted = format!("{name}={value}");
    environment(pid).is_some_and(|entries| entries.iter().any(|entry| entry == wanted.as_bytes()))
}

#[cfg(target_os = "linux")]
fn hold_if_marked(pid: i32, name: &str, value: &str) -> Option<MarkedProcess> {
    use std::os::fd::FromRawFd;
    // Open before checking the mark. The descriptor refers to this process
    // even if its numeric PID is reused after the check.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    let pidfd = (fd >= 0).then(|| unsafe { std::os::fd::OwnedFd::from_raw_fd(fd as i32) });
    // Older kernels and resource exhaustion can deny pidfd_open. Preserve
    // existing cleanup there, with the documented numeric-signal race.
    carries(pid, name, value).then_some(MarkedProcess { pid, pidfd })
}

#[cfg(target_os = "macos")]
fn hold_if_marked(pid: i32, name: &str, value: &str) -> Option<MarkedProcess> {
    let start_time = process_start_time(pid);
    carries(pid, name, value).then_some(MarkedProcess { pid, start_time })
}

#[cfg(target_os = "macos")]
fn process_start_time(pid: i32) -> Option<(u64, u64)> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    (read == size).then_some((info.pbi_start_tvsec, info.pbi_start_tvusec))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn hold_if_marked(_pid: i32, _name: &str, _value: &str) -> Option<MarkedProcess> {
    None
}

pub(super) fn still_marked(process: &MarkedProcess, name: &str, value: &str) -> bool {
    #[cfg(target_os = "linux")]
    if let Some(pidfd) = &process.pidfd {
        if unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                std::os::fd::AsRawFd::as_raw_fd(pidfd),
                0,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        } != 0
        {
            return false;
        }
    }
    #[cfg(target_os = "macos")]
    if process
        .start_time
        .is_some_and(|start| process_start_time(process.pid) != Some(start))
    {
        return false;
    }
    carries(process.pid, name, value)
}

/// Recheck the mark, then address the held identity where the OS supports it.
pub(super) fn signal_if_marked(process: &MarkedProcess, name: &str, value: &str, signal: i32) {
    if !still_marked(process, name, value) {
        return;
    }
    #[cfg(target_os = "linux")]
    if let Some(pidfd) = &process.pidfd {
        unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                std::os::fd::AsRawFd::as_raw_fd(pidfd),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            );
        }
    } else {
        unsafe { libc::kill(process.pid, signal) };
    }
    // macOS has no pidfd. The start time and mark are checked immediately
    // above; a helper can still exit and have its PID reused before kill.
    #[cfg(target_os = "macos")]
    unsafe {
        libc::kill(process.pid, signal);
    }
}

#[cfg(target_os = "linux")]
fn all_pids() -> Vec<i32> {
    std::fs::read_dir("/proc")
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn environment(pid: i32) -> Option<Vec<Vec<u8>>> {
    // A zombie has an empty environ; it is already dead.
    let bytes = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    Some(split_nul(&bytes))
}

#[cfg(target_os = "macos")]
fn all_pids() -> Vec<i32> {
    // SAFETY: the first call sizes the buffer; the second fills at most it.
    unsafe {
        let count = libc::proc_listallpids(std::ptr::null_mut(), 0);
        if count <= 0 {
            return Vec::new();
        }
        let mut pids = vec![0i32; count as usize + 64];
        let bytes = (pids.len() * std::mem::size_of::<i32>()) as libc::c_int;
        let filled = libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes);
        if filled <= 0 {
            return Vec::new();
        }
        pids.truncate((filled as usize).min(pids.len()));
        pids.retain(|pid| *pid > 0);
        pids
    }
}

#[cfg(target_os = "macos")]
fn environment(pid: i32) -> Option<Vec<Vec<u8>>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size: libc::size_t = 0;
    // SAFETY: standard two-step sysctl; the buffer is sized by the first call.
    unsafe {
        if libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
            || size < 4
        {
            return None;
        }
        let mut buffer = vec![0u8; size];
        if libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buffer.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return None;
        }
        buffer.truncate(size);
        Some(procargs_environment(&buffer))
    }
}

/// `KERN_PROCARGS2`: argc, the executable path, padding NULs, argc argument
/// strings, then the environment strings.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) fn procargs_environment(buffer: &[u8]) -> Vec<Vec<u8>> {
    if buffer.len() < 4 {
        return Vec::new();
    }
    let argc = i32::from_ne_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]).max(0) as usize;
    let mut rest = &buffer[4..];
    // Executable path, then its padding.
    let path_end = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
    rest = &rest[path_end..];
    let start = rest.iter().position(|b| *b != 0).unwrap_or(rest.len());
    rest = &rest[start..];
    let mut strings = split_nul(rest).into_iter();
    for _ in 0..argc {
        strings.next();
    }
    strings.take_while(|entry| !entry.is_empty()).collect()
}

fn split_nul(bytes: &[u8]) -> Vec<Vec<u8>> {
    bytes.split(|b| *b == 0).map(<[u8]>::to_vec).collect()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn all_pids() -> Vec<i32> {
    Vec::new()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn environment(_pid: i32) -> Option<Vec<Vec<u8>>> {
    None
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    fn checked_helper_exit_cannot_signal_an_injected_reused_number() {
        let mark = format!("{}-pidfd-test", std::process::id());
        let mut helper = Command::new("sleep")
            .arg("60")
            .env("SHELLX_CUT_ENGINE_RUN", &mark)
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let mut held = None;
        for _ in 0..50 {
            held = hold_if_marked(helper.id() as i32, "SHELLX_CUT_ENGINE_RUN", &mark);
            if held.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let mut held = held.expect("helper mark is visible");
        assert!(held.pidfd.is_some(), "pidfd is available on this Linux box");
        helper.kill().unwrap();
        helper.wait().unwrap();
        let mut replacement = Command::new("sleep")
            .arg("60")
            .env("SHELLX_CUT_ENGINE_RUN", &mark)
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        for _ in 0..50 {
            if carries(replacement.id() as i32, "SHELLX_CUT_ENGINE_RUN", &mark) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(carries(
            replacement.id() as i32,
            "SHELLX_CUT_ENGINE_RUN",
            &mark
        ));
        // Inject numeric reuse deterministically. The held descriptor still
        // names the exited helper, while the number and mark now name a live
        // replacement. A checked numeric kill would terminate replacement.
        held.pid = replacement.id() as i32;
        signal_if_marked(&held, "SHELLX_CUT_ENGINE_RUN", &mark, libc::SIGTERM);
        std::thread::sleep(std::time::Duration::from_millis(100));
        let survived = replacement.try_wait().unwrap().is_none();
        if survived {
            replacement.kill().unwrap();
        }
        replacement.wait().unwrap();
        assert!(survived, "the replacement received the helper's signal");
    }
}

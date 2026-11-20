//! Find this user's processes by an environment entry, on Linux (`/proc`)
//! and macOS (`KERN_PROCARGS2`). Another user's processes cannot be read and
//! are never touched. macOS does not show the environment of Apple's own
//! tools (`/bin/sh`, `/usr/bin/caffeinate`), so those are never found here:
//! the engine's one long-running Apple tool, `caffeinate`, ends by itself
//! with the engine (`-w`), and an `sh` wrapper ends with the helper it runs.

/// Process ids (not this process) whose environment holds `name=value`.
pub(super) fn marked(name: &str, value: &str) -> Vec<i32> {
    let me = std::process::id() as i32;
    all_pids()
        .into_iter()
        .filter(|pid| *pid > 1 && *pid != me && carries(*pid, name, value))
        .collect()
}

/// Whether `pid` is alive and its environment holds `name=value`.
pub(super) fn carries(pid: i32, name: &str, value: &str) -> bool {
    let wanted = format!("{name}={value}");
    environment(pid).is_some_and(|entries| entries.iter().any(|entry| entry == wanted.as_bytes()))
}

/// Send `signal` only if `pid` still carries the mark: a pid that ended and
/// was reused by an unrelated process is left alone.
pub(super) fn signal_if_marked(pid: i32, name: &str, value: &str, signal: i32) {
    if carries(pid, name, value) {
        // SAFETY: kill(2) with a checked pid and a valid signal number.
        unsafe {
            libc::kill(pid, signal);
        }
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

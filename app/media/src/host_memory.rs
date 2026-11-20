//! How much memory this computer has and how much a new render can use now.
//!
//! - Linux: `/proc/meminfo` MemTotal / MemAvailable (the kernel's own estimate
//!   of memory a new program can get without swapping), plus swap.
//! - macOS: `sysctl hw.memsize` and `vm_stat` (free + inactive + speculative +
//!   purgeable pages). macOS also compresses memory, so a render can use more
//!   than this before it is stopped; the number is a floor, not a limit.
//! - Windows: GlobalMemoryStatusEx (ullTotalPhys / ullAvailPhys, and the page
//!   file in ullTotalPageFile).
//!
//! Every reading is best effort: None means "not known", never "zero".

/// Physical memory and what a new program can take now, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct HostMemory {
    pub total_bytes: u64,
    pub available_bytes: u64,
    /// Swap or page file size beyond physical memory (0 when none or unknown).
    pub swap_bytes: u64,
}

#[cfg(target_os = "linux")]
pub fn host_memory() -> Option<HostMemory> {
    parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

/// Pure parser for `/proc/meminfo` (kB values).
pub fn parse_meminfo(text: &str) -> Option<HostMemory> {
    let field = |name: &str| {
        text.lines().find_map(|l| {
            l.strip_prefix(name)
                .and_then(|r| r.split_whitespace().next())
                .and_then(|n| n.parse::<u64>().ok())
                .map(|kb| kb.saturating_mul(1024))
        })
    };
    Some(HostMemory {
        total_bytes: field("MemTotal:")?,
        available_bytes: field("MemAvailable:")?,
        swap_bytes: field("SwapTotal:").unwrap_or(0),
    })
}

#[cfg(target_os = "macos")]
pub fn host_memory() -> Option<HostMemory> {
    use std::process::Command;
    let run = |prog: &str, args: &[&str]| -> Option<String> {
        let mut command = Command::new(prog);
        command.args(args);
        let control =
            crate::ffmpeg::OwnedProcessControl::bounded(std::time::Duration::from_secs(5), || {
                false
            });
        let out = crate::ffmpeg::run_owned_command(&mut command, &control, "read memory").ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let total: u64 = run("/usr/sbin/sysctl", &["-n", "hw.memsize"])?
        .trim()
        .parse()
        .ok()?;
    let available = parse_vm_stat(&run("/usr/bin/vm_stat", &[])?)?;
    Some(HostMemory {
        total_bytes: total,
        available_bytes: available.min(total),
        // macOS grows its swap files on demand; there is no fixed size to add.
        swap_bytes: 0,
    })
}

/// Pure parser for `vm_stat`: (free + inactive + speculative + purgeable) x page size.
pub fn parse_vm_stat(text: &str) -> Option<u64> {
    let page: u64 = text
        .lines()
        .next()?
        .split("page size of ")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let pages = |name: &str| -> u64 {
        text.lines()
            .find_map(|l| l.strip_prefix(name))
            .and_then(|r| r.trim().trim_end_matches('.').parse::<u64>().ok())
            .unwrap_or(0)
    };
    let free = pages("Pages free:")
        + pages("Pages inactive:")
        + pages("Pages speculative:")
        + pages("Pages purgeable:");
    Some(free.saturating_mul(page))
}

#[cfg(windows)]
pub fn host_memory() -> Option<HostMemory> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: MEMORYSTATUSEX is plain data; dwLength must be set before the
    // call, which fills the rest of the struct we own.
    unsafe {
        let mut status: MEMORYSTATUSEX = std::mem::zeroed();
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut status) == 0 {
            return None;
        }
        Some(HostMemory {
            total_bytes: status.ullTotalPhys,
            available_bytes: status.ullAvailPhys,
            swap_bytes: status.ullTotalPageFile.saturating_sub(status.ullTotalPhys),
        })
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn host_memory() -> Option<HostMemory> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meminfo_and_vm_stat_parse() {
        let m = parse_meminfo(
            "MemTotal:       64592896 kB\nMemFree:  1 kB\nMemAvailable:   48258000 kB\nSwapTotal:       8388604 kB\n",
        )
        .unwrap();
        assert_eq!(m.total_bytes, 64592896 * 1024);
        assert_eq!(m.available_bytes, 48258000 * 1024);
        assert_eq!(m.swap_bytes, 8388604 * 1024);
        assert_eq!(parse_meminfo("MemFree: 1 kB\n"), None);

        let vm = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\n\
                  Pages free:                               100.\n\
                  Pages active:                             999.\n\
                  Pages inactive:                            50.\n\
                  Pages speculative:                         10.\n\
                  Pages purgeable:                            5.\n";
        assert_eq!(parse_vm_stat(vm), Some(165 * 16384));
    }
}

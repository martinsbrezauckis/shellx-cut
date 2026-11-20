//! How much work a render's process tree is doing, for the stall watch.
//!
//! Output time alone cannot tell a stuck ffmpeg from a busy one: a reversed
//! clip, or a clip far into its source (renders decode every source from its
//! start), keeps the output still for minutes while ffmpeg decodes at full
//! speed. USER processor time can: a stuck ffmpeg (a filter deadlock, or a
//! process the kernel holds at its cgroup memory limit) does none of its own
//! work. System time is not counted: a process held at MemoryHigh can spend
//! several % of a core in the kernel's memory reclaim while making no
//! progress at all (seen on the box: 2.7 % of a core, output frozen 15 min).
//!
//! Per system:
//! - Linux: the user time of every process in the render's process group
//!   (`/proc/<pid>/stat`), and the `high` counter of the leader's cgroup
//!   (`memory.events`): it rises each time the kernel throttles the cgroup
//!   because it is over MemoryHigh.
//! - macOS: the user time of every process in the group (`proc_listpgrppids`
//!   + `proc_pid_rusage`, Mach time converted to ns).
//! - Windows: the Job Object's total user time.
//!
//! Reads that fail give `None`; the watch then never ends a render on its own
//! (the operation deadline still does).

use super::tree::ProcessTree;
use std::time::Duration;

/// One reading of the render tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Usage {
    /// User processor time used so far by every process of the render.
    pub(super) cpu: Duration,
    /// Linux: times the render's cgroup was throttled for being over its
    /// memory limit (MemoryHigh); `None` elsewhere or when not readable.
    pub(super) memory_high_events: Option<u64>,
    /// Linux: the render cgroup's memory use and limit in bytes, when set.
    pub(super) memory: Option<(u64, u64)>,
}

pub(super) fn usage(tree: &ProcessTree) -> Option<Usage> {
    platform::usage(tree)
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{ProcessTree, Usage};
    use std::time::Duration;

    pub(super) fn usage(tree: &ProcessTree) -> Option<Usage> {
        let group = tree.group();
        let ticks = clock_ticks()?;
        let mut total_ticks = 0u64;
        let mut seen = false;
        for entry in std::fs::read_dir("/proc").ok()?.flatten() {
            let name = entry.file_name();
            let Some(pid) = name
                .to_str()
                .filter(|n| n.bytes().all(|b| b.is_ascii_digit()))
            else {
                continue;
            };
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                continue; // the process ended between the listing and the read
            };
            if let Some((pgrp, cpu)) = parse_stat(&stat) {
                if pgrp == group {
                    total_ticks = total_ticks.saturating_add(cpu);
                    seen = true;
                }
            }
        }
        if !seen {
            return None;
        }
        let cgroup = std::fs::read_to_string(format!("/proc/{group}/cgroup"))
            .ok()
            .and_then(|text| cgroup_dir(&text));
        let read = |file: &str| {
            cgroup
                .as_ref()
                .and_then(|dir| std::fs::read_to_string(format!("{dir}/{file}")).ok())
        };
        let memory_high_events = read("memory.events").and_then(|t| high_events(&t));
        let current = read("memory.current").and_then(|t| t.trim().parse::<u64>().ok());
        let high = read("memory.high").and_then(|t| t.trim().parse::<u64>().ok());
        Some(Usage {
            cpu: Duration::from_nanos(total_ticks.saturating_mul(1_000_000_000 / ticks)),
            memory_high_events,
            memory: current.zip(high),
        })
    }

    fn clock_ticks() -> Option<u64> {
        let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        u64::try_from(ticks).ok().filter(|t| *t > 0)
    }

    /// `(process group, utime in clock ticks)` from `/proc/<pid>/stat`. The
    /// command name may hold spaces and parentheses, so fields are counted
    /// after its LAST `)`: state, ppid, pgrp, ..., utime (12th).
    pub(super) fn parse_stat(stat: &str) -> Option<(i32, u64)> {
        let rest = stat.rsplit_once(')')?.1;
        let fields: Vec<&str> = rest.split_whitespace().collect();
        let pgrp = fields.get(2)?.parse().ok()?;
        let utime: u64 = fields.get(11)?.parse().ok()?;
        Some((pgrp, utime))
    }

    /// The cgroup v2 directory of a process from its `/proc/<pid>/cgroup`.
    pub(super) fn cgroup_dir(text: &str) -> Option<String> {
        let path = text.lines().find_map(|l| l.strip_prefix("0::"))?.trim();
        Some(format!("/sys/fs/cgroup{path}"))
    }

    pub(super) fn high_events(text: &str) -> Option<u64> {
        text.lines()
            .find_map(|l| l.strip_prefix("high "))
            .and_then(|n| n.trim().parse().ok())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{ProcessTree, Usage};
    use std::time::Duration;

    pub(super) fn usage(tree: &ProcessTree) -> Option<Usage> {
        let group = tree.group();
        let mut pids = vec![0 as libc::pid_t; 256];
        // libproc's proc_listpgrppids returns the NUMBER of pids (it divides
        // proc_listpids' byte count by the pid size itself). Reading it as a
        // byte count found no process, so no reading was ever made and the
        // stall watch never ended a render on macOS (seen on an Apple silicon Mac).
        let listed = unsafe {
            libc::proc_listpgrppids(
                group,
                pids.as_mut_ptr().cast(),
                (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int,
            )
        };
        let count = usize::try_from(listed).unwrap_or(0).min(pids.len());
        let mut members: Vec<libc::pid_t> =
            pids[..count].iter().copied().filter(|p| *p > 0).collect();
        if !members.contains(&group) {
            members.push(group); // the group leader is the render's ffmpeg
        }
        let mut mach_total = 0u64;
        let mut seen = false;
        for &pid in &members {
            let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
            let rc = unsafe {
                libc::proc_pid_rusage(
                    pid,
                    libc::RUSAGE_INFO_V2,
                    (&mut info as *mut libc::rusage_info_v2).cast(),
                )
            };
            if rc == 0 {
                mach_total = mach_total.saturating_add(info.ri_user_time);
                seen = true;
            }
        }
        if !seen {
            return None;
        }
        // rusage times are Mach absolute time units (1 ns on Intel, not on
        // Apple silicon): convert with the timebase.
        #[allow(deprecated)]
        let (numer, denom) = unsafe {
            let mut base: libc::mach_timebase_info = std::mem::zeroed();
            if libc::mach_timebase_info(&mut base) != 0 || base.denom == 0 {
                return None;
            }
            (u128::from(base.numer), u128::from(base.denom))
        };
        let nanos = u128::from(mach_total) * numer / denom;
        Some(Usage {
            cpu: Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX)),
            memory_high_events: None,
            memory: None,
        })
    }
}

#[cfg(windows)]
mod platform {
    use super::{ProcessTree, Usage};
    use std::time::Duration;

    pub(super) fn usage(tree: &ProcessTree) -> Option<Usage> {
        use windows_sys::Win32::System::JobObjects::{
            JobObjectBasicAccountingInformation, QueryInformationJobObject,
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
        };
        let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
        let ok = unsafe {
            QueryInformationJobObject(
                tree.job(),
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return None;
        }
        // 100-nanosecond units.
        let hundreds = u64::try_from(info.TotalUserTime).unwrap_or(0);
        Some(Usage {
            cpu: Duration::from_nanos(hundreds.saturating_mul(100)),
            memory_high_events: None,
            memory: None,
        })
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod platform {
    use super::{ProcessTree, Usage};

    pub(super) fn usage(_tree: &ProcessTree) -> Option<Usage> {
        None
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::platform::{cgroup_dir, high_events, parse_stat};

    #[test]
    fn stat_fields_are_counted_after_the_last_parenthesis() {
        let stat =
            "4242 (ffmpeg (x) y) S 1 4242 4242 0 -1 4194560 100 0 0 0 1250 375 0 0 30 10 57 0";
        assert_eq!(parse_stat(stat), Some((4242, 1250)));
        assert_eq!(parse_stat("garbage"), None);
    }

    #[test]
    fn cgroup_memory_readings_parse() {
        assert_eq!(
            cgroup_dir("0::/user.slice/user-1500.slice/run-r1.scope\n").as_deref(),
            Some("/sys/fs/cgroup/user.slice/user-1500.slice/run-r1.scope")
        );
        assert_eq!(
            high_events("low 0\nhigh 13918\nmax 0\noom 0\n"),
            Some(13918)
        );
        assert_eq!(high_events("low 0\n"), None);
    }
}

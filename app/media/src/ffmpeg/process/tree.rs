//! Platform child-tree ownership for bounded media renders.

use std::io;
use std::process::{Child, Command};

#[cfg(unix)]
pub(super) fn configure(command: &mut Command) -> io::Result<()> {
    use std::os::unix::process::CommandExt;

    // The render leader is also its process-group leader, so every descendant
    // is owned before its executable begins running.
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn configure(command: &mut Command) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

    // Claim the Job Object before the render process can create a child. A
    // normal spawn followed by assignment has a real escape race on Windows.
    command.creation_flags(CREATE_SUSPENDED);
    Ok(())
}

#[cfg(not(any(unix, windows)))]
pub(super) fn configure(_command: &mut Command) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "render tree ownership is unavailable on this platform",
    ))
}

#[cfg(unix)]
pub(super) struct ProcessTree {
    pgid: i32,
}

#[cfg(unix)]
impl ProcessTree {
    pub(super) fn establish(child: &Child) -> io::Result<Self> {
        let pgid = i32::try_from(child.id())
            .map_err(|_| io::Error::other("render worker process id exceeds i32"))?;
        validate_process_group(pgid)?;
        Ok(Self { pgid })
    }

    pub(super) fn exit_ready(&self, child: &mut Child) -> io::Result<bool> {
        let pid = i32::try_from(child.id())
            .map_err(|_| io::Error::other("render worker process id exceeds i32"))?;
        if pid != self.pgid {
            return Err(io::Error::other(
                "render worker process group identity drifted",
            ));
        }

        // Child::try_wait reaps an exited Unix leader. Reaping would release
        // its numeric PID/PGID before descendants are closed, allowing an
        // unrelated process group to reuse the number. Observe the exit with
        // WNOWAIT so the zombie leader remains our kernel-backed ownership
        // token until hard_stop has closed the group.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                child.id() as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { info.si_pid() } != 0)
        }
    }

    pub(super) fn soft_stop(&mut self) -> io::Result<()> {
        signal_group(self.pgid, libc::SIGTERM)
    }

    pub(super) fn hard_stop(&mut self) -> io::Result<()> {
        signal_group(self.pgid, libc::SIGKILL)
    }

    pub(super) fn close_after_exit(&mut self) -> io::Result<()> {
        match self.hard_stop() {
            Ok(()) => Ok(()),
            // Darwin returns EPERM when the reserved group contains only its
            // unreaped zombie leader. The preceding WNOWAIT observation keeps
            // the PGID from being reused; a live same-UID descendant makes the
            // group signal succeed and is therefore still forcibly closed.
            #[cfg(target_os = "macos")]
            Err(error) if error.raw_os_error() == Some(libc::EPERM) => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(unix)]
fn signal_group(pgid: i32, signal: i32) -> io::Result<()> {
    validate_process_group(pgid)?;
    let result = unsafe { libc::kill(-pgid, signal) };
    if result == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn validate_process_group(pgid: i32) -> io::Result<()> {
    if pgid <= 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing unsafe render process group {pgid}"),
        ));
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::{configure, signal_group, validate_process_group, ProcessTree};
    use std::process::Command;
    use std::time::{Duration, Instant};

    #[test]
    fn unsafe_process_group_ids_fail_before_signalling() {
        for pgid in [i32::MIN, -1, 0, 1] {
            assert_eq!(
                validate_process_group(pgid).unwrap_err().kind(),
                std::io::ErrorKind::InvalidInput
            );
            assert_eq!(
                signal_group(pgid, libc::SIGKILL).unwrap_err().kind(),
                std::io::ErrorKind::InvalidInput
            );
        }
    }

    #[test]
    fn exited_group_leader_is_not_reaped_before_tree_close() {
        let mut command = Command::new("sh");
        command.args(["-c", "exit 0"]);
        configure(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        let mut tree = ProcessTree::establish(&child).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !tree.exit_ready(&mut child).unwrap() {
            assert!(Instant::now() < deadline, "child did not exit in time");
            std::thread::sleep(Duration::from_millis(5));
        }

        tree.close_after_exit().unwrap();
        assert!(child.wait().unwrap().success());
    }
}

#[cfg(windows)]
pub(super) struct ProcessTree {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl ProcessTree {
    pub(super) fn establish(child: &Child) -> io::Result<Self> {
        use std::mem::size_of;
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SET_QUOTA, PROCESS_SUSPEND_RESUME, PROCESS_TERMINATE,
        };

        let pid = child.id();
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(io::Error::from_raw_os_error(GetLastError() as i32));
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
            {
                let error = io::Error::from_raw_os_error(GetLastError() as i32);
                CloseHandle(job);
                return Err(error);
            }
            let process: HANDLE = OpenProcess(
                PROCESS_SET_QUOTA | PROCESS_TERMINATE | PROCESS_SUSPEND_RESUME,
                0,
                pid,
            );
            if process.is_null() {
                let error = io::Error::from_raw_os_error(GetLastError() as i32);
                CloseHandle(job);
                return Err(error);
            }
            if AssignProcessToJobObject(job, process) == 0 {
                let error = io::Error::from_raw_os_error(GetLastError() as i32);
                CloseHandle(process);
                CloseHandle(job);
                return Err(error);
            }
            let resume = nt_resume_process(process);
            CloseHandle(process);
            if resume != 0 {
                let error = io::Error::other(format!("NtResumeProcess failed with {resume:#x}"));
                let _ = windows_sys::Win32::System::JobObjects::TerminateJobObject(job, 1);
                CloseHandle(job);
                return Err(error);
            }
            Ok(Self { handle: job })
        }
    }

    pub(super) fn exit_ready(&self, child: &mut Child) -> io::Result<bool> {
        child.try_wait().map(|status| status.is_some())
    }

    pub(super) fn soft_stop(&mut self) -> io::Result<()> {
        Ok(())
    }

    pub(super) fn hard_stop(&mut self) -> io::Result<()> {
        use windows_sys::Win32::Foundation::GetLastError;
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;

        if unsafe { TerminateJobObject(self.handle, 1) } != 0 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(
                unsafe { GetLastError() } as i32
            ))
        }
    }

    pub(super) fn close_after_exit(&mut self) -> io::Result<()> {
        self.hard_stop()
    }
}

#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.handle) };
    }
}

#[cfg(windows)]
#[link(name = "ntdll")]
extern "system" {
    fn NtResumeProcess(process: windows_sys::Win32::Foundation::HANDLE) -> i32;
}

#[cfg(windows)]
unsafe fn nt_resume_process(process: windows_sys::Win32::Foundation::HANDLE) -> i32 {
    NtResumeProcess(process)
}

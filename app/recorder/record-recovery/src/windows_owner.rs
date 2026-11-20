//! Windows owner observation for restart recovery: process start time as
//! identity, and an exited process recognised even while handles keep it.

use crate::contract::OwnerProbe;

pub(crate) fn owner_probe(pid: u32) -> OwnerProbe {
    use windows_sys::Win32::Foundation::STILL_ACTIVE;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return if GetLastError() == ERROR_INVALID_PARAMETER {
                OwnerProbe::Dead
            } else {
                OwnerProbe::Ambiguous
            };
        }
        let mut created = std::mem::zeroed();
        let mut exited = std::mem::zeroed();
        let mut kernel = std::mem::zeroed();
        let mut user = std::mem::zeroed();
        let ok = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0;
        // A process that has exited keeps its process object (and start time)
        // while any handle to it remains open, for example in an antivirus
        // scanner. Its exit code tells it apart from a running owner.
        let mut exit_code = 0u32;
        let has_exited =
            GetExitCodeProcess(handle, &mut exit_code) != 0 && exit_code != STILL_ACTIVE as u32;
        CloseHandle(handle);
        if has_exited {
            OwnerProbe::Dead
        } else if ok {
            OwnerProbe::Identity(format!(
                "{}:{}",
                created.dwLowDateTime, created.dwHighDateTime
            ))
        } else {
            OwnerProbe::Ambiguous
        }
    }
}

//! Exact parent-PID lookup for the Windows foreground-child admission.

/// Return the direct parent process only when Windows' live process snapshot
/// can prove it. An unavailable or recycled relationship must fail closed.
pub(super) fn current_parent_pid() -> Option<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let entry_size = u32::try_from(std::mem::size_of::<PROCESSENTRY32W>())
        .expect("PROCESSENTRY32W size fits u32");
    // SAFETY: Toolhelp writes only the initialized PROCESSENTRY32W record and
    // the snapshot handle is closed on every path after a successful create.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let current = std::process::id();
        let mut entry = PROCESSENTRY32W {
            dwSize: entry_size,
            ..Default::default()
        };
        let mut result = None;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                if entry.th32ProcessID == current {
                    result = Some(entry.th32ParentProcessID);
                    break;
                }
                entry = PROCESSENTRY32W {
                    dwSize: entry_size,
                    ..Default::default()
                };
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
        result.filter(|pid| *pid != 0)
    }
}

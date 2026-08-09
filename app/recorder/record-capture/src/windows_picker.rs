//! Interactive-desktop monitor and window enumeration for Windows capture.

use crate::{MonitorInfo, WindowInfo};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, GetThreadDesktop, OpenInputDesktop, SetThreadDesktop, DESKTOP_ACCESS_FLAGS,
    DESKTOP_CONTROL_FLAGS, DESKTOP_ENUMERATE, DESKTOP_READOBJECTS,
};
use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClientRect, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, GWL_EXSTYLE, WS_EX_TOOLWINDOW,
};
use windows_capture::monitor::Monitor as WcMonitor;

fn one_based_index(position: usize) -> Option<u32> {
    u32::try_from(position).ok()?.checked_add(1)
}

pub(crate) fn list_monitors() -> Vec<MonitorInfo> {
    let monitors = match WcMonitor::enumerate() {
        Ok(monitors) => monitors,
        Err(_) => return Vec::new(),
    };
    let primary_index = WcMonitor::primary()
        .ok()
        .and_then(|monitor| monitor.index().ok())
        .and_then(|index| u32::try_from(index).ok());
    monitors
        .iter()
        .enumerate()
        .filter_map(|(position, monitor)| {
            let index = monitor
                .index()
                .ok()
                .and_then(|index| u32::try_from(index).ok())
                .or_else(|| one_based_index(position))?;
            Some(MonitorInfo {
                index,
                name: monitor
                    .name()
                    .unwrap_or_else(|_| format!("Monitor {index}")),
                width: monitor.width().unwrap_or(0),
                height: monitor.height().unwrap_or(0),
                primary: primary_index == Some(index),
            })
        })
        .collect()
}

struct WinCollector {
    items: Vec<WindowInfo>,
    self_pid: u32,
}

unsafe extern "system" fn enum_window_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let collector = unsafe { &mut *(lparam.0 as *mut WinCollector) };
    if window_is_capturable(hwnd, collector.self_pid) {
        if let (Some(id), Some(title)) =
            (one_based_index(collector.items.len()), window_title(hwnd))
        {
            collector.items.push(WindowInfo {
                id,
                title,
                app: String::new(),
            });
        }
    }
    TRUE
}

fn window_title(hwnd: HWND) -> Option<String> {
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    if length <= 0 {
        return None;
    }
    let mut buffer = vec![0u16; usize::try_from(length).ok()?.checked_add(1)?];
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    let text = String::from_utf16_lossy(buffer.get(..usize::try_from(copied).ok()?)?)
        .trim()
        .to_string();
    (!text.is_empty()).then_some(text)
}

fn window_is_capturable(hwnd: HWND, self_pid: u32) -> bool {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return false;
        }
        let minimized = IsIconic(hwnd).as_bool();
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == self_pid
            || GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & (WS_EX_TOOLWINDOW.0 as isize) != 0
        {
            return false;
        }
        let mut cloaked = 0u32;
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            std::mem::size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0
        {
            return false;
        }
        if !minimized {
            let mut rect = RECT::default();
            if GetClientRect(hwnd, &mut rect).is_ok() && (rect.right <= 0 || rect.bottom <= 0) {
                return false;
            }
        }
        true
    }
}

pub(crate) fn list_windows() -> Vec<WindowInfo> {
    std::thread::spawn(|| {
        let mut collector = WinCollector {
            items: Vec::new(),
            self_pid: unsafe { GetCurrentProcessId() },
        };
        unsafe {
            let original = GetThreadDesktop(GetCurrentThreadId()).ok();
            let access = DESKTOP_ACCESS_FLAGS(DESKTOP_READOBJECTS.0 | DESKTOP_ENUMERATE.0);
            let input = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, access).ok();
            if let Some(desktop) = input {
                let _ = SetThreadDesktop(desktop);
            }
            let _ = EnumWindows(
                Some(enum_window_proc),
                LPARAM(&mut collector as *mut _ as isize),
            );
            if let Some(desktop) = original {
                let _ = SetThreadDesktop(desktop);
            }
            if let Some(desktop) = input {
                let _ = CloseDesktop(desktop);
            }
        }
        collector.items
    })
    .join()
    .unwrap_or_default()
}

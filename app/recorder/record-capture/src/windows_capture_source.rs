//! Exact Windows source resolution shared by capture and encoder admission.

use crate::windows::{cap_err, even_capture_dimension};
use record_core::{RecordError, Result};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{IsIconic, ShowWindow, SW_RESTORE};
use windows_capture::{monitor::Monitor, window::Window};

pub(crate) fn monitor(index: Option<u32>, id: Option<&str>) -> Result<Monitor> {
    if let Some(id) = id {
        return crate::windows_monitor_target::resolve_monitor(id)
            .map_err(|error| cap_err("resolve the selected monitor identity", error));
    }
    match index {
        Some(index) => Monitor::from_index(
            usize::try_from(index)
                .map_err(|_| cap_err("get monitor by index", "index is out of range"))?,
        )
        .map_err(|error| cap_err("get monitor by index", error)),
        None => Monitor::primary().map_err(|error| cap_err("get primary monitor", error)),
    }
}

pub(crate) const WINDOW_MINIMIZED_REASON: &str = "window_minimized";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowResolution {
    ReadinessCheck,
    RecordingStart,
}

fn admit_minimized_window(
    mode: WindowResolution,
    minimized: bool,
    restore: impl FnOnce(),
) -> Result<()> {
    if !minimized {
        return Ok(());
    }
    if mode == WindowResolution::ReadinessCheck {
        return Err(RecordError::new(
            WINDOW_MINIMIZED_REASON,
            "The window is minimized. Restore it, then check again.",
            "the selected window is minimized",
        ));
    }
    restore();
    Ok(())
}

pub(crate) fn window(id: &str, mode: WindowResolution) -> Result<(Window, u32, u32)> {
    let target = crate::windows_picker::resolve_window(id)
        .map_err(crate::window_target::window_unavailable)?;
    let window = Window::from_raw_hwnd(target.0);
    let hwnd = HWND(window.as_raw_hwnd());
    // SAFETY: the resolved live window owns this borrowed HWND.
    let minimized = unsafe { IsIconic(hwnd).as_bool() };
    admit_minimized_window(mode, minimized, || {
        // SAFETY: SW_RESTORE borrows the same HWND and no output storage.
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        for _ in 0..40 {
            // SAFETY: IsIconic accepts this borrowed handle, even if it closes.
            if unsafe { !IsIconic(hwnd).as_bool() } {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    })?;
    let width = window
        .width()
        .map_err(|error| cap_err("window width", error))?;
    let height = window
        .height()
        .map_err(|error| cap_err("window height", error))?;
    let width = even_capture_dimension(width).ok_or_else(|| {
        cap_err(
            "capture the chosen window",
            "the window width is too small to encode (is it minimized?)",
        )
    })?;
    let height = even_capture_dimension(height).ok_or_else(|| {
        cap_err(
            "capture the chosen window",
            "the window height is too small to encode (is it minimized?)",
        )
    })?;
    Ok((window, width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_rejects_a_minimized_window_without_restoring_it() {
        let mut restore_calls = 0;
        let error = admit_minimized_window(WindowResolution::ReadinessCheck, true, || {
            restore_calls += 1;
        })
        .unwrap_err();
        assert_eq!(restore_calls, 0);
        assert_eq!(error.code, WINDOW_MINIMIZED_REASON);
    }

    #[test]
    fn recording_start_still_restores_a_minimized_window() {
        let mut restore_calls = 0;
        admit_minimized_window(WindowResolution::RecordingStart, true, || {
            restore_calls += 1;
        })
        .unwrap();
        assert_eq!(restore_calls, 1);
    }
}

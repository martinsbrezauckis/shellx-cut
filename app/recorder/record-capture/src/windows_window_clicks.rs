//! Selected-window physical click mapping, owned by the same WGC stream.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowThreadProcessId, IsChild, IsIconic, IsWindow, WindowFromPhysicalPoint,
};

use crate::surface_coordinates::AbsoluteInputOutput;
use crate::window_click_coordinates::{WindowClicks, WindowRect};
use record_core::ClickSample;

#[derive(Clone)]
pub(crate) struct WindowClickCapture {
    hwnd: usize,
    pid: u32,
    start: Instant,
    output: (u32, u32),
    state: Arc<Mutex<WindowClicks>>,
    worker: Arc<crate::window_click_worker::WindowClickWorker>,
}

impl WindowClickCapture {
    pub(crate) fn new(
        id: &str,
        admitted_hwnd: HWND,
        width: u32,
        height: u32,
        start: Instant,
    ) -> Result<Self, &'static str> {
        let (hwnd, pid) =
            crate::window_target::admitted_windows_window_identity(id, admitted_hwnd.0 as usize)
                .ok_or(
                    "selected-window click owner does not match the admitted capture identity",
                )?;
        let state = Arc::new(Mutex::new(WindowClicks::new(width, height)));
        let worker = crate::window_click_worker::WindowClickWorker::new(
            state.clone(),
            start,
            move |point| observed_geometry(hwnd, pid, point),
        )
        .map_err(|_| "selected-window geometry observer could not start")?;
        Ok(Self {
            hwnd,
            pid,
            start,
            output: (width, height),
            state,
            worker: Arc::new(worker),
        })
    }

    /// DWM extended bounds are physical pixels, unlike DPI-virtualized WindowRect.
    /// A missing/replaced/minimized source never inherits another window's origin.
    pub(crate) fn geometry(&self) -> Option<WindowRect> {
        window_geometry(self.hwnd, self.pid)
    }

    pub(crate) fn observe_event(&self, event: &rdevin::Event, callback_ms: u64) {
        let (button, down) = match event.event_type {
            rdevin::EventType::ButtonPress(button) => (crate::input::map_button(button), true),
            rdevin::EventType::ButtonRelease(button) => (crate::input::map_button(button), false),
            _ => return,
        };
        let point = event.native_pointer.as_ref().map(|p| (p.x, p.y, p.age_ms));
        self.worker.enqueue(callback_ms, point, button, down);
    }

    /// Bind only encoder-accepted WGC pixels, not source-start or preview success.
    /// Use the exact full-source fit returned by the accepted immutable sample.
    pub(crate) fn accepted_frame(
        &self,
        before: Option<WindowRect>,
        width: u32,
        height: u32,
        fit: crate::window_frame_fit::WindowFrameFit,
    ) {
        let after = self.geometry();
        let content = after
            .filter(|rect| before == Some(*rect) && rect.width == width && rect.height == height)
            .filter(|_| fit.source_size() == (width, height) && fit.output_size() == self.output)
            .map(|_| fit);
        let t_ms = u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .accepted_fit(t_ms, content);
    }

    pub(crate) fn source_closed(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .close();
    }

    pub(crate) fn finish(
        &self,
        duration_ms: u64,
        clicks: &mut Vec<ClickSample>,
    ) -> AbsoluteInputOutput {
        self.worker.seal();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let sealed =
            std::mem::replace(&mut *state, WindowClicks::new(self.output.0, self.output.1));
        state.close();
        let (samples, correlation) = sealed.finish(duration_ms);
        *clicks = samples;
        AbsoluteInputOutput {
            cursor: vec![],
            scrolls: vec![],
            correlation,
        }
    }
}

fn window_geometry(raw: usize, expected_pid: u32) -> Option<WindowRect> {
    let hwnd = HWND(raw as *mut std::ffi::c_void);
    let mut pid = 0;
    let mut rect = RECT::default();
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() || IsIconic(hwnd).as_bool() {
            return None;
        }
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid != expected_pid {
            return None;
        }
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut RECT as *mut std::ffi::c_void,
            std::mem::size_of::<RECT>() as u32,
        )
        .ok()?;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid != expected_pid {
            return None;
        }
    }
    let width = u32::try_from(rect.right.checked_sub(rect.left)?).ok()?;
    let height = u32::try_from(rect.bottom.checked_sub(rect.top)?).ok()?;
    (width > 0 && height > 0).then_some(WindowRect {
        left: rect.left,
        top: rect.top,
        width,
        height,
    })
}

fn observed_geometry(raw: usize, expected_pid: u32, point: (f64, f64)) -> Option<WindowRect> {
    let hwnd = HWND(raw as *mut std::ffi::c_void);
    let before = window_geometry(raw, expected_pid)?;
    let hit = unsafe {
        WindowFromPhysicalPoint(POINT {
            x: point.0 as i32,
            y: point.1 as i32,
        })
    };
    let mut pid = 0;
    let owned = unsafe {
        GetWindowThreadProcessId(hit, Some(&mut pid));
        pid == expected_pid && (hit == hwnd || IsChild(hwnd, hit).as_bool())
    };
    let after = window_geometry(raw, expected_pid)?;
    (owned && before == after).then_some(after)
}

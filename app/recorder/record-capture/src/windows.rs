//! windows.rs — live screen + input capture on Windows.
//!
//! Compiled ONLY for `cfg(windows)` + the `capture-windows` feature.
//! - SCREEN: windows-capture (Windows Graphics Capture) → MP4 via its built-in
//!   Media Foundation encoder (no ffmpeg). Cursor WITHOUT the OS cursor + WITHOUT
//!   the capture border — we re-render a synthetic cursor in the polish pass.
//! - INPUT: the shared rdevin hook (see input.rs).
//!
//! When running Windows capture tests from WSL, launch the built Windows app via
//! `cmd.exe /c "C:\…\cutd.exe …"`; a direct WSL-interop UNC working dir breaks
//! WGC dispatcher init (0x80070490).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use record_core::{error_codes, EventTrack, Monitor as RMonitor, RecordError, Result, Settings};

use windows_capture::{
    capture::{Context, GraphicsCaptureApiHandler},
    encoder::{AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder},
    frame::Frame,
    graphics_capture_api::InternalCaptureControl,
    monitor::Monitor as WcMonitor,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings as WcSettings,
    },
    window::Window as WcWindow,
};

use crate::{input, Capture, CaptureConfig, CaptureOutput, MonitorInfo, WindowInfo};

fn one_based_index(position: usize) -> Option<u32> {
    u32::try_from(position).ok()?.checked_add(1)
}

fn even_capture_dimension(value: i32) -> Option<u32> {
    let even = u32::try_from(value).ok()? & !1;
    (even >= 2).then_some(even)
}

fn capture_fps(value: f64) -> u32 {
    let bounded = if value.is_finite() {
        value.clamp(1.0, 240.0)
    } else {
        30.0
    };
    bounded.round() as u32
}

// Raw Win32 for our OWN window enumeration (see list_windows below). windows-capture's
// Window::enumerate uses EnumChildWindows(GetDesktopWindow()), which returns EMPTY from
// cutd's sidecar thread; we use EnumWindows on a thread bound to the interactive desktop.
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
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, ShowWindow, GWL_EXSTYLE, SW_RESTORE,
    WS_EX_TOOLWINDOW,
};

/// Enumerate the attached displays for the in-app monitor PICKER (Windows impl of
/// [`crate::list_monitors`]). Uses `windows-capture`'s `Monitor::enumerate()` and
/// maps each to a [`MonitorInfo`] with the 1-based index the capture backend wants.
///
/// Robustness: each monitor's `.index()` / `.name()` / `.width()` / `.height()` is a
/// fallible Win32 call. If `.index()` fails we fall back to the enumeration position
/// (`pos + 1`, which is also what `Monitor::from_index` keys off); a failed `.name()`
/// falls back to `"Monitor N"`; failed dimensions fall back to `0`. We never drop a
/// monitor for a single bad accessor — a partial card is more useful than an empty list.
/// A theoretical index beyond the UI's u32 contract is skipped rather than truncated.
/// The primary is marked by comparing each monitor's index to
/// `Monitor::primary()`'s index (best-effort; no primary match → none flagged).
pub(crate) fn list_monitors() -> Vec<MonitorInfo> {
    let monitors = match WcMonitor::enumerate() {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };
    // The primary's 1-based index, if it can be resolved (best-effort).
    let primary_index: Option<u32> = WcMonitor::primary()
        .ok()
        .and_then(|p| p.index().ok())
        .and_then(|i| u32::try_from(i).ok());

    monitors
        .iter()
        .enumerate()
        .filter_map(|(pos, m)| {
            // Prefer the OS-reported 1-based index; fall back to the enumeration
            // position (from_index keys off the same ordering).
            let index = m
                .index()
                .ok()
                .and_then(|i| u32::try_from(i).ok())
                .or_else(|| one_based_index(pos))?;
            let name = m.name().unwrap_or_else(|_| format!("Monitor {index}"));
            let width = m.width().unwrap_or(0);
            let height = m.height().unwrap_or(0);
            let primary = primary_index == Some(index);
            Some(MonitorInfo {
                index,
                name,
                width,
                height,
                primary,
            })
        })
        .collect()
}

/// Collector handed to the `EnumWindows` callback through its `LPARAM`.
struct WinCollector {
    items: Vec<WindowInfo>,
    self_pid: u32,
}

/// `EnumWindows` callback — keep visible, titled, non-tool, non-cloaked, non-minimized
/// top-level windows that aren't our own process. Always returns TRUE
/// so enumeration continues.
unsafe extern "system" fn enum_window_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: list_windows passes an aligned, non-null pointer to its live stack
    // collector, and EnumWindows invokes callbacks synchronously before returning.
    let col = unsafe { &mut *(lparam.0 as *mut WinCollector) };
    if window_is_capturable(hwnd, col.self_pid) {
        if let Some(title) = window_title(hwnd) {
            let Some(id) = one_based_index(col.items.len()) else {
                return TRUE;
            };
            col.items.push(WindowInfo {
                id,
                title,
                app: String::new(),
            });
        }
    }
    TRUE
}

/// The window's title text, or None if empty/blank.
fn window_title(hwnd: HWND) -> Option<String> {
    // SAFETY: GetWindowTextLengthW does not dereference caller-owned memory. An
    // invalid or disappearing HWND is reported as a zero/error result.
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return None;
    }
    let capacity = usize::try_from(len).ok()?.checked_add(1)?;
    let mut buf = vec![0u16; capacity];
    // SAFETY: buf is initialized, writable, and its slice length is supplied to
    // Win32 by the generated binding. A stale HWND returns zero.
    let copied = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if copied <= 0 {
        return None;
    }
    let copied = usize::try_from(copied).ok()?;
    let t = String::from_utf16_lossy(buf.get(..copied)?)
        .trim()
        .to_string();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

/// Source-picker filter: a real top-level app window — visible, not our own process, not a
/// tool window, not DWM-cloaked, non-empty client area. MINIMIZED windows are KEPT: a
/// minimized app is still a valid target the user expects to see, and excluding them made
/// the source list flicker (a window vanished the moment it was minimized, reappeared when
/// restored. WS_VISIBLE stays set while iconic; IsIconic was what dropped
/// them. The capture backend restores a minimized target on start so WGC sees real content.
fn window_is_capturable(hwnd: HWND, self_pid: u32) -> bool {
    // SAFETY: every call treats hwnd as an opaque borrowed handle. Win32 reports
    // invalid/stale handles through return values, and all output pointers refer
    // to initialized local storage with the exact size passed to the API.
    unsafe {
        // Truly hidden windows are out — but a MINIMIZED window keeps WS_VISIBLE, so this
        // only rejects genuinely-hidden ones.
        if !IsWindowVisible(hwnd).as_bool() {
            return false;
        }
        let minimized = IsIconic(hwnd).as_bool();
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == self_pid {
            return false;
        }
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if ex & (WS_EX_TOOLWINDOW.0 as isize) != 0 {
            return false;
        }
        let mut cloaked: u32 = 0;
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
        // Degenerate-window filter (no client area). SKIP it for minimized windows, which
        // legitimately report a 0×0 client rect while iconic — that 0×0 was the SECOND way
        // minimized windows were being dropped.
        if !minimized {
            let mut rect = RECT::default();
            if GetClientRect(hwnd, &mut rect).is_ok() && (rect.right <= 0 || rect.bottom <= 0) {
                return false;
            }
        }
        true
    }
}

/// Enumerate the on-screen application windows for the in-app WINDOW picker (Windows impl
/// of [`crate::list_windows`]). Raw `EnumWindows` (reliable return, unlike `EnumChildWindows`
/// whose BOOL is documented "not used"), filtered to visible, non-minimized windows,
/// not our process, not a tool window, not DWM-cloaked, non-empty client area). cutd inherits
/// the interactive desktop from its launcher, so it sees the real top-level windows (verified
/// live: ShellX Cut / Chrome / the terminal). `title` is what `CaptureConfig.window` re-resolves.
///
/// NOTE: the long-standing `windows:0` was NOT this function — `screen_record.doctor`'s JSON
/// response simply never serialized the `windows` field (dispatch.rs), so it never reached the
/// UI regardless of what this returned.
pub(crate) fn list_windows() -> Vec<WindowInfo> {
    // EnumWindows enumerates the CALLING THREAD's desktop. cutd's HTTP handler runs on
    // a tokio worker that is NOT reliably bound to the interactive (input) desktop, so a
    // direct call intermittently enumerated 0 windows with real Chrome and terminal windows;
    // terminal windows came back once, then empty on every subsequent probe). Run the
    // enumeration on a DEDICATED thread explicitly bound to the input desktop
    // (OpenInputDesktop + SetThreadDesktop) — the deterministic way to see the real
    // top-level windows from a server thread. We restore + close the desktop handle so we
    // don't leak one per 3s poll.
    std::thread::spawn(|| {
        // SAFETY: GetCurrentProcessId has no arguments or memory preconditions.
        let self_pid = unsafe { GetCurrentProcessId() };
        let mut col = WinCollector {
            items: Vec::new(),
            self_pid,
        };
        // SAFETY: this new thread owns no windows or hooks. The input desktop
        // handle remains open while attached; EnumWindows is synchronous and the
        // LPARAM points to col until it returns. We restore the borrowed original
        // desktop before closing only the handle returned by OpenInputDesktop.
        unsafe {
            let orig = GetThreadDesktop(GetCurrentThreadId()).ok();
            let access = DESKTOP_ACCESS_FLAGS(DESKTOP_READOBJECTS.0 | DESKTOP_ENUMERATE.0);
            let input = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, access).ok();
            if let Some(d) = input {
                let _ = SetThreadDesktop(d);
            }
            let _ = EnumWindows(Some(enum_window_proc), LPARAM(&mut col as *mut _ as isize));
            if let Some(o) = orig {
                let _ = SetThreadDesktop(o);
            }
            if let Some(d) = input {
                let _ = CloseDesktop(d);
            }
        }
        col.items
    })
    .join()
    .unwrap_or_default()
}

fn cap_err(ctx: &str, e: impl std::fmt::Display) -> RecordError {
    RecordError::new(error_codes::CAPTURE, ctx, e.to_string())
        .with_action("ensure this is a Windows desktop session with Graphics Capture available")
}

/// Encoder flags handed to the capture handler (its `new` builds the encoder).
#[derive(Clone)]
struct EncFlags {
    w: u32,
    h: u32,
    fps: u32,
    path: String,
}

/// windows-capture handler: each arrived frame is fed to the MP4 encoder.
struct Handler {
    encoder: Option<VideoEncoder>,
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = EncFlags;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> std::result::Result<Self, Self::Error> {
        let f = ctx.flags;
        let encoder = VideoEncoder::new(
            VideoSettingsBuilder::new(f.w, f.h).frame_rate(f.fps),
            AudioSettingsBuilder::default().disabled(true),
            ContainerSettingsBuilder::default(),
            &f.path,
        )?;
        Ok(Self {
            encoder: Some(encoder),
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _ctl: InternalCaptureControl,
    ) -> std::result::Result<(), Self::Error> {
        if let Some(e) = self.encoder.as_mut() {
            e.send_frame(frame)?;
        }
        Ok(())
    }

    fn on_closed(&mut self) -> std::result::Result<(), Self::Error> {
        if let Some(e) = self.encoder.take() {
            e.finish()?;
        }
        Ok(())
    }
}

/// Live Windows capture backend.
pub struct WindowsCapture;

impl WindowsCapture {
    pub fn new() -> Self {
        Self
    }
}

impl Capture for WindowsCapture {
    fn capture(&self, cfg: &CaptureConfig, stop: Arc<AtomicBool>) -> Result<CaptureOutput> {
        // Open-ended capture: `None` = run until the external `stop` is set
        // (huge cap so only `stop` ends it); a concrete `duration_ms` is an upper
        // bound. The wait loop below polls `stop` 10×/s so `screen_record.stop` ends
        // the WGC capture promptly.
        let dur = cfg.duration_ms.unwrap_or(u64::MAX / 4);
        let fps = capture_fps(cfg.fps);

        // Resolve the capture SOURCE. A specific window (by title) takes precedence
        // over a monitor; else the chosen monitor; else primary. WGC accepts either a
        // Window or a Monitor as the capture item (both impl TryInto<…ItemType>).
        enum Src {
            Monitor(WcMonitor),
            Window(WcWindow),
        }
        let (src, w, h) = if let Some(ref title) = cfg.window {
            let win = WcWindow::from_contains_name(title)
                .map_err(|e| cap_err("find the window to capture by title", e))?;
            // The source picker lists minimized windows so the list is stable
            // as windows are minimized/restored). A minimized window has a 0×0 capture
            // area, so RESTORE the target before capturing — selecting a minimized window
            // then records real content instead of erroring "is it minimized?".
            let hwnd = HWND(win.as_raw_hwnd());
            // SAFETY: hwnd is borrowed from the live windows-capture Window. Win32
            // treats a stale handle as not iconic rather than dereferencing Rust memory.
            if unsafe { IsIconic(hwnd).as_bool() } {
                // SAFETY: hwnd remains owned by win; ShowWindow borrows the opaque
                // handle and SW_RESTORE requires no caller-owned output storage.
                unsafe {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                // Wait for it to leave the iconic state (so width/height read the real
                // restored rect, not the 0×0 minimized one), bounded so a stuck restore
                // can't hang the capture start.
                for _ in 0..40 {
                    // SAFETY: same live borrowed HWND contract as the initial check.
                    if unsafe { !IsIconic(hwnd).as_bool() } {
                        break;
                    }
                    thread::sleep(Duration::from_millis(25));
                }
            }
            let ww = win.width().map_err(|e| cap_err("window width", e))?;
            let wh = win.height().map_err(|e| cap_err("window height", e))?;
            let Some(ww) = even_capture_dimension(ww) else {
                return Err(cap_err(
                    "capture the chosen window",
                    "the window width is too small to encode (is it minimized?)",
                ));
            };
            let Some(wh) = even_capture_dimension(wh) else {
                return Err(cap_err(
                    "capture the chosen window",
                    "the window height is too small to encode (is it minimized?)",
                ));
            };
            // yuv420p + the WGC encoder want even dimensions; the helper also
            // rejects 0/1 rather than rounding a one-pixel surface down to zero.
            (Src::Window(win), ww, wh)
        } else {
            let monitor = match cfg.monitor {
                Some(i) => {
                    let index = usize::try_from(i)
                        .map_err(|_| cap_err("get monitor by index", "index is out of range"))?;
                    WcMonitor::from_index(index).map_err(|e| cap_err("get monitor by index", e))?
                }
                None => WcMonitor::primary().map_err(|e| cap_err("get primary monitor", e))?,
            };
            let mw = monitor.width().map_err(|e| cap_err("monitor width", e))?;
            let mh = monitor.height().map_err(|e| cap_err("monitor height", e))?;
            (Src::Monitor(monitor), mw, mh)
        };

        let out_dir = cfg.out_dir.trim_end_matches(['/', '\\']).to_string();
        std::fs::create_dir_all(&out_dir).map_err(|e| cap_err("create output dir", e))?;
        let path = format!("{out_dir}/source.mp4");

        // `stop` is the EXTERNAL flag passed in, not a fresh internal one:
        // it drives mic + input + the capture-window wait loop, so `screen_record.stop`
        // can end the capture before `dur`.

        // Start the mic in PARALLEL — it must NEVER gate the screen. Blocking up to 8 s
        // here for the mic to go "ready" starved the screen capture on a machine with no
        // input device (or a slow permission grant): a short clip captured nothing and a
        // long one lost its first ~8 s. The Record surface pre-warms the mic via `mic::warm`
        // so the first-frame audio race is handled up front; spawn and move on. No
        // input device → the mic thread returns Err → audio is None (handled at join).
        let mic_handle = if cfg.audio {
            let ready = Arc::new(AtomicBool::new(false));
            Some(crate::mic::spawn_mic(
                format!("{out_dir}/mic.wav"),
                stop.clone(),
                ready,
            ))
        } else {
            None
        };

        // Clock + input + screen all begin together, aligned with the live mic.
        let start = Instant::now();
        let input = input::spawn_listener(start, stop.clone(), cfg.capture_keys);

        let flags = EncFlags {
            w,
            h,
            fps,
            path: path.clone(),
        };
        // Start WGC against the resolved source. Monitor + Window are distinct types,
        // so the Settings::new + start is duplicated per arm (flags moves into the one
        // arm that runs — match arms are mutually exclusive, so this borrow-checks).
        let control = match src {
            Src::Monitor(m) => Handler::start_free_threaded(WcSettings::new(
                m,
                CursorCaptureSettings::WithoutCursor,
                DrawBorderSettings::WithoutBorder,
                SecondaryWindowSettings::Default,
                MinimumUpdateIntervalSettings::Default,
                DirtyRegionSettings::Default,
                ColorFormat::Rgba8,
                flags,
            )),
            Src::Window(win) => Handler::start_free_threaded(WcSettings::new(
                win,
                CursorCaptureSettings::WithoutCursor,
                DrawBorderSettings::WithoutBorder,
                SecondaryWindowSettings::Default,
                MinimumUpdateIntervalSettings::Default,
                DirtyRegionSettings::Default,
                ColorFormat::Rgba8,
                flags,
            )),
        }
        .map_err(|e| cap_err("start capture", e))?;

        // Poll the external stop (and the deadline) instead of one fixed sleep:
        // open-ended capture (`dur == u64::MAX/4`) would otherwise block ~forever.
        while !stop.load(Ordering::Relaxed) && start.elapsed() < Duration::from_millis(dur) {
            thread::sleep(Duration::from_millis(100));
        }
        stop.store(true, Ordering::Relaxed);
        control.stop().map_err(|e| cap_err("stop capture", e))?;

        let audio =
            mic_handle.and_then(
                |h| match crate::mic::join_bounded(h, Duration::from_secs(2)) {
                    Some(Ok(p)) => Some(p),
                    Some(Err(e)) => {
                        eprintln!("warning: mic capture failed, recording without audio: {e}");
                        None
                    }
                    None => {
                        eprintln!(
                            "warning: mic capture did not stop within 2s, recording without audio"
                        );
                        None
                    }
                },
            );
        let (cursor, clicks, scrolls, keys) = input.lock().unwrap().snapshot();
        let duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

        let events = EventTrack {
            duration_ms,
            screen_w: w,
            screen_h: h,
            monitors: vec![RMonitor {
                id: 0,
                x: 0,
                y: 0,
                w,
                h,
                primary: true,
            }],
            cursor,
            clicks,
            scrolls,
            keys,
        };
        Ok(CaptureOutput {
            source_video: path,
            events,
            webcam_video: None,
            audio,
            settings: Settings {
                width: w,
                height: h,
                fps: fps as f32,
                audio_rate: 48_000,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{capture_fps, even_capture_dimension, one_based_index};

    #[test]
    fn checked_capture_numbers_reject_or_bound_invalid_values() {
        assert_eq!(one_based_index(0), Some(1));
        assert_eq!(one_based_index(usize::MAX), None);
        assert_eq!(even_capture_dimension(-1), None);
        assert_eq!(even_capture_dimension(1), None);
        assert_eq!(even_capture_dimension(3), Some(2));
        assert_eq!(capture_fps(f64::NAN), 30);
        assert_eq!(capture_fps(0.1), 1);
        assert_eq!(capture_fps(500.0), 240);
    }
}

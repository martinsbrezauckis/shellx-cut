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

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use record_core::{error_codes, EventTrack, Monitor as RMonitor, RecordError, Result, Settings};

use windows_capture::{
    capture::GraphicsCaptureApiHandler,
    monitor::Monitor as WcMonitor,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings as WcSettings,
    },
    window::Window as WcWindow,
};

use crate::windows_wgc_handler::{EncFlags, Handler, LiveWgcControl};
use crate::{
    checkpoint::Checkpoints,
    input,
    region_geometry::NativePixelCrop,
    surface_coordinates,
    windows_wgc_run::{
        WgcAcceptedCapture, WgcCaptureRange, WgcCheckpointPublisher, WgcNativeControl, WgcRunOwner,
        WgcStartObservation, WgcStartedControl,
    },
    Capture, CaptureConfig, CaptureOutput, MonitorInfo, WindowInfo,
};

pub(crate) struct WindowsCheckpointPublisher {
    pub(crate) checkpoints: Checkpoints,
}

impl WgcCheckpointPublisher for WindowsCheckpointPublisher {
    fn reserve(&mut self, start_ms: u64) -> Result<(u64, std::path::PathBuf)> {
        self.checkpoints.begin_windows_wgc(start_ms)
    }

    fn verify_and_publish_new(
        &mut self,
        sequence: u64,
        staging: &Path,
        facts: record_recovery::CheckpointFacts,
    ) -> Result<record_recovery::Checkpoint> {
        self.checkpoints.publish(sequence, staging, facts)
    }
}

fn even_capture_dimension(value: i32) -> Option<u32> {
    let even = u32::try_from(value).ok()? & !1;
    (even >= 2).then_some(even)
}

fn capture_fps(value: f64) -> u32 {
    record_core::backend_fps_v1(value)
}

/// The global desktop rectangle WGC captures for this monitor. rdevin's low-level
/// hook reports this desktop coordinate space, so an exact transform needs it.
pub(crate) fn monitor_surface(monitor: &WcMonitor) -> Option<surface_coordinates::CaptureSurface> {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO};

    let mut info = MONITORINFO {
        cbSize: u32::try_from(std::mem::size_of::<MONITORINFO>()).ok()?,
        rcMonitor: RECT::default(),
        rcWork: RECT::default(),
        dwFlags: 0,
    };
    // SAFETY: the raw HMONITOR remains owned by `monitor`; `info` is initialized
    // writable storage of the exact Win32 structure size.
    if unsafe { !GetMonitorInfoW(HMONITOR(monitor.as_raw_hmonitor()), &mut info).as_bool() } {
        return None;
    }
    let rect = info.rcMonitor;
    surface_coordinates::CaptureSurface::new(
        f64::from(rect.left),
        f64::from(rect.top),
        f64::from(rect.right - rect.left),
        f64::from(rect.bottom - rect.top),
    )
}

pub(crate) fn wgc_monitor_range(
    surface: surface_coordinates::CaptureSurface,
) -> Option<WgcCaptureRange> {
    let (origin_x, origin_y, width, height) = surface.global_geometry();
    let integral = |value: f64| value.is_finite() && value.fract() == 0.0;
    if !integral(origin_x) || !integral(origin_y) || !integral(width) || !integral(height) {
        return None;
    }
    WgcCaptureRange::new(
        i32::try_from(origin_x as i64).ok()?,
        i32::try_from(origin_y as i64).ok()?,
        u32::try_from(width as u64).ok()?,
        u32::try_from(height as u64).ok()?,
    )
    .ok()
}

pub(crate) fn observe_wgc_start(start: Instant) -> Result<WgcStartObservation> {
    let start_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    WgcStartObservation::observed_now(start_ms)
}

fn ffmpeg_bin() -> String {
    std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string())
}
fn ffprobe_bin() -> String {
    std::env::var("SHELLX_RECORD_FFPROBE").unwrap_or_else(|_| "ffprobe".to_string())
}

// Raw Win32 for our OWN window enumeration (see list_windows below). windows-capture's
// Window::enumerate uses EnumChildWindows(GetDesktopWindow()), which returns EMPTY from
// cutd's sidecar thread; we use EnumWindows on a thread bound to the interactive desktop.
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{IsIconic, ShowWindow, SW_RESTORE};

pub(crate) fn list_monitors() -> Vec<MonitorInfo> {
    crate::windows_picker::list_monitors()
}

pub(crate) fn list_windows() -> Vec<WindowInfo> {
    crate::windows_picker::list_windows()
}

pub(crate) fn cap_err(ctx: &str, e: impl std::fmt::Display) -> RecordError {
    RecordError::new(error_codes::CAPTURE, ctx, e.to_string())
        .with_action("ensure this is a Windows desktop session with Graphics Capture available")
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
        // WGC frames and the low-level mouse hook use physical per-monitor
        // coordinates. Hold this guard for every monitor lookup, Region crop,
        // and input-map calculation in this capture worker.
        let _dpi_context = crate::windows_runtime::enter_per_monitor_dpi_v2()
            .map_err(|error| cap_err("enter per-monitor DPI coordinate space", error))?;
        crate::windows_runtime::pin_process_mta()
            .map_err(|error| cap_err("initialize Windows capture runtime", error))?;
        // Open-ended capture: `None` = run until the external `stop` is set
        // (huge cap so only `stop` ends it); a concrete `duration_ms` is an upper
        // bound. The wait loop below polls `stop` 10×/s so `screen_record.stop` ends
        // the WGC capture promptly.
        let dur = cfg.duration_ms.unwrap_or(u64::MAX / 4);
        let fps = capture_fps(cfg.fps);

        // Resolve the capture SOURCE. A specific opaque native window id takes precedence
        // over a monitor; else an exact opaque monitor id; else the legacy chosen
        // monitor index; else primary. An exact monitor id never falls back to an
        // ordinal/name/geometry match. WGC accepts either a Window or a Monitor as
        // the capture item (both impl TryInto<…ItemType>).
        #[derive(Clone, Copy)]
        enum Src {
            Monitor(WcMonitor),
            Window(WcWindow),
        }
        let (src, parent_w, parent_h, parent_surface) = if let Some(ref window_id) = cfg.window {
            let target = crate::windows_picker::resolve_window(window_id)
                .map_err(|error| cap_err("resolve the selected window identity", error))?;
            let win = WcWindow::from_raw_hwnd(target.0);
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
            // WGC does not expose timestamped window rectangles from this encoder
            // callback. A startup rect becomes stale on move/resize, so window
            // pointer positions stay unavailable until that provenance exists.
            (Src::Window(win), ww, wh, None)
        } else {
            let monitor = if let Some(id) = cfg.monitor_id.as_deref() {
                crate::windows_monitor_target::resolve_monitor(id)
                    .map_err(|error| cap_err("resolve the selected monitor identity", error))?
            } else {
                match cfg.monitor {
                    Some(i) => {
                        let index = usize::try_from(i).map_err(|_| {
                            cap_err("get monitor by index", "index is out of range")
                        })?;
                        WcMonitor::from_index(index)
                            .map_err(|e| cap_err("get monitor by index", e))?
                    }
                    None => WcMonitor::primary().map_err(|e| cap_err("get primary monitor", e))?,
                }
            };
            let mw = monitor.width().map_err(|e| cap_err("monitor width", e))?;
            let mh = monitor.height().map_err(|e| cap_err("monitor height", e))?;
            let surface = monitor_surface(&monitor);
            (Src::Monitor(monitor), mw, mh, surface)
        };
        crate::windows_controller_exclusion::admit_controller_placement(
            cfg.controller_placement.as_ref(),
            matches!(src, Src::Window(_)),
        );

        let (w, h, surface, crop) = match cfg.region {
            Some(region) => {
                if matches!(src, Src::Window(_)) {
                    return Err(cap_err(
                        "capture the selected Region",
                        "a Region must be bound to one exact monitor, not a window",
                    ));
                }
                let crop = NativePixelCrop::from_capture_region(region).ok_or_else(|| {
                    cap_err("capture the selected Region", "the native crop is invalid")
                })?;
                if crop.parent_size() != (parent_w, parent_h) {
                    return Err(cap_err(
                        "capture the selected Region",
                        "the selected display changed size before WGC started",
                    ));
                }
                let subsurface = parent_surface
                    .and_then(|surface| {
                        surface.subsurface_from_native_crop(crop, parent_w, parent_h)
                    })
                    .ok_or_else(|| {
                        cap_err(
                            "capture the selected Region",
                            "the selected display geometry no longer maps to the native crop",
                        )
                    })?;
                let (width, height) = crop.output_size();
                (
                    width,
                    height,
                    Some(subsurface.capture_surface()),
                    Some(crop),
                )
            }
            None => (parent_w, parent_h, parent_surface, None),
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
        let start = cfg
            .clock
            .as_ref()
            .map(crate::CaptureClock::start)
            .unwrap_or_else(Instant::now);
        // The owner seals immediately after WGC's measured final-video boundary;
        // any early capture error drops it and performs best-effort native cleanup.
        let input = input::InputListener::start(start, cfg.capture_keys)?;
        let mic_handle = if cfg.audio {
            let ready = Arc::new(AtomicBool::new(false));
            let mic_path = format!("{out_dir}/mic.wav");
            Some(match cfg.microphone_level.clone() {
                Some(level) => crate::mic_endpoint::spawn_microphone_capture_with_level(
                    mic_path,
                    cfg.microphone_source.clone(),
                    stop.clone(),
                    ready,
                    start,
                    level,
                ),
                None => crate::mic_endpoint::spawn_microphone_capture(
                    mic_path,
                    cfg.microphone_source.clone(),
                    stop.clone(),
                    ready,
                    start,
                ),
            })
        } else {
            None
        };

        let range = match src {
            Src::Monitor(_) => surface.and_then(wgc_monitor_range),
            Src::Window(_) => None,
        }
        .filter(|range| range.width == w && range.height == h);
        let accepted = WgcAcceptedCapture::new(
            Settings {
                width: w,
                height: h,
                fps: fps as f32,
                audio_rate: 48_000,
            },
            range,
        )?;
        let mut selected_window_segment_started = false;
        let mut start_wgc = |target: &Src,
                             destination: &Path|
         -> Result<WgcStartedControl<LiveWgcControl>> {
            let source_lifecycle = cfg.source_lifecycle.clone();
            if matches!(target, Src::Window(_)) {
                if let Some(lifecycle) = source_lifecycle.as_ref() {
                    if selected_window_segment_started {
                        lifecycle.arm_next_owned_segment(
                                "Windows Graphics Capture is watching the exact selected window for closure.",
                            );
                    } else {
                        lifecycle.arm_initial_selected_source(
                                "Windows Graphics Capture is watching the exact selected window for closure.",
                            );
                        selected_window_segment_started = true;
                    }
                }
            }
            let flags = EncFlags {
                w,
                h,
                fps,
                path: destination.display().to_string(),
                crop,
                readiness: cfg.readiness.clone(),
                source_lifecycle: source_lifecycle.clone(),
                stop: stop.clone(),
            };
            let control = match *target {
                Src::Monitor(m) => Handler::start_free_threaded(WcSettings::new(
                    m,
                    CursorCaptureSettings::WithoutCursor,
                    DrawBorderSettings::Default,
                    SecondaryWindowSettings::Default,
                    MinimumUpdateIntervalSettings::Default,
                    DirtyRegionSettings::Default,
                    ColorFormat::Rgba8,
                    flags,
                )),
                Src::Window(win) => Handler::start_free_threaded(WcSettings::new(
                    win,
                    CursorCaptureSettings::WithoutCursor,
                    DrawBorderSettings::Default,
                    SecondaryWindowSettings::Default,
                    MinimumUpdateIntervalSettings::Default,
                    DirtyRegionSettings::Default,
                    ColorFormat::Rgba8,
                    flags,
                )),
            }
            .map_err(|e| cap_err("start capture", e))?;
            Ok(WgcStartedControl::new(
                LiveWgcControl {
                    close: Some(Box::new(move || {
                        control
                            .stop()
                            .map_err(|error| cap_err("finalize WGC checkpoint", error))
                    })),
                    source_lifecycle,
                },
                accepted.clone(),
            ))
        };
        let (duration_ms, checkpoints) = match Checkpoints::open(cfg.checkpoint.as_ref())? {
            Some(checkpoints) => {
                let interval_ms = checkpoints.interval_ms();
                let publisher = WindowsCheckpointPublisher { checkpoints };
                let mut owner = WgcRunOwner::new(src, start_wgc, publisher);
                let mut segment = owner.begin(0, || observe_wgc_start(start))?;
                let duration_ms = loop {
                    let end_at = segment.start_ms.saturating_add(interval_ms).min(dur);
                    while !stop.load(Ordering::Relaxed)
                        && start.elapsed() < Duration::from_millis(end_at)
                    {
                        thread::sleep(Duration::from_millis(50));
                    }
                    let ended_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                    if stop.load(Ordering::Relaxed) || ended_ms >= dur {
                        if let Some(lifecycle) = cfg.source_lifecycle.as_ref() {
                            lifecycle.expect_terminal_close();
                        }
                        let sealed = owner
                            .stop(|| {
                                u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
                            })?
                            .expect("active WGC run must seal before capture returns");
                        break sealed.boundary.end_ms;
                    }
                    let reserved_start_ms =
                        u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                    let (_sealed, resumed) = owner.rollover_checkpoint(
                        || u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
                        reserved_start_ms,
                        || observe_wgc_start(start),
                    )?;
                    segment = resumed;
                };
                (duration_ms, Some(owner.into_publisher().checkpoints))
            }
            None => {
                let mut control = start_wgc(&src, Path::new(&path))?.control;
                while !stop.load(Ordering::Relaxed) && start.elapsed() < Duration::from_millis(dur)
                {
                    thread::sleep(Duration::from_millis(50));
                }
                let duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                if let Some(lifecycle) = cfg.source_lifecycle.as_ref() {
                    lifecycle.expect_terminal_close();
                }
                control.close()?;
                (duration_ms, None)
            }
        };
        stop.store(true, Ordering::Relaxed);
        let (cursor, mut clicks, scrolls, keys) = input.seal(duration_ms)?;
        let (source_path, verified_media) = if let Some(checkpoints) = checkpoints.as_ref() {
            let (source, media) =
                checkpoints.stitch(&ffmpeg_bin(), &ffprobe_bin(), "source.mp4")?;
            (source.display().to_string(), Some(media))
        } else {
            (path, None)
        };

        let (audio, microphone_outcome) = crate::microphone_result::finish(cfg.audio, mic_handle);
        let coordinates = if cfg.window.is_some() {
            surface_coordinates::unavailable_window_rdevin_input(cursor, &mut clicks, scrolls)
        } else {
            surface_coordinates::map_rdevin_input(surface, w, h, cursor, &mut clicks, scrolls)
        };
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
            cursor: coordinates.cursor,
            cursor_correlation: coordinates.correlation,
            clicks,
            scrolls: coordinates.scrolls,
            keys,
        };
        Ok(CaptureOutput {
            source_video: source_path,
            events,
            camera_artifact: None,
            webcam_video: None,
            audio,
            microphone_outcome,
            settings: Settings {
                width: w,
                height: h,
                fps: fps as f32,
                audio_rate: 48_000,
            },
            capture_quality: None,
            verified_media,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{capture_fps, even_capture_dimension};

    #[test]
    fn checked_capture_numbers_reject_or_bound_invalid_values() {
        assert_eq!(even_capture_dimension(-1), None);
        assert_eq!(even_capture_dimension(1), None);
        assert_eq!(even_capture_dimension(3), Some(2));
        assert_eq!(capture_fps(f64::NAN), 30);
        assert_eq!(capture_fps(0.1), 1);
        assert_eq!(capture_fps(500.0), 240);
    }
}

//! windows.rs — live screen + input capture on Windows.
//!
//! Compiled ONLY for `cfg(windows)` + the `capture-windows` feature.
//! - SCREEN: windows-capture (Windows Graphics Capture) → MP4 via its built-in
//!   Media Foundation encoder (no ffmpeg). Monitor capture omits the OS cursor
//!   for synthetic polish; window capture keeps one native cursor while exact clicks
//!   use separately attested physical window geometry.
//! - INPUT: the shared rdevin hook (see input.rs).
//!
//! When running Windows capture tests from WSL, launch the built Windows app via
//! `cmd.exe /c "C:\…\cutd.exe …"`; a direct WSL-interop UNC working dir breaks
//! WGC dispatcher init (0x80070490).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
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
    region_geometry::NativePixelCrop,
    surface_coordinates,
    windows_wgc_run::{
        WgcAcceptedCapture, WgcCaptureRange, WgcNativeControl, WgcStartObservation,
        WgcStartedControl,
    },
    Capture, CaptureConfig, CaptureOutput, MonitorInfo, WindowInfo,
};

pub(crate) fn even_capture_dimension(value: i32) -> Option<u32> {
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
            let (win, width, height) = crate::windows_capture_source::window(
                window_id,
                crate::windows_capture_source::WindowResolution::RecordingStart,
            )?;
            (Src::Window(win), width, height, None)
        } else {
            let monitor =
                crate::windows_capture_source::monitor(cfg.monitor, cfg.monitor_id.as_deref())?;
            let width = monitor
                .width()
                .map_err(|error| cap_err("monitor width", error))?;
            let height = monitor
                .height()
                .map_err(|error| cap_err("monitor height", error))?;
            let surface = monitor_surface(&monitor);
            (Src::Monitor(monitor), width, height, surface)
        };
        crate::windows_controller_exclusion::admit_controller_placement(
            cfg.controller_placement.as_ref(),
            matches!(src, Src::Window(_)),
        );
        if let Some(preview) = cfg.active_preview.as_ref() {
            if matches!(src, Src::Window(_)) {
                preview.set_controller_safety(
                    crate::active_capture_preview::RecursionStatus::Possible,
                    crate::active_capture_preview::ControllerExclusionStatus::NotApplicable,
                );
            } else {
                let excluded = cfg.controller_placement.as_ref().is_some_and(|placement| {
                    placement.status().state == crate::CaptureControllerPlacementState::Excluded
                });
                preview.set_controller_safety(
                    if excluded {
                        crate::active_capture_preview::RecursionStatus::None
                    } else {
                        crate::active_capture_preview::RecursionStatus::Possible
                    },
                    if excluded {
                        crate::active_capture_preview::ControllerExclusionStatus::ConfirmedExcluded
                    } else {
                        crate::active_capture_preview::ControllerExclusionStatus::NotConfirmed
                    },
                );
            }
            preview.enable();
        }

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
        // The owner seals immediately after WGC's measured final-video boundary;
        // any early capture error drops it and performs best-effort native cleanup.
        let window_hwnd = match src {
            Src::Window(win) => Some(HWND(win.as_raw_hwnd())),
            Src::Monitor(_) => None,
        };
        let mic_gate = cfg
            .audio
            .then(|| Arc::new(crate::mic::MicRecordingGate::default()));
        let anchor = crate::windows_first_frame::WindowsFirstFrameAnchor::new(cfg.clock.clone())
            .with_mic_gate(mic_gate.clone());
        let (input, window_clicks) = crate::windows_input::start_deferred(
            cfg,
            window_hwnd,
            w,
            h,
            anchor.input_origin_cell(),
        )?;
        anchor.set_input_seed(input.seed_handle());
        let mic_handle = if cfg.audio {
            let ready = Arc::new(AtomicBool::new(false));
            let mic_path = format!("{out_dir}/mic.wav");
            Some(crate::mic_endpoint::spawn_microphone_capture_with_gate(
                mic_path,
                cfg.microphone_source.clone(),
                stop.clone(),
                ready,
                Instant::now(),
                mic_gate,
                cfg.microphone_level.clone(),
            ))
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
        let current_wgc_timing = Arc::new(Mutex::new(None));
        let factory_wgc_timing = current_wgc_timing.clone();
        let mut start_wgc = |target: &Src,
                             destination: &Path,
                             reserved_start_ms: u64|
         -> Result<WgcStartedControl<LiveWgcControl>> {
            let started_origin = anchor.qpc_origin();
            let timed_clock = started_origin
                .map(|(start, origin)| {
                    origin
                        .reserved_boundary_hns(reserved_start_ms)
                        .map(|boundary| (start, origin, boundary))
                        .map_err(|error| cap_err("reserve WGC native sample clock", error))
                })
                .transpose()?;
            let first_frame = started_origin.is_none().then(|| anchor.clone());
            let source_lifecycle = cfg.source_lifecycle.clone();
            let timing = match started_origin {
                Some((start, _)) => {
                    crate::windows_wgc_timing::WgcTimingRecorder::new(start, fps, destination)
                }
                None => {
                    crate::windows_wgc_timing::WgcTimingRecorder::new_deferred(fps, destination)
                }
            };
            *factory_wgc_timing
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(timing.clone());
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
                timing: Some(timing.clone()),
                timed_clock,
                first_frame,
                window_clicks: window_clicks.clone(),
                active_preview: cfg.active_preview.as_ref().and_then(|preview| {
                    preview
                        .begin_segment()
                        .map(|generation| (preview.clone(), generation))
                }),
            };
            let preview_on_close = cfg.active_preview.clone();
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
                    CursorCaptureSettings::WithCursor,
                    DrawBorderSettings::Default,
                    SecondaryWindowSettings::Default,
                    MinimumUpdateIntervalSettings::Default,
                    DirtyRegionSettings::Default,
                    ColorFormat::Rgba8,
                    flags,
                )),
            }
            .map_err(|e| cap_err("start capture", e))?;
            let rotation_callback = control.callback();
            let closing_callback = control.callback();
            Ok(WgcStartedControl::new(
                LiveWgcControl {
                    rotate: Some(Box::new(move |next| {
                        rotation_callback.lock().rotate_writer(next)
                    })),
                    close: Some(Box::new(move |boundary| {
                        if let Some(preview) = preview_on_close.as_ref() {
                            preview.clear_current_generation();
                        }
                        let timing = closing_callback.lock().current_timing();
                        let result = crate::windows_wgc_handler::stop_with_held_sample(
                            control,
                            boundary.unwrap_or_else(Instant::now),
                        );
                        if let Some(timing) = timing {
                            timing.control_stopped(Instant::now(), result.is_ok());
                            if let Err(error) = timing.persist() {
                                eprintln!("WGC timing observation could not be retained: {error}");
                            }
                        }
                        result
                    })),
                    source_lifecycle,
                },
                accepted.clone(),
            ))
        };
        let (duration_ms, checkpoints) = match Checkpoints::open(cfg.checkpoint.as_ref())? {
            Some(checkpoints) => {
                let (duration, checkpoints) = crate::windows_continuous::capture(
                    cfg,
                    &stop,
                    &anchor,
                    &accepted.settings,
                    checkpoints,
                    |destination, reserved_start_ms| {
                        let started = start_wgc(&src, destination, reserved_start_ms)?;
                        let timing = current_wgc_timing
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .as_ref()
                            .cloned()
                            .ok_or_else(|| {
                                cap_err("rotate recording writer", "capture timing missing")
                            })?;
                        Ok((started.control, timing))
                    },
                )?;
                (duration, Some(checkpoints))
            }
            None => {
                let mut control = start_wgc(&src, Path::new(&path), 0)?.control;
                let Some((start, _)) = anchor.wait_started(&stop, || Ok(()))? else {
                    stop.store(true, Ordering::Release);
                    let _ = control.close();
                    return Err(cap_err(
                        "capture screen",
                        "capture stopped before the first video frame arrived",
                    ));
                };
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
            let (source, media, notes) =
                checkpoints.stitch(&ffmpeg_bin(), &ffprobe_bin(), "source.mp4")?;
            crate::capture_notes::publish(std::path::Path::new(&out_dir), &notes);
            (source.display().to_string(), Some(media))
        } else {
            (path, None)
        };

        let (audio, microphone_outcome) = crate::microphone_result::finish(cfg.audio, mic_handle);
        let coordinates = if let Some(window_clicks) = window_clicks {
            window_clicks.finish(duration_ms, &mut clicks)
        } else if cfg.window.is_some() {
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

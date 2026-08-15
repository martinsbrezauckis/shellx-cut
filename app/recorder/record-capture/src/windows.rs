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

use crate::{
    checkpoint::Checkpoints, input, surface_coordinates, Capture, CaptureConfig, CaptureOutput,
    MonitorInfo, WindowInfo,
};

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

/// The global desktop rectangle WGC captures for this monitor. rdevin's low-level
/// hook reports this desktop coordinate space, so an exact transform needs it.
fn monitor_surface(monitor: &WcMonitor) -> Option<surface_coordinates::CaptureSurface> {
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
        crate::windows_runtime::pin_process_mta()
            .map_err(|error| cap_err("initialize Windows capture runtime", error))?;
        // Open-ended capture: `None` = run until the external `stop` is set
        // (huge cap so only `stop` ends it); a concrete `duration_ms` is an upper
        // bound. The wait loop below polls `stop` 10×/s so `screen_record.stop` ends
        // the WGC capture promptly.
        let dur = cfg.duration_ms.unwrap_or(u64::MAX / 4);
        let fps = capture_fps(cfg.fps);

        // Resolve the capture SOURCE. A specific window (by title) takes precedence
        // over a monitor; else the chosen monitor; else primary. WGC accepts either a
        // Window or a Monitor as the capture item (both impl TryInto<…ItemType>).
        #[derive(Clone, Copy)]
        enum Src {
            Monitor(WcMonitor),
            Window(WcWindow),
        }
        let (src, w, h, surface) = if let Some(ref title) = cfg.window {
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
            // WGC does not expose timestamped window rectangles from this encoder
            // callback. A startup rect becomes stale on move/resize, so window
            // pointer positions stay unavailable until that provenance exists.
            (Src::Window(win), ww, wh, None)
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
            let surface = monitor_surface(&monitor);
            (Src::Monitor(monitor), mw, mh, surface)
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
        let mic_handle = if cfg.audio {
            let ready = Arc::new(AtomicBool::new(false));
            Some(crate::mic::spawn_mic(
                format!("{out_dir}/mic.wav"),
                stop.clone(),
                ready,
                start,
            ))
        } else {
            None
        };

        // Clock + input + screen all begin together, aligned with the live mic.
        let input = input::spawn_listener(start, stop.clone(), cfg.capture_keys);

        // A WGC encoder writes an open MP4 until `CaptureControl::stop()` joins its
        // thread and Handler::on_closed calls `VideoEncoder::finish()`. Rotating by
        // stopping/joining and then starting a fresh WGC control is therefore the only
        // safe checkpoint boundary. The elapsed clock includes the bounded restart gap;
        // each manifest fact keeps that event offset so concat preserves the gap.
        let mut checkpoints = Checkpoints::open(cfg.checkpoint.as_ref())?;
        let mut segment_start_ms;
        let mut segment = checkpoints
            .as_mut()
            .map(|c| c.begin_windows_wgc(0))
            .transpose()?
            .map(|(sequence, staging)| (Some(sequence), staging.display().to_string()))
            .unwrap_or((None, path.clone()));
        let start_wgc = |destination: String| {
            let flags = EncFlags {
                w,
                h,
                fps,
                path: destination,
            };
            match src {
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
            .map_err(|e| cap_err("start capture", e))
        };
        let mut control = start_wgc(segment.1.clone())?;
        // The initial WGC start is the first encoder-start timestamp; do not use
        // the manifest reservation's pre-spawn value for timeline facts.
        segment_start_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        let duration_ms = loop {
            let interval_end = checkpoints
                .as_ref()
                .map(|c| segment_start_ms.saturating_add(c.interval_ms()))
                .unwrap_or(dur);
            let end_at = interval_end.min(dur);
            while !stop.load(Ordering::Relaxed) && start.elapsed() < Duration::from_millis(end_at) {
                thread::sleep(Duration::from_millis(50));
            }
            // This is the last elapsed instant the old encoder was accepting frames;
            // finalization and the new WGC start below are a measured restart gap.
            let capture_end_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            // `stop()` joins the WGC thread, so `on_closed` has completed `finish()`.
            control
                .stop()
                .map_err(|e| cap_err("finalize WGC checkpoint", e))?;
            let ended_ms = capture_end_ms;
            if stop.load(Ordering::Relaxed) || ended_ms >= dur {
                if let (Some(checkpoints), Some(sequence)) = (checkpoints.as_mut(), segment.0) {
                    checkpoints.publish(
                        sequence,
                        Path::new(&segment.1),
                        record_recovery::CheckpointFacts {
                            start_ms: segment_start_ms,
                            end_ms: ended_ms,
                            event_offset_ms: segment_start_ms,
                            // Mic timing is not available at segment-finalize time;
                            // the Windows system sidecar publishes its real packet
                            // offset separately before the normal receipt.
                            audio_offset_ms: None,
                        },
                    )?;
                }
                break ended_ms;
            }
            // Publish before reserving another output: one manifest may own exactly
            // one open segment. The verification time is a real measured restart
            // gap, not hidden behind a pre-opened next checkpoint.
            if let (Some(checkpoints), Some(sequence)) = (checkpoints.as_mut(), segment.0) {
                checkpoints.publish(
                    sequence,
                    Path::new(&segment.1),
                    record_recovery::CheckpointFacts {
                        start_ms: segment_start_ms,
                        end_ms: ended_ms,
                        event_offset_ms: segment_start_ms,
                        audio_offset_ms: None,
                    },
                )?;
            }
            let reserved_start_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            let next_segment = checkpoints
                .as_mut()
                .expect("checkpoint capture remains configured")
                .begin_windows_wgc(reserved_start_ms)
                .map(|(sequence, staging)| (Some(sequence), staging.display().to_string()))?;
            let next_control = start_wgc(next_segment.1.clone())?;
            // The returned WGC start is the new encoder boundary; a prior end or
            // reservation timestamp would shorten the stitched wall clock.
            let next_start_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            segment_start_ms = next_start_ms;
            segment = next_segment;
            control = next_control;
        };
        stop.store(true, Ordering::Relaxed);
        let source_path = if let Some(checkpoints) = checkpoints.as_ref() {
            checkpoints
                .stitch(&ffmpeg_bin(), &ffprobe_bin(), "source.mp4")?
                .display()
                .to_string()
        } else {
            path
        };

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
        let (cursor, mut clicks, scrolls, keys) = input.lock().unwrap().snapshot();
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

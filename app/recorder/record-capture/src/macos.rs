//! macos.rs — live screen + input capture on macOS.
//!
//! Compiled ONLY for `cfg(target_os = "macos")` + the `capture-macos` feature.
//! - SCREEN: ffmpeg's `avfoundation` input device with `-capture_cursor 0` (hides
//!   the OS cursor — we re-render synthetic). Chosen over hand-rolling
//!   ScreenCaptureKit → AVAssetWriter: far simpler, robust, same "shell to ffmpeg"
//!   approach as the rest of the pipeline. ffmpeg must be on PATH (or
//!   SHELLX_RECORD_FFMPEG) and built with avfoundation.
//! - INPUT: the shared rdevin hook (see input.rs).
//!
//! PERMISSIONS (TCC): the host process needs Screen Recording (for the capture)
//! and Accessibility (for the input hook) granted in System Settings, else capture
//! fails / input is silently empty.
//!
//! COORDINATES: rdevin reports global desktop points while ScreenCaptureKit records
//! physical pixels. A validated native surface transform maps them before they are
//! considered exact; absent/outside geometry is deliberately unavailable.

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use record_core::{error_codes, EventTrack, Monitor as RMonitor, RecordError, Result, Settings};

use crate::macos_finalization::stop_audio_at_video_boundary;
use crate::macos_system_tap::{SystemAudioResult, SystemAudioTap};
use crate::{
    checkpoint::Checkpoints, input, macos_checkpoint::SegmentOutput, surface_coordinates, Capture,
    CaptureConfig, CaptureOutput, MonitorInfo, WindowInfo,
};

// ScreenCaptureKit (the Mac counterpart to windows-capture/WGC). The whole module is
// cfg(all(target_os="macos", feature="capture-macos")) so the screencapturekit dep is
// always present here — no per-item cfg needed.
use screencapturekit::prelude::*;

/// SCK requires CoreGraphics to be initialised before `SCShareableContent` is touched
/// off the main thread, else it aborts with CGS_REQUIRE_INIT. The crate ships a tiny
/// C shim for exactly this; call it once at the top of every SCK entry point.
pub(crate) fn sck_init_cg() {
    extern "C" {
        fn sc_initialize_core_graphics();
    }
    // SAFETY: the shim takes no arguments, retains no Rust state, and is designed
    // to be called repeatedly before ScreenCaptureKit entry points.
    unsafe { sc_initialize_core_graphics() }
}

fn one_based_index(position: usize) -> Option<u32> {
    u32::try_from(position).ok()?.checked_add(1)
}

/// Enumerate on-screen application windows for the in-app picker (the Mac arm of
/// [`crate::list_windows`], mirroring windows.rs). Real top-level app windows only:
/// titled, layer 0 (skips the menubar/Dock/desktop overlays), not our own process.
/// Minimized windows are KEPT because SCWindow still lists them. Needs
/// Screen-Recording TCC consent, else SCK returns an empty/partial set.
pub(crate) fn list_windows() -> Vec<WindowInfo> {
    sck_init_cg();
    let Ok(content) = SCShareableContent::get() else {
        return Vec::new();
    };
    // Per-element accessors, NOT the batched .snapshot() — snapshot() in screencapturekit
    // v8.0.0 indexes a zero-len Vec and PANICS the moment there is any real content (i.e.
    // once Screen-Recording TCC is granted).
    // cutd runs as a SIDECAR child of the Tauri shell, and it's the SHELL (the parent) that
    // owns our on-screen "ShellX Cut" window — so filtering only our own pid still leaves the
    // app in the picker. Exclude the parent process too.
    extern "C" {
        fn getppid() -> i32;
    }
    let self_pid = std::process::id() as i32;
    // SAFETY: getppid takes no arguments and has no memory-safety preconditions.
    let parent_pid = unsafe { getppid() };
    let mut out: Vec<WindowInfo> = Vec::new();
    for w in content.windows() {
        if w.window_layer() != 0 {
            continue; // normal app windows live on layer 0
        }
        let title = match w.title() {
            Some(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => continue,
        };
        let app = w.owning_application();
        if app
            .as_ref()
            .map(|a| a.process_id())
            .is_some_and(|p| p == self_pid || p == parent_pid)
        {
            continue; // never offer our own windows (cutd or the owning Tauri shell)
        }
        let app_name = app.map(|a| a.application_name()).unwrap_or_default();
        let Some(id) = one_based_index(out.len()) else {
            break;
        };
        out.push(WindowInfo {
            id,
            title,
            app: app_name,
        });
    }
    out
}

/// Enumerate displays for the in-app monitor picker (the Mac arm of
/// [`crate::list_monitors`]). 1-based index matches `CaptureConfig.monitor`; the
/// capture path maps that back to the same SCShareableContent display ordering.
pub(crate) fn list_monitors_checked() -> Result<Vec<MonitorInfo>> {
    sck_init_cg();
    let content = SCShareableContent::get()
        .map_err(|e| cap_err("SCShareableContent::get", format!("{e:?}")))?;
    // Per-element accessors, NOT .snapshot() (panics on real content in v8.0.0 — see
    // list_windows).
    Ok(content
        .displays()
        .iter()
        .enumerate()
        .filter_map(|(pos, d)| {
            let index = one_based_index(pos)?;
            let (width, height) = (d.width(), d.height());
            Some(MonitorInfo {
                index,
                name: format!("Display {index} ({width}×{height})"),
                width,
                height,
                primary: pos == 0, // SCK lists the main display first (best-effort)
            })
        })
        .collect())
}

fn ffprobe_bin() -> String {
    std::env::var("SHELLX_RECORD_FFPROBE").unwrap_or_else(|_| "ffprobe".to_string())
}
fn ffmpeg_bin() -> String {
    std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string())
}

fn cap_err(ctx: &str, e: impl std::fmt::Display) -> RecordError {
    RecordError::new(error_codes::CAPTURE, ctx, e.to_string()).with_action(
        "grant Screen Recording + Accessibility in System Settings; check \
         `ffmpeg -f avfoundation -list_devices true -i \"\"`",
    )
}

fn requested_fps(fps: f64) -> u32 {
    fps.max(1.0).round() as u32
}

fn recording_stream_config(width: u32, height: u32, fps: u32) -> SCStreamConfiguration {
    SCStreamConfiguration::new()
        .with_width(width)
        .with_height(height)
        // Without a minimum frame interval, macOS 26 can emit only a short
        // initial burst for a static desktop. The checkpoint journal then
        // truthfully rejects the mismatched wall-clock interval on stitch.
        .with_fps(fps)
}

/// Probe the produced file's dimensions (avfoundation picks the display's native size).
fn probe_dims(path: &str) -> Option<(u32, u32)> {
    let out = Command::new(ffprobe_bin())
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0:s=,",
            path,
        ])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let line = s.lines().next()?.trim();
    let mut it = line.split(',');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

/// RAII guard that keeps the display awake + powered ON for a screen capture.
///
/// ScreenCaptureKit fails the recording with "failure to process first sample
/// buffer" (surfaced earlier as a bogus "no output file") when the display is
/// asleep/blanked — exactly what happens when the machine has been idle or the user
/// walks away mid-recording. A screen recorder must not depend on someone watching
/// the screen, so for the whole capture we hold an IOKit display-sleep assertion via
/// the always-present `/usr/bin/caffeinate` (no extra crate): `-u` wakes a dimmed
/// display (and the synchronous `-t 1` doubles as a settle so the panel is up before
/// SCK grabs frame 0), then a held `caffeinate -d` child prevents it sleeping again
/// until this guard drops at the end of the capture.
struct DisplayAwake(Option<std::process::Child>);

impl Drop for DisplayAwake {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn keep_display_awake() -> DisplayAwake {
    // Wake the display synchronously (turns it back on if it had blanked); the brief
    // 1 s blocks until the assertion is in effect, giving the panel time to come up.
    let _ = Command::new("caffeinate").args(["-u", "-t", "1"]).status();
    // Hold display-sleep prevention for the capture's lifetime (reaped on drop).
    let child = Command::new("caffeinate").arg("-d").spawn().ok();
    DisplayAwake(child)
}

/// Live macOS capture backend.
pub struct MacCapture;

impl MacCapture {
    pub fn new() -> Self {
        Self
    }
}

impl Capture for MacCapture {
    fn capture(&self, cfg: &CaptureConfig, stop: Arc<AtomicBool>) -> Result<CaptureOutput> {
        // The external `stop` flag is shared with screen, mic, and input capture.
        // With no duration, capture remains open until that flag is set. A bounded
        // capture stops at its deadline. The spawned screen process is interrupted
        // and finalized on either path, matching the Linux capture lifecycle.
        let bounded_ms = cfg.duration_ms; // None ⇒ record until stop
        let fps = requested_fps(cfg.fps);

        let out_dir = cfg.out_dir.trim_end_matches('/').to_string();
        std::fs::create_dir_all(&out_dir).map_err(|e| cap_err("create output dir", e))?;
        let path = format!("{out_dir}/source.mp4");

        // Keep the screen awake + on for the entire capture (held until this fn returns)
        // so SCK always has frames to record — see [`DisplayAwake`]. This is the fix for
        // the "recording produced an error" seen when the machine had gone idle.
        let _display_awake = keep_display_awake();

        // ScreenCaptureKit capture (the Mac counterpart to WGC). Honors cfg.window (TRUE
        // per-window capture) else the chosen display, and records straight
        // to source.mp4 via SCRecordingOutput (macOS 15+). The external
        // stop / deadline ends it → stream.stop_capture() finalizes the mp4.
        use screencapturekit::shareable_content::SCShareableContentInfo;

        sck_init_cg();
        let content = SCShareableContent::get()
            .map_err(|e| cap_err("SCShareableContent::get", format!("{e:?}")))?;

        // Build the filter via the PER-ELEMENT accessors (NOT the batched snapshot(), which
        // panics on real content in v8.0.0): a specific window (matched by title, like
        // the Windows path) or the chosen display, plus a fallback point size.
        let windows = content.windows();
        let displays = content.displays();
        let (filter, fb_w, fb_h, surface) = if let Some(ref want) = cfg.window {
            let win = windows
                .iter()
                .find(|w| w.title().is_some_and(|t| t.contains(want.as_str())))
                .ok_or_else(|| {
                    cap_err("find the window to capture", "no window matches the title")
                })?;
            let fr = win.frame();
            (
                SCContentFilter::create().with_window(win).build(),
                fr.size.width as u32,
                fr.size.height as u32,
                // RecordingOutput gives no capture-clock window geometry samples;
                // a launch-time frame would become false-exact after move/resize.
                None,
            )
        } else {
            // cfg.monitor is the 1-based index from list_monitors(); map to the same ordering.
            let idx = cfg
                .monitor
                .and_then(|m| usize::try_from(m).ok())
                .map(|m| m.saturating_sub(1))
                .unwrap_or(0);
            let disp = displays
                .get(idx)
                .or_else(|| displays.first())
                .ok_or_else(|| cap_err("select a display", "no displays available"))?;
            let fr = disp.frame();
            (
                SCContentFilter::create()
                    .with_display(disp)
                    .with_excluding_windows(&[])
                    .build(),
                disp.width(),
                disp.height(),
                surface_coordinates::CaptureSurface::new(
                    fr.origin.x,
                    fr.origin.y,
                    fr.size.width,
                    fr.size.height,
                ),
            )
        };

        // Native pixel size handles Retina capture buffers; fall back to
        // the snapshot point size. Even dims for the H.264 encoder.
        let (cap_w, cap_h) = SCShareableContentInfo::for_filter(&filter)
            .map(|i| i.pixel_size())
            .filter(|(w, h)| *w > 0 && *h > 0)
            .unwrap_or((fb_w.max(2), fb_h.max(2)));
        // The main stream stays video-only. `capturesAudio` on this
        // SCRecordingOutput stream can stall video delivery and produce no audio
        // buffers. SCRecordingOutput + capturesAudio is therefore not used. DESKTOP/
        // SYSTEM audio is instead captured by a SEPARATE audio-only SCStream started below (the
        // canonical SCK pattern from the crate README: capturesAudio + an Audio output handler,
        // NO SCRecordingOutput), so the video path remains independent.
        let requested_w = cap_w & !1;
        let requested_h = cap_h & !1;
        let stream_config = recording_stream_config(requested_w, requested_h, fps);

        let stream = SCStream::new(&filter, &stream_config);
        let mut checkpoints = Checkpoints::open(cfg.checkpoint.as_ref())?;
        let mut segment = checkpoints
            .as_mut()
            .map(|owner| owner.begin(0))
            .transpose()?;
        let segment_path = segment
            .as_ref()
            .map(|(_, path)| path.clone())
            .unwrap_or_else(|| std::path::PathBuf::from(&path));
        let mut recording =
            SegmentOutput::new(&segment_path).map_err(|e| cap_err("create recording output", e))?;
        stream
            .add_recording_output(recording.output())
            .map_err(|e| cap_err("attach recording output", format!("{e:?}")))?;
        stream
            .start_capture()
            .map_err(|e| cap_err("start ScreenCaptureKit capture", format!("{e:?}")))?;
        // Open the shared clock only after SCK accepted the output.  Mic/input
        // are deliberately non-blocking, but their timestamps must use this
        // same origin as the later checkpoint facts and external audio worker.
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
        let input = input::spawn_listener(start, stop.clone(), cfg.capture_keys);
        // SCK start returning is the first encoder-start boundary available from
        // this API. The journal's open reservation is intentionally not reused as
        // a capture timestamp.
        let mut segment_start_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

        // Desktop/system audio runs through the Core Audio process tap (mac_systemaudio.mm),
        // started in parallel with the video. Returns an opaque ctx pointer (null on failure). A
        // failure must NOT take down the running video capture — we just log + carry on mic-only.
        // The pointer is used + freed on THIS thread only (raw, not Send — never moved off-thread).
        let mut sys_tap = if cfg.system_audio {
            let tap_start_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            let tap = SystemAudioTap::start(tap_start_ms);
            let _ = std::fs::write(
                format!("{out_dir}/sysaudio.debug"),
                format!("coreaudio_tap_started={}\n", tap.is_some()),
            );
            tap
        } else {
            None
        };

        // The stopped tap payload is held in memory while video checkpoint work
        // completes. This freezes its real PCM at the capture-clock boundary;
        // publishing the WAV later never extends it with stitch time.
        let mut stopped_system_audio: Option<SystemAudioResult> = None;

        // Rotate a detached, fully-finalized `SCRecordingOutput`; the stream itself
        // stays live. This is the SCK equivalent of WGC encoder rotation.
        let duration_ms = loop {
            let full_end = bounded_ms.unwrap_or(u64::MAX / 4);
            let checkpoint_end = checkpoints
                .as_ref()
                .map(|owner| segment_start_ms.saturating_add(owner.interval_ms()))
                .unwrap_or(full_end)
                .min(full_end);
            while !stop.load(Ordering::Relaxed)
                && start.elapsed() < Duration::from_millis(checkpoint_end)
            {
                thread::sleep(Duration::from_millis(50));
            }
            // This is the last elapsed instant the current output can contain. The
            // asynchronous SCK close that follows is a real, padded restart gap.
            let capture_end_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            let final_segment =
                stop.load(Ordering::Relaxed) || start.elapsed() >= Duration::from_millis(full_end);
            if final_segment {
                // Stop native video and Core Audio as one capture-end boundary.
                // `wait_complete`/checkpoint probing/stitching may take seconds on
                // a 4K sparse desktop and must not become recorded audio.
                stop.store(true, Ordering::Relaxed); // end mic + input at this boundary
                stopped_system_audio = stop_audio_at_video_boundary(
                    || {
                        let _ = stream.stop_capture();
                    },
                    &mut sys_tap,
                    SystemAudioTap::finish,
                );
            }
            let _ = stream.remove_recording_output(recording.output());
            recording
                .wait_complete()
                .map_err(|e| cap_err("finalize ScreenCaptureKit checkpoint", e))?;
            if final_segment {
                if let (Some(owner), Some((sequence, staging))) =
                    (checkpoints.as_mut(), segment.take())
                {
                    owner.publish(
                        sequence,
                        &staging,
                        record_recovery::CheckpointFacts {
                            start_ms: segment_start_ms,
                            end_ms: capture_end_ms,
                            event_offset_ms: segment_start_ms,
                            // Neither mic nor Core Audio tap exposes a proven
                            // first-packet offset at this video-finalize boundary.
                            audio_offset_ms: None,
                        },
                    )?;
                }
                break capture_end_ms;
            }
            let completed = segment.take();
            // Seal the only open segment before reserving another. Completion, probe,
            // and publication delay are deliberately reflected as a stitched gap.
            if let (Some(owner), Some((sequence, staging))) = (checkpoints.as_mut(), completed) {
                owner.publish(
                    sequence,
                    &staging,
                    record_recovery::CheckpointFacts {
                        start_ms: segment_start_ms,
                        end_ms: capture_end_ms,
                        event_offset_ms: segment_start_ms,
                        audio_offset_ms: None,
                    },
                )?;
            }
            let reserved_start = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            let next = checkpoints
                .as_mut()
                .expect("checkpoint rotation configured")
                .begin(reserved_start)?;
            let next_path = next.1.clone();
            let next_recording = SegmentOutput::new(&next_path)
                .map_err(|e| cap_err("create rotated recording output", e))?;
            stream
                .add_recording_output(next_recording.output())
                .map_err(|e| cap_err("attach rotated recording output", format!("{e:?}")))?;
            // `add_recording_output` returning is the SCK encoder-start boundary.
            let next_start = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            segment_start_ms = next_start;
            segment = Some(next);
            recording = next_recording;
        };
        let source_path = if let Some(owner) = checkpoints.as_ref() {
            owner
                .stitch(&ffmpeg_bin(), &ffprobe_bin(), "source.mp4")?
                .display()
                .to_string()
        } else {
            path.clone()
        };

        // Flush the Core Audio payload stopped at the video boundary to
        // `<out_dir>/system.wav`. `sxc_sysaudio_stop` tears the tap down and hands back malloc'd
        // interleaved f32 PCM (+ channel count + sample rate); we convert to 16-bit and write the
        // WAV via hound. Only write when we actually captured samples — an empty file would mask
        // "no desktop audio was playing" and block the cutd orchestrator's fall-back. The polish
        // pass picks this up as the `a_system` track, trimmed to the video length.
        if let Some(result) = stopped_system_audio {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(format!("{out_dir}/sysaudio.debug"))
                .map(|mut f| {
                    use std::io::Write;
                    let _ = writeln!(
                        f,
                        "coreaudio_stop rc={} samples={} channels={} rate={} first_packet_offset_ms={:?}",
                        result.rc,
                        result.count,
                        result.channels,
                        result.rate,
                        result.first_packet_offset_ms,
                    );
                });
            if result.rc == 0 {
                let first_packet_offset_ms = result.first_packet_offset_ms;
                if let Some(samples) = result.samples {
                    let ch = result.channels.clamp(1, 8) as u16;
                    let sr = if result.rate.is_finite()
                        && (8_000.0..=384_000.0).contains(&result.rate)
                    {
                        result.rate.round() as u32
                    } else {
                        48_000
                    };
                    if let Err(error) = crate::macos_system_audio::publish_padded_system_wav(
                        std::path::Path::new(&out_dir),
                        samples.as_slice(),
                        ch,
                        sr,
                        first_packet_offset_ms,
                    ) {
                        eprintln!("warning: {error}");
                    }
                }
            }
        }

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
        // `SCStreamConfiguration` pins the requested physical frame dimensions. If
        // ffprobe is unavailable, use that known negotiated target rather than a
        // made-up 1920×1080 transform.
        let (w, h) = probe_dims(&source_path).unwrap_or((requested_w, requested_h));
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
    use super::{recording_stream_config, requested_fps};

    #[test]
    fn recording_stream_config_preserves_requested_static_desktop_rate() {
        let config = recording_stream_config(1920, 1080, requested_fps(29.6));
        assert_eq!(config.fps(), 30);
        assert_eq!(requested_fps(0.0), 1);
    }
}

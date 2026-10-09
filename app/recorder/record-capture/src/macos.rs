//! macos.rs — live screen + input capture on macOS.
//!
//! Compiled ONLY for `cfg(target_os = "macos")` + the `capture-macos` feature.
//! - SCREEN: ScreenCaptureKit direct-to-file capture. Its stream configuration
//!   respects `CaptureConfig.capture_cursor` for displays; window capture keeps
//!   the OS cursor because its input geometry cannot support a synthetic cursor.
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

use crate::active_capture_preview::{
    ActiveCapturePreview, ControllerExclusionStatus, RecursionStatus,
};
use crate::macos_finalization::stop_audio_at_video_boundary;
use crate::macos_region_capture::verified_region_output_size;
use crate::macos_system_tap::{SystemAudioResult, SystemAudioTap};
use crate::{
    checkpoint::Checkpoints, macos_checkpoint::SegmentOutput, surface_coordinates, Capture,
    CaptureConfig, CaptureOutput, MonitorInfo, WindowInfo,
};

// ScreenCaptureKit (the Mac counterpart to windows-capture/WGC). The whole module is
// cfg(all(target_os="macos", feature="capture-macos")) so the screencapturekit dep is
// always present here — no per-item cfg needed.
use screencapturekit::prelude::*;
use screencapturekit::{
    error::{SCError, SCStreamErrorCode},
    stream::StreamCallbacks,
};

/// A failed start or early return must not leave an old frame attached to a
/// live capture reservation. Explicit Stop clears before output finalization.
struct PreviewClearGuard(Option<ActiveCapturePreview>);

impl PreviewClearGuard {
    fn clear(&mut self) {
        if let Some(preview) = self.0.take() {
            preview.clear_current_generation();
        }
    }
}

impl Drop for PreviewClearGuard {
    fn drop(&mut self) {
        self.clear();
    }
}

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
    // Only an explicit marker from the immediate Tauri parent admits a parent
    // as our controller. An adopted engine must not hide its Terminal or an
    // unrelated launcher merely because it happens to be the OS parent.
    let self_pid = std::process::id() as i32;
    let controller_owner_pid = crate::macos_capture_target::admitted_controller_owner_pid();
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
            .is_some_and(|p| p == self_pid || Some(p) == controller_owner_pid)
        {
            continue; // never offer cutd or the admitted owning Tauri shell
        }
        let app_name = app.map(|a| a.application_name()).unwrap_or_default();
        out.push(WindowInfo {
            id: crate::window_target::macos_window_id(w.window_id()),
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
            // A later region picker must bind the exact native ScreenCaptureKit
            // display id. Keep the legacy picker row, but never substitute an
            // ordinal/title/geometry value if that identity is unavailable.
            let id = crate::macos_monitor_target::monitor_id(d);
            let index = one_based_index(pos)?;
            let (width, height) = (d.width(), d.height());
            Some(MonitorInfo {
                id,
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

pub(super) fn cap_err(ctx: &str, e: impl std::fmt::Display) -> RecordError {
    RecordError::new(error_codes::CAPTURE, ctx, e.to_string()).with_action(
        "grant Screen Recording + Accessibility in System Settings; check \
         `ffmpeg -f avfoundation -list_devices true -i \"\"`",
    )
}

fn requested_fps(fps: f64) -> u32 {
    record_core::backend_fps_v1(fps)
}

/// ScreenCaptureKit reports many terminal conditions through one error
/// callback. Only these typed variants identify the exact selected window as
/// gone; permission, user Stop, system teardown, encoder, and disk errors are
/// intentionally not source loss.
fn selected_window_source_lost(error: &SCError) -> bool {
    matches!(
        error,
        SCError::WindowNotFound(_)
            | SCError::SCStreamError {
                code: SCStreamErrorCode::NoCaptureSource,
                ..
            }
    )
}

pub(super) fn recording_stream_config(
    width: u32,
    height: u32,
    fps: u32,
    capture_cursor: bool,
) -> SCStreamConfiguration {
    SCStreamConfiguration::new()
        .with_width(width)
        .with_height(height)
        .with_shows_cursor(capture_cursor)
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
        sck_init_cg();
        // The active SCRecordingOutput stream stays video-only. The target
        // owner builds the exact display/window filter without opening a
        // second ScreenCaptureKit stream for controller state or metering.
        let (
            filter,
            requested_w,
            requested_h,
            surface,
            stream_config,
            region_output,
            window_clicks,
        ) = crate::macos_capture_target::prepare_capture_target(cfg, fps)?;

        let preview = cfg.active_preview.clone();
        if let Some(preview) = preview.as_ref() {
            preview.set_controller_safety(
                RecursionStatus::Possible,
                if cfg.window.is_some() {
                    ControllerExclusionStatus::NotApplicable
                } else {
                    ControllerExclusionStatus::NotConfirmed
                },
            );
            preview.enable();
        }
        let preview_generation = preview
            .as_ref()
            .and_then(ActiveCapturePreview::begin_segment);
        let mut preview_clear = PreviewClearGuard(preview.clone());

        let selected_window = cfg.window.is_some();
        let source_lifecycle = cfg.source_lifecycle.clone();
        if selected_window {
            if let Some(lifecycle) = source_lifecycle.as_ref() {
                lifecycle.arm_initial_selected_source(
                    "ScreenCaptureKit is watching the exact selected window for disappearance.",
                );
            }
        }
        let source_stop = stop.clone();
        let callback_readiness = cfg.readiness.clone();
        let callback_preview = preview.clone();
        let callback_window_clicks = window_clicks.clone();
        let delegate = StreamCallbacks::new().on_error(move |error| {
            if let Some(window_clicks) = callback_window_clicks.as_ref() {
                window_clicks.source_closed();
            }
            let _source_lost = selected_window
                && source_lifecycle.as_ref().is_some_and(|lifecycle| {
                    selected_window_source_lost(&error)
                        && lifecycle.selected_source_closed(
                            "The exact selected macOS window disappeared from ScreenCaptureKit.",
                        )
                });
            // SCK error delivery ends this stream regardless of type. Only the
            // typed exact-window transition above can publish source_lost;
            // UserStopped, permission, encoder, and other errors remain
            // ordinary terminal capture events.
            if let Some(readiness) = callback_readiness.as_ref() {
                readiness.mark_terminal();
            }
            if let Some(preview) = callback_preview.as_ref() {
                preview.clear_current_generation();
            }
            source_stop.store(true, Ordering::Release);
        });
        let mut stream = SCStream::new_with_delegate(&filter, &stream_config, delegate);
        crate::macos_readiness::attach_screen_frame_observer(
            &mut stream,
            cfg.readiness.clone(),
            preview
                .clone()
                .zip(preview_generation)
                .map(|(preview, generation)| (preview, generation, Instant::now())),
            window_clicks.clone(),
        )
        .map_err(|error| cap_err("attach ScreenCaptureKit frame observer", error))?;
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
        // SCK can synchronously deliver its first pixel-bearing Window sample
        // during start_capture. Arm that owner before requesting capture and
        // reuse this actual origin for every later input/audio/checkpoint stamp.
        let window_start = crate::macos_input::arm_window_start(window_clicks.as_ref(), cfg);
        stream
            .start_capture()
            .map_err(|e| cap_err("start ScreenCaptureKit capture", format!("{e:?}")))?;
        // Display/Region retain their post-acceptance clock policy. Window
        // input does not open until start_capture successfully accepts output.
        let (start, listener) =
            crate::macos_input::start(&mut stream, cfg, window_clicks.clone(), window_start)?;
        let mut input = Some(listener);
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
        let (duration_ms, sealed_input) = loop {
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
            let sealed_input = if final_segment {
                // Stop native video and Core Audio as one capture-end boundary.
                // `wait_complete`/checkpoint probing/stitching may take seconds on
                // a 4K sparse desktop and must not become recorded audio.
                if let Some(lifecycle) = cfg.source_lifecycle.as_ref() {
                    lifecycle.expect_terminal_close();
                }
                stop.store(true, Ordering::Relaxed); // end mic + input at this boundary
                preview_clear.clear();
                stopped_system_audio = stop_audio_at_video_boundary(
                    || {
                        let _ = stream.stop_capture();
                    },
                    &mut sys_tap,
                    SystemAudioTap::finish,
                );
                // `capture_end_ms` is the exact final-video boundary. Seal the
                // listener before recording-output completion or any stitch/audio
                // work can add late sidecar samples.
                Some(
                    input
                        .take()
                        .expect("input listener remains owned until the final segment")
                        .seal(capture_end_ms)?,
                )
            } else {
                None
            };
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
                break (
                    capture_end_ms,
                    sealed_input.expect("final segment seals its input listener"),
                );
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
        let (source_path, verified_media) = if let Some(owner) = checkpoints.as_ref() {
            let (source, media) = owner.stitch(&ffmpeg_bin(), &ffprobe_bin(), "source.mp4")?;
            (source.display().to_string(), Some(media))
        } else {
            (path.clone(), None)
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

        let (audio, microphone_outcome) = crate::microphone_result::finish(cfg.audio, mic_handle);
        // `SCStreamConfiguration` pins the requested physical frame dimensions. If
        // ffprobe is unavailable, use that known negotiated target rather than a
        // made-up 1920×1080 transform.
        let observed_dimensions = probe_dims(&source_path);
        let (w, h) = if region_output {
            verified_region_output_size((requested_w, requested_h), observed_dimensions)?
        } else {
            observed_dimensions.unwrap_or((requested_w, requested_h))
        };
        let (cursor, mut clicks, scrolls, keys) = sealed_input;
        let coordinates = if cfg.window.is_some() {
            match window_clicks.as_ref() {
                Some(owner) => owner.finish(duration_ms, (w, h), &mut clicks),
                None => surface_coordinates::unavailable_window_rdevin_input(
                    cursor,
                    &mut clicks,
                    scrolls,
                ),
            }
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
    use super::{recording_stream_config, requested_fps, selected_window_source_lost};
    use screencapturekit::error::{SCError, SCStreamErrorCode};

    #[test]
    fn recording_stream_config_preserves_requested_static_desktop_rate() {
        let config = recording_stream_config(1920, 1080, requested_fps(29.6), false);
        assert_eq!(config.fps(), 30);
        assert!(!config.shows_cursor());
        assert!(recording_stream_config(1920, 1080, 30, true).shows_cursor());
        assert_eq!(requested_fps(0.0), 1);
    }

    #[test]
    fn only_typed_selected_window_disappearance_is_source_loss() {
        assert!(selected_window_source_lost(&SCError::WindowNotFound(
            "fixture window".into()
        )));
        assert!(selected_window_source_lost(&SCError::SCStreamError {
            code: SCStreamErrorCode::NoCaptureSource,
            message: None,
        }));
        assert!(!selected_window_source_lost(&SCError::SCStreamError {
            code: SCStreamErrorCode::UserStopped,
            message: None,
        }));
        assert!(!selected_window_source_lost(&SCError::CaptureStartFailed(
            "encoder failure".into()
        )));
    }
}

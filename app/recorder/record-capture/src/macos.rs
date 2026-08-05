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
//! CAVEAT: on Retina displays rdevin reports points (logical) while avfoundation
//! captures pixels (physical); click/cursor coords may need ×backingScaleFactor.
//! Kept explicit so packaging and permission checks can report this platform state.

use std::process::Command;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use record_core::{error_codes, EventTrack, Monitor as RMonitor, RecordError, Result, Settings};

use crate::{input, Capture, CaptureConfig, CaptureOutput, MonitorInfo, WindowInfo};

// ScreenCaptureKit (the Mac counterpart to windows-capture/WGC). The whole module is
// cfg(all(target_os="macos", feature="capture-macos")) so the screencapturekit dep is
// always present here — no per-item cfg needed.
use screencapturekit::prelude::*;

/// SCK requires CoreGraphics to be initialised before `SCShareableContent` is touched
/// off the main thread, else it aborts with CGS_REQUIRE_INIT. The crate ships a tiny
/// C shim for exactly this; call it once at the top of every SCK entry point.
fn sck_init_cg() {
    extern "C" {
        fn sc_initialize_core_graphics();
    }
    // SAFETY: the shim takes no arguments, retains no Rust state, and is designed
    // to be called repeatedly before ScreenCaptureKit entry points.
    unsafe { sc_initialize_core_graphics() }
}

// Desktop/system audio uses the macOS 14.4+ Core Audio process-tap API,
// implemented in the native shim `src/mac_systemaudio.mm` (compiled by build.rs). This
// REPLACES the ScreenCaptureKit `capturesAudio` path, which silently delivers zero audio
// buffers on macOS 15+/26 (a documented SCK bug — reproduced on native macOS: the audio
// SCStream starts and captures video, but audio_calls stays 0 even with the consent granted).
// The Core Audio process tap runs in parallel
// with the proven SCK VIDEO stream; cutd's "Screen & System Audio Recording" consent authorizes
// it. `start` returns an opaque ctx (NULL on failure); `stop` hands back malloc'd interleaved
// f32 PCM that we convert to the 48 kHz-ish 16-bit `system.wav` (the a_system track).
extern "C" {
    fn sxc_sysaudio_start() -> *mut std::ffi::c_void;
    fn sxc_sysaudio_stop(
        ctx: *mut std::ffi::c_void,
        out_samples: *mut *mut f32,
        out_count: *mut u64,
        out_channels: *mut u32,
        out_rate: *mut f64,
    ) -> i32;
    fn sxc_sysaudio_free(p: *mut f32);
}

struct SystemAudioTap {
    ctx: Option<NonNull<std::ffi::c_void>>,
}

impl SystemAudioTap {
    fn start() -> Option<Self> {
        // SAFETY: the native function has no inputs and returns either null or a
        // uniquely owned context that must be consumed by sxc_sysaudio_stop.
        let ctx = NonNull::new(unsafe { sxc_sysaudio_start() })?;
        Some(Self { ctx: Some(ctx) })
    }

    fn finish(mut self) -> SystemAudioResult {
        let ctx = self.ctx.take().expect("system audio context is present");
        let mut out_ptr = std::ptr::null_mut();
        let mut count = 0;
        let mut channels = 0;
        let mut rate = 0.0;
        // SAFETY: ctx is the unique live context returned by start. Each output
        // pointer targets initialized writable storage and remains valid for the call.
        let rc = unsafe {
            sxc_sysaudio_stop(
                ctx.as_ptr(),
                &mut out_ptr,
                &mut count,
                &mut channels,
                &mut rate,
            )
        };

        let samples = if rc == 0 {
            // SAFETY: a successful native stop returns either null for zero samples
            // or a malloc allocation containing exactly count initialized f32 values.
            unsafe { SystemAudioBuffer::from_ffi(out_ptr, count) }
        } else {
            // The native contract leaves this null on failure. Free defensively if a
            // future shim returns an allocation together with an error.
            if !out_ptr.is_null() {
                // SAFETY: non-null output pointers from the shim are malloc-owned and
                // may only be released through its matching free function.
                unsafe { sxc_sysaudio_free(out_ptr) };
            }
            None
        };

        SystemAudioResult {
            rc,
            samples,
            count,
            channels,
            rate,
        }
    }
}

impl Drop for SystemAudioTap {
    fn drop(&mut self) {
        let Some(ctx) = self.ctx.take() else {
            return;
        };
        // SAFETY: ctx is still uniquely owned. Null output pointers request teardown
        // without transferring a sample buffer, which the native shim explicitly supports.
        unsafe {
            sxc_sysaudio_stop(
                ctx.as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
        }
    }
}

struct SystemAudioBuffer {
    ptr: NonNull<f32>,
    len: usize,
}

impl SystemAudioBuffer {
    unsafe fn from_ffi(ptr: *mut f32, count: u64) -> Option<Self> {
        let ptr = NonNull::new(ptr)?;
        let len = match usize::try_from(count) {
            Ok(len) if len > 0 && len <= (isize::MAX as usize) / std::mem::size_of::<f32>() => len,
            _ => {
                // SAFETY: ptr came from the native stop function and has not been freed.
                unsafe { sxc_sysaudio_free(ptr.as_ptr()) };
                return None;
            }
        };
        Some(Self { ptr, len })
    }

    fn as_slice(&self) -> &[f32] {
        // SAFETY: construction validates a non-null malloc pointer, a non-zero length,
        // and the slice size bound. The allocation remains owned by self for this borrow.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl Drop for SystemAudioBuffer {
    fn drop(&mut self) {
        // SAFETY: this is the unique pointer returned by the native shim and it is
        // released exactly once when the owning buffer drops.
        unsafe { sxc_sysaudio_free(self.ptr.as_ptr()) };
    }
}

struct SystemAudioResult {
    rc: i32,
    samples: Option<SystemAudioBuffer>,
    count: u64,
    channels: u32,
    rate: f64,
}

fn one_based_index(position: usize) -> Option<u32> {
    u32::try_from(position).ok()?.checked_add(1)
}

/// Clamp + scale a float PCM sample to 16-bit (the `system.wav`/hound target).
fn sck_f32_to_i16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * 32767.0) as i16
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

fn cap_err(ctx: &str, e: impl std::fmt::Display) -> RecordError {
    RecordError::new(error_codes::CAPTURE, ctx, e.to_string()).with_action(
        "grant Screen Recording + Accessibility in System Settings; check \
         `ffmpeg -f avfoundation -list_devices true -i \"\"`",
    )
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
        let fps = cfg.fps.max(1.0);

        let out_dir = cfg.out_dir.trim_end_matches('/').to_string();
        std::fs::create_dir_all(&out_dir).map_err(|e| cap_err("create output dir", e))?;
        let path = format!("{out_dir}/source.mp4");

        // Keep the screen awake + on for the entire capture (held until this fn returns)
        // so SCK always has frames to record — see [`DisplayAwake`]. This is the fix for
        // the "recording produced an error" seen when the machine had gone idle.
        let _display_awake = keep_display_awake();

        // Mic on its OWN thread, started in PARALLEL — it must NEVER gate the screen.
        // This previously BLOCKED up to 8 s waiting for the mic to go "ready"; on a machine
        // with no input device (or a slow permission grant) that starved the ScreenCaptureKit
        // start, so a short recording captured ZERO frames ("failure to process first sample
        // buffer") and a long one silently lost its first ~8 s of video. The Record surface
        // already pre-warms the mic via `mic::warm`, so the first-frame audio race is
        // handled up front; here we just spawn and go straight on to start the screen. With no
        // input device the mic thread returns Err → `audio` is None and the recording proceeds
        // video-only (handled at join below). `ready` is still set on first sample but no longer
        // awaited — the screen no longer waits on the microphone.
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

        let start = Instant::now();
        let input = input::spawn_listener(start, stop.clone(), cfg.capture_keys);

        // ScreenCaptureKit capture (the Mac counterpart to WGC). Honors cfg.window (TRUE
        // per-window capture) else the chosen display, and records straight
        // to source.mp4 via SCRecordingOutput (macOS 15+). The external
        // stop / deadline ends it → stream.stop_capture() finalizes the mp4.
        use screencapturekit::recording_output::{
            RecordingCallbacks, SCRecordingOutput, SCRecordingOutputCodec,
            SCRecordingOutputConfiguration, SCRecordingOutputFileType,
        };
        use screencapturekit::shareable_content::SCShareableContentInfo;
        let _ = fps; // SCK records at the display refresh; fps is kept only as the Settings hint.

        sck_init_cg();
        let content = SCShareableContent::get()
            .map_err(|e| cap_err("SCShareableContent::get", format!("{e:?}")))?;

        // Build the filter via the PER-ELEMENT accessors (NOT the batched snapshot(), which
        // panics on real content in v8.0.0): a specific window (matched by title, like
        // the Windows path) or the chosen display, plus a fallback point size.
        let windows = content.windows();
        let displays = content.displays();
        let (filter, fb_w, fb_h) = if let Some(ref want) = cfg.window {
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
            (
                SCContentFilter::create()
                    .with_display(disp)
                    .with_excluding_windows(&[])
                    .build(),
                disp.width(),
                disp.height(),
            )
        };

        // Native pixel size handles Retina capture buffers; fall back to
        // the snapshot point size. Even dims for the H.264 encoder.
        let (cap_w, cap_h) = SCShareableContentInfo::for_filter(&filter)
            .map(|i| i.pixel_size())
            .filter(|(w, h)| *w > 0 && *h > 0)
            .unwrap_or((fb_w.max(2), fb_h.max(2)));
        // The main stream stays video-only. `capturesAudio` on this
        // SCRecordingOutput stream stalls the whole capture to ~2 frames and delivers ZERO audio
        // buffers (proven on macOS 26: registered=Some, audio_calls=0, video duration 0.03 s) —
        // SCRecordingOutput + capturesAudio is a broken combination in this crate/OS. DESKTOP/
        // SYSTEM audio is instead captured by a SEPARATE audio-only SCStream started below (the
        // canonical SCK pattern from the crate README: capturesAudio + an Audio output handler,
        // NO SCRecordingOutput), so the proven video path is never put at risk.
        let stream_config = SCStreamConfiguration::new()
            .with_width(cap_w & !1)
            .with_height(cap_h & !1);

        let rec_config = SCRecordingOutputConfiguration::new()
            .with_output_url(std::path::Path::new(&path))
            .with_video_codec(SCRecordingOutputCodec::H264)
            .with_output_file_type(SCRecordingOutputFileType::MP4);

        // SCRecordingOutput writes (and finalizes the moov trailer of) the mp4
        // ASYNCHRONOUSLY: `stop_capture()` / `remove_recording_output()` return BEFORE the
        // file is on disk. A short capture (~3 s) checked the path immediately and saw nothing
        // → the spurious "ScreenCaptureKit produced no output file" (a long capture only
        // "worked" because it had time to flush). The delegate's `recording_did_finish` fires
        // when the file is fully written; `recording_did_fail` carries a real capture error.
        // We wait on these after stop (with a file-size-stability backstop) instead of racing.
        let rec_finished = Arc::new(AtomicBool::new(false));
        let rec_failed: Arc<std::sync::Mutex<Option<String>>> =
            Arc::new(std::sync::Mutex::new(None));
        let cb_fin = rec_finished.clone();
        let cb_fail = rec_failed.clone();
        let callbacks = RecordingCallbacks::new()
            .on_finish(move || cb_fin.store(true, Ordering::Relaxed))
            .on_fail(move |e| {
                if let Ok(mut g) = cb_fail.lock() {
                    *g = Some(e);
                }
            });
        let recording =
            SCRecordingOutput::new_with_delegate(&rec_config, callbacks).ok_or_else(|| {
                cap_err(
                    "create the recording output",
                    "SCRecordingOutput::new_with_delegate returned None (needs macOS 15+)",
                )
            })?;

        let stream = SCStream::new(&filter, &stream_config);
        stream
            .add_recording_output(&recording)
            .map_err(|e| cap_err("attach the recording output", format!("{e:?}")))?;
        stream
            .start_capture()
            .map_err(|e| cap_err("start ScreenCaptureKit capture", format!("{e:?}")))?;

        // Desktop/system audio runs through the Core Audio process tap (mac_systemaudio.mm),
        // started in parallel with the video. Returns an opaque ctx pointer (null on failure). A
        // failure must NOT take down the running video capture — we just log + carry on mic-only.
        // The pointer is used + freed on THIS thread only (raw, not Send — never moved off-thread).
        let sys_tap = if cfg.system_audio {
            let tap = SystemAudioTap::start();
            let _ = std::fs::write(
                format!("{out_dir}/sysaudio.debug"),
                format!("coreaudio_tap_started={}\n", tap.is_some()),
            );
            tap
        } else {
            None
        };

        // Poll the external stop + the optional deadline; SCK runs the capture on its own threads.
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            if let Some(ms) = bounded_ms {
                if start.elapsed() >= Duration::from_millis(ms) {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
        let _ = stream.stop_capture(); // requests finalize; the mp4 flush is still async
        let _ = stream.remove_recording_output(&recording);
        stop.store(true, Ordering::Relaxed); // end mic + input

        // Wait for the recording to actually land on disk before declaring success/failure.
        // PRIMARY signal: the delegate's `recording_did_finish`. BACKSTOP (covers a binding
        // that doesn't deliver the callback): the output file exists AND its size has been
        // stable for ~300 ms (the trailer write has completed). 20 s cap so a stuck finalize
        // still returns a clear error rather than hanging the recorder.
        let fin_deadline = Instant::now() + Duration::from_secs(20);
        let mut last_size = 0u64;
        let mut stable_ticks = 0u32;
        loop {
            if let Some(e) = rec_failed.lock().ok().and_then(|mut g| g.take()) {
                return Err(cap_err("screen recording failed", e));
            }
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if rec_finished.load(Ordering::Relaxed) && size > 0 {
                break; // delegate confirmed finish AND a non-empty file is present
            }
            if size > 0 && size == last_size {
                stable_ticks += 1;
                if stable_ticks >= 3 {
                    break; // file stopped growing → trailer flushed (backstop)
                }
            } else {
                stable_ticks = 0;
            }
            last_size = size;
            if Instant::now() >= fin_deadline {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }

        if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) == 0 {
            return Err(cap_err(
                "screen capture failed",
                "ScreenCaptureKit produced no output file (the recording never finalized)",
            ));
        }

        // Stop the Core Audio tap and flush its desktop/system audio to
        // `<out_dir>/system.wav`. `sxc_sysaudio_stop` tears the tap down and hands back malloc'd
        // interleaved f32 PCM (+ channel count + sample rate); we convert to 16-bit and write the
        // WAV via hound. Only write when we actually captured samples — an empty file would mask
        // "no desktop audio was playing" and block the cutd orchestrator's fall-back. The polish
        // pass picks this up as the `a_system` track, trimmed to the video length.
        if let Some(sys_tap) = sys_tap {
            let result = sys_tap.finish();
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(format!("{out_dir}/sysaudio.debug"))
                .map(|mut f| {
                    use std::io::Write;
                    let _ = writeln!(
                        f,
                        "coreaudio_stop rc={} samples={} channels={} rate={}",
                        result.rc, result.count, result.channels, result.rate
                    );
                });
            if result.rc == 0 {
                if let Some(samples) = result.samples {
                    let ch = result.channels.clamp(1, 8) as u16;
                    let sr = if result.rate.is_finite()
                        && (8_000.0..=384_000.0).contains(&result.rate)
                    {
                        result.rate.round() as u32
                    } else {
                        48_000
                    };
                    let sys_path = format!("{out_dir}/system.wav");
                    let spec = hound::WavSpec {
                        channels: ch,
                        sample_rate: sr,
                        bits_per_sample: 16,
                        sample_format: hound::SampleFormat::Int,
                    };
                    match hound::WavWriter::create(&sys_path, spec) {
                        Ok(mut w) => {
                            for &v in samples.as_slice() {
                                let _ = w.write_sample(sck_f32_to_i16(v));
                            }
                            if let Err(e) = w.finalize() {
                                eprintln!("warning: system.wav finalize failed: {e}");
                            }
                        }
                        Err(e) => eprintln!("warning: system.wav create failed: {e}"),
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
        let (w, h) = probe_dims(&path).unwrap_or((1920, 1080));
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

//! record-capture — screen + input capture abstraction.
//!
//! Role: turn "the user's screen + input" into the two artifacts the rest of the
//! pipeline needs — a source video + an `EventTrack`. The capture backends are
//! the genuinely platform-specific, permission-heavy part, so they sit behind the
//! `Capture` trait:
//!
//! - `ReplayCapture` (this crate, all platforms): reads a pre-recorded track +
//!   video from disk. Powers tests AND the "import an existing recording" path.
//! - `LiveCapture` (per-OS, behind `capture-windows` / `capture-macos` features):
//!   Windows = windows-capture (WGC) + rdevin; macOS = ScreenCaptureKit + rdevin.
//!   Camera capture is not implemented; callers must not present the optional
//!   webcam output as available.
//!
//! `doctor()` reports capability cards (mirrors ShellX Cut's system.doctor) so the
//! UI/agent can tell what's present vs needs install/permission.

mod capture_clock;
mod checkpoint;
pub mod doctor;
mod doctor_portal;
mod doctor_probe;
mod doctor_process;
mod doctor_system_audio;
mod replay;
#[cfg(any(
    test,
    all(windows, feature = "capture-windows"),
    all(target_os = "macos", feature = "capture-macos"),
    all(target_os = "linux", feature = "capture-linux")
))]
mod surface_coordinates;
#[cfg(test)]
mod surface_coordinates_tests;

// Shared Wayland coordinate transform + click/metadata matching. Kept separate
// from the portal backend so its timing and scale rules stay deterministic in tests.
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
pub mod cursor_correlation;
#[cfg(all(test, target_os = "linux", feature = "capture-linux"))]
mod cursor_correlation_tests;

// Shared rdevin input hook — compiled when any live-capture backend is active.
#[cfg(any(
    all(windows, feature = "capture-windows"),
    all(target_os = "macos", feature = "capture-macos"),
    all(target_os = "linux", feature = "capture-linux")
))]
mod input;

#[cfg(feature = "mic")]
mod macos_system_audio;
#[cfg(feature = "mic")]
mod mic;
#[cfg(feature = "mic")]
mod mic_timing;
mod system_audio_probe;
#[cfg(feature = "mic")]
mod system_audio_timing;

#[cfg(all(windows, feature = "capture-windows"))]
mod windows;
#[cfg(all(windows, feature = "capture-windows"))]
mod windows_picker;

#[cfg(all(windows, feature = "capture-windows"))]
mod windows_probe;

#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod macos;
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod macos_checkpoint;
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod macos_finalization;
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod macos_system_tap;

#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod macos_probe;

#[cfg(all(target_os = "linux", feature = "capture-linux"))]
mod linux;
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
mod linux_capture_state;
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
mod linux_media;
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
mod linux_source_publication;
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
mod linux_token;

// Native PipeWire default-sink monitor capture. This avoids assigning an
// unobservable FFmpeg/Pulse subprocess start time to the first audio packet.
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
mod linux_system_audio;
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
mod linux_system_audio_target;

// evdev input backend (Wayland) — rdevin can't hook Wayland.
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
mod input_evdev;

// Unified Wayland capture via pipewire-rs (frames + absolute cursor metadata).
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
#[doc(hidden)]
pub mod wayland_pw;

pub use capture_clock::CaptureClock;
pub use doctor::{doctor, Card};
pub use doctor_portal::{is_linux_portal_prompt_deferred, LINUX_PORTAL_PROMPT_DEFERRED_DETAIL};
pub use replay::ReplayCapture;
pub use system_audio_probe::{
    probe_system_audio, reserve_system_audio, SystemAudioLease, SystemAudioProbe, DEFAULT_WINDOW_MS,
};

/// One physical display the user can pick as the capture target.
///
/// Returned by [`list_monitors`] so the UI / agent can offer a monitor PICKER on a
/// multi-display setup. The `index` is the 1-based index the capture backend wants
/// in [`CaptureConfig::monitor`] (`WcMonitor::from_index` on Windows), so the UI can
/// pass a chosen `MonitorInfo.index` straight back into `screen_record.start`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorInfo {
    /// 1-based monitor index (what `CaptureConfig.monitor` expects).
    pub index: u32,
    /// Friendly display name (best-effort; falls back to "Monitor N" on Windows).
    pub name: String,
    /// Current mode width in pixels.
    pub width: u32,
    /// Current mode height in pixels.
    pub height: u32,
    /// True for the OS primary display.
    pub primary: bool,
}

/// One on-screen application window the user can pick as the capture target.
///
/// Returned by [`list_windows`] so the UI / agent can offer a WINDOW picker (record
/// just one app, not the whole screen). The capture backend re-resolves the window
/// from its `title` at start (`Window::from_contains_name` on Windows), so the UI
/// passes a chosen `WindowInfo.title` straight back as `CaptureConfig.window`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInfo {
    /// 1-based position in the enumeration — a stable-enough key for the UI list.
    pub id: u32,
    /// The window title (what `CaptureConfig.window` matches on).
    pub title: String,
    /// Owning process / app name, best-effort (for display, e.g. "chrome.exe").
    pub app: String,
}

/// Enumerate the displays available as capture targets, for the in-app monitor
/// PICKER. The list mirrors the backend's 1-based indexing so a chosen
/// `MonitorInfo.index` is passed straight back as `CaptureConfig.monitor`.
///
/// Platform behavior:
/// - **Windows** (`capture-windows`): real enumeration via the `windows-capture`
///   crate (`Monitor::enumerate()`), with the primary marked by comparing each
///   monitor's index to `Monitor::primary()`.
/// - **macOS** (`capture-macos`): real ScreenCaptureKit enumeration. The checked
///   variant preserves a TCC/access error so callers do not mistake a denied
///   Screen Recording permission for an available capture backend.
/// - **Linux / headless builds**: returns an EMPTY vec. On Linux the XDG
///   ScreenCast portal shows its OWN source picker at capture time, so an in-app
///   list is neither needed nor available.
pub fn list_monitors_checked() -> Result<Vec<MonitorInfo>> {
    #[cfg(all(windows, feature = "capture-windows"))]
    {
        return Ok(windows::list_monitors());
    }
    #[cfg(all(target_os = "macos", feature = "capture-macos"))]
    {
        return macos::list_monitors_checked();
    }
    #[allow(unreachable_code)]
    {
        // Linux (portal picks the source) and any build without a live-capture
        // backend: no in-app monitor list.
        Ok(Vec::new())
    }
}

/// Compatibility wrapper for callers that only need the picker rows. Capability
/// doctors should use [`list_monitors_checked`] so a permission failure remains
/// distinguishable from a platform that deliberately uses an OS source picker.
pub fn list_monitors() -> Vec<MonitorInfo> {
    list_monitors_checked().unwrap_or_default()
}

/// Enumerate the on-screen application windows available as capture targets, for the
/// in-app WINDOW picker (record a single app instead of the full screen). Mirrors
/// [`list_monitors`]'s platform behavior:
/// - **Windows** (`capture-windows`): real enumeration via `windows-capture`
///   (`Window::enumerate()`), filtered to valid, titled, non-trivial windows; the
///   chosen `WindowInfo.title` goes back as `CaptureConfig.window` and is re-resolved
///   at capture start.
/// - **macOS** (`capture-macos`): real ScreenCaptureKit enumeration.
/// - **Linux / headless**: returns an EMPTY vec (Linux's XDG portal offers window
///   selection in its own picker). An empty list means there is no in-app picker.
pub fn list_windows() -> Vec<WindowInfo> {
    #[cfg(all(windows, feature = "capture-windows"))]
    {
        return windows::list_windows();
    }
    #[cfg(all(target_os = "macos", feature = "capture-macos"))]
    {
        return macos::list_windows();
    }
    #[allow(unreachable_code)]
    {
        Vec::new()
    }
}

/// The result of [`warm_mic`]: whether the default microphone went LIVE (samples
/// flowed) within the warm window, the device name when known, and whether this
/// build has a mic backend at all. The UI calls warm_mic on entering the Record
/// surface so the OS mic-permission prompt is answered + the cpal stream is spun up
/// BEFORE the user hits record. Pure probe — opens the default input briefly, never
/// writes the recording.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicWarm {
    /// True iff the mic produced at least one audio callback within the window.
    pub live: bool,
    /// The default input device name, when the host could name it.
    pub device: Option<String>,
    /// True when this build was compiled with the `mic` feature (a mic backend).
    pub supported: bool,
}

/// Warm the default microphone for up to `max_ms` (see [`MicWarm`]). No-op stub
/// (`supported:false`) on a build without the `mic` feature.
pub fn warm_mic(max_ms: u64) -> MicWarm {
    #[cfg(feature = "mic")]
    {
        let (live, device) = mic::warm(max_ms);
        return MicWarm {
            live,
            device,
            supported: true,
        };
    }
    #[allow(unreachable_code)]
    {
        let _ = max_ms;
        MicWarm {
            live: false,
            device: None,
            supported: false,
        }
    }
}

#[cfg(all(target_os = "linux", feature = "capture-linux"))]
pub use linux_system_audio::capture_system_pipewire;
/// Native endpoint-independent WASAPI process-loopback capture (desktop/system
/// audio) → 16-bit WAV (no ffmpeg, no virtual cable). The returned metadata
/// records the first real packet offset from the caller's capture clock; the WAV
/// itself contains only WASAPI-delivered samples.
/// Windows-only; blocks until `stop` or `max_ms`. FOUNDATION for the roadmap's further
/// recording features: device SELECTION is a swap of the device resolution inside
/// `mic::capture_system_loopback`, and per-APPLICATION audio is a sibling on the process-
/// loopback API — both reuse the same WAV contract. Other OSes keep the ffmpeg
/// monitor path for now (later they fold into this native module too).
#[cfg(all(windows, feature = "mic"))]
pub use mic::capture_system_loopback;
#[cfg(feature = "mic")]
pub use system_audio_timing::SystemAudioCapture;

/// The live capture backend for this build, or `None` when none is compiled
/// (default/WSL/Linux builds). Lets the CLI compile everywhere and fail with a
/// clear message where live capture isn't available.
pub fn live_capture() -> Option<Box<dyn Capture>> {
    #[cfg(all(windows, feature = "capture-windows"))]
    {
        return Some(Box::new(windows::WindowsCapture::new()));
    }
    #[cfg(all(target_os = "macos", feature = "capture-macos"))]
    {
        return Some(Box::new(macos::MacCapture::new()));
    }
    #[cfg(all(target_os = "linux", feature = "capture-linux"))]
    {
        return Some(Box::new(linux::LinuxCapture::new()));
    }
    #[allow(unreachable_code)]
    {
        None
    }
}

use record_core::{EventTrack, RecordingProject, Result, Settings};
use serde::{Deserialize, Serialize};

/// Debug probe: run ONLY the evdev input listener for `seconds` and return the
/// (clicks, keys, cursor, scrolls) sample counts. No portal/screen capture — lets us
/// verify `/dev/input` reading in isolation (e.g. under sudo, any session type).
/// Returns None on non-Linux-capture builds.
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
pub fn evdev_probe(seconds: u64, capture_keys: bool) -> Option<(usize, usize, usize, usize)> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    let stop = Arc::new(AtomicBool::new(false));
    let start = std::time::Instant::now();
    let input = input_evdev::spawn_evdev_listener(start, stop.clone(), capture_keys, 1920, 1080);
    eprintln!(">>> evdev probe: generate input NOW for {seconds}s <<<");
    std::thread::sleep(std::time::Duration::from_secs(seconds));
    stop.store(true, Ordering::Relaxed);
    let g = input.lock().unwrap();
    Some((
        g.clicks.len(),
        g.keys.len(),
        g.cursor.len(),
        g.scrolls.len(),
    ))
}
#[cfg(not(all(target_os = "linux", feature = "capture-linux")))]
pub fn evdev_probe(_seconds: u64, _capture_keys: bool) -> Option<(usize, usize, usize, usize)> {
    None
}

/// What to capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureConfig {
    /// Stop after this many ms (None = until `stop`/Ctrl-C; live backends only).
    pub duration_ms: Option<u64>,
    pub fps: f64,
    /// Capture the OS cursor into the video? Default false — we HIDE it and
    /// re-render a synthetic cursor during capture polish.
    pub capture_cursor: bool,
    /// Which monitor (None = primary).
    pub monitor: Option<u32>,
    /// Capture just ONE application window by title (None = whole monitor/screen).
    /// Takes precedence over `monitor` when set. Windows-only; matched with
    /// `Window::from_contains_name` at capture start.
    pub window: Option<String>,
    pub audio: bool,
    /// Capture DESKTOP/SYSTEM audio (game/app sound) in the SAME capture, as a SEPARATE
    /// mixable track. Only the macOS (ScreenCaptureKit) backend reads this — it sets the
    /// stream's `capturesAudio`; the screen_record orchestrator then splits it out to
    /// `system.wav` + strips `source.mp4` to video-only (Linux/Windows capture system audio
    /// via their own parallel loopback path and ignore this flag).
    pub system_audio: bool,
    /// Record KEYSTROKES for the key-cast overlay. OFF by default — keys can reveal
    /// passwords / secrets / private input. Opt-in, and ideally surfaced in the UI.
    pub capture_keys: bool,
    /// Directory to write captured artifacts into.
    pub out_dir: String,
    /// Durable, independently playable media checkpoint publication. The server
    /// creates the manifest before capture; live backends rotate/finalize segments
    /// at this interval and never publish an open MP4.
    #[serde(default)]
    pub checkpoint: Option<CheckpointConfig>,
    /// Server-owned origin shared with sidecar workers. Never serialized; replay
    /// and screenshot captures leave it empty.
    #[serde(skip, default)]
    pub clock: Option<CaptureClock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointConfig {
    pub manifest_dir: String,
    pub interval_ms: u64,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            duration_ms: None,
            fps: 30.0,
            capture_cursor: false,
            monitor: None,
            window: None,
            audio: false,
            system_audio: false,
            capture_keys: false,
            out_dir: ".".to_string(),
            checkpoint: None,
            clock: None,
        }
    }
}

/// The artifacts a capture produced.
#[derive(Debug, Clone)]
pub struct CaptureOutput {
    pub source_video: String,
    pub events: EventTrack,
    pub webcam_video: Option<String>,
    pub audio: Option<String>,
    pub settings: Settings,
}

impl CaptureOutput {
    /// Fold the captured artifacts into a `RecordingProject` (ready for autoedit).
    pub fn into_project(self) -> RecordingProject {
        let mut p = RecordingProject::new(self.source_video, self.settings, self.events);
        p.webcam_video = self.webcam_video;
        p.audio = self.audio;
        p
    }
}

/// A capture backend: produce a source video + event track per `cfg`.
///
/// `stop` is an EXTERNAL stop flag owned by the caller: set it from another
/// thread to end an in-progress capture early. This is what makes OPEN-ENDED
/// ("record until I stop") recording possible — with `cfg.duration_ms == None` the
/// backend runs until `stop` is set, instead of a fixed wall-clock window. When a
/// `duration_ms` IS given it still serves as an upper bound; whichever fires first
/// (the deadline or the external stop) ends the capture. Backends that can't yet
/// poll `stop` inside their native loop keep their bounded behavior (documented
/// per-backend) but must still accept the parameter.
pub trait Capture {
    fn capture(
        &self,
        cfg: &CaptureConfig,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<CaptureOutput>;
}

#[cfg(test)]
mod monitor_tests {
    //! Shape + cross-platform contract for the monitor PICKER list.

    use super::*;

    /// `MonitorInfo` serializes to the exact `{index,name,width,height,primary}`
    /// shape the cutd `screen_record.doctor` result and the UI `<select>` consume.
    #[test]
    fn monitor_info_serializes_to_the_expected_shape() {
        let m = MonitorInfo {
            index: 1,
            name: "Monitor 1".into(),
            width: 3840,
            height: 2160,
            primary: true,
        };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["index"], 1);
        assert_eq!(v["name"], "Monitor 1");
        assert_eq!(v["width"], 3840);
        assert_eq!(v["height"], 2160);
        assert_eq!(v["primary"], true);
        // Round-trips back to an equal value (Deserialize contract).
        let back: MonitorInfo = serde_json::from_value(v).unwrap();
        assert_eq!(back, m);
    }

    /// On the Linux/headless build this crate is tested on, `list_monitors()` is the
    /// empty in-app list (the portal/full-screen path). Asserting empty here documents+locks
    /// that contract. Windows (WGC) AND macOS (ScreenCaptureKit) do REAL enumeration,
    /// so this empty-list assertion is scoped to builds WITHOUT
    /// either native picker backend (else it would wrongly fail on native-picker builds).
    #[cfg(not(any(
        all(windows, feature = "capture-windows"),
        all(target_os = "macos", feature = "capture-macos")
    )))]
    #[test]
    fn list_monitors_is_empty_without_a_native_picker_backend() {
        assert!(
            list_monitors().is_empty(),
            "without a native picker backend the in-app monitor list is empty (OS portal / full-screen path)"
        );
    }
}

#[cfg(test)]
mod stop_tests {
    //! Backend-agnostic proof that the EXTERNAL stop flag ends a capture early.
    //!
    //! The real LinuxCapture/WindowsCapture poll loops (`while !stop.load() && elapsed
    //! < dur { sleep }`) need a live desktop ScreenCast portal / WGC, which a headless
    //! CI or a headless WSL environment without the portal can't provide. This mock `Capture`
    //! reproduces the EXACT poll-loop contract those backends use — `None` ⇒
    //! unbounded cap (`u64::MAX/4`), poll the passed-in `stop` — so the open-ended +
    //! early-stop behavior is proven deterministically off any platform.

    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// A backend whose "capture loop" is the same poll loop linux.rs/windows.rs run:
    /// run until the deadline (`duration_ms`, or `u64::MAX/4` when None) OR `stop`.
    /// Records how long it actually ran into `events.duration_ms`.
    struct PollMock;
    impl Capture for PollMock {
        fn capture(&self, cfg: &CaptureConfig, stop: Arc<AtomicBool>) -> Result<CaptureOutput> {
            // Mirror the backends: None ⇒ effectively-never deadline (open-ended).
            let dur = cfg.duration_ms.unwrap_or(u64::MAX / 4);
            let start = Instant::now();
            while !stop.load(Ordering::Relaxed) && start.elapsed() < Duration::from_millis(dur) {
                std::thread::sleep(Duration::from_millis(5));
            }
            let ran_ms = start.elapsed().as_millis() as u64;
            let events = EventTrack {
                duration_ms: ran_ms,
                screen_w: 1920,
                screen_h: 1080,
                monitors: vec![],
                cursor: vec![],
                cursor_correlation: record_core::CursorCorrelation::default(),
                clicks: vec![],
                scrolls: vec![],
                keys: vec![],
            };
            Ok(CaptureOutput {
                source_video: "mock.mp4".into(),
                events,
                webcam_video: None,
                audio: None,
                settings: Settings {
                    width: 1920,
                    height: 1080,
                    fps: 30.0,
                    audio_rate: 48_000,
                },
            })
        }
    }

    /// OPEN-ENDED (`duration_ms: None`): the capture must run UNTIL `stop` is set,
    /// NOT to any fixed default. We set stop after ~150ms and assert the capture ran
    /// for ~that long and ended promptly — proving None is unbounded and stop-driven.
    #[test]
    fn open_ended_capture_runs_until_stop_not_a_default() {
        let cfg = CaptureConfig {
            duration_ms: None, // open-ended
            ..Default::default()
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stop_c = stop.clone();
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            stop_c.store(true, Ordering::Relaxed);
        });
        let t0 = Instant::now();
        let out = PollMock.capture(&cfg, stop).unwrap();
        let wall = t0.elapsed();
        stopper.join().unwrap();
        // Ran roughly until stop (~150ms), nowhere near the legacy 6 s / 15 s default,
        // and ended within a small margin of the stop signal (prompt response).
        assert!(
            out.events.duration_ms >= 120 && out.events.duration_ms < 1_000,
            "open-ended capture ran {}ms — expected ~150ms (until stop), not a fixed default",
            out.events.duration_ms
        );
        assert!(
            wall < Duration::from_millis(800),
            "capture should end promptly after stop; took {wall:?}"
        );
    }

    /// A `duration_ms` cap is an UPPER bound: an EARLY stop ends the capture well
    /// before the cap (whichever fires first wins).
    #[test]
    fn early_stop_beats_the_duration_cap() {
        let cfg = CaptureConfig {
            duration_ms: Some(10_000), // 10 s cap
            ..Default::default()
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stop_c = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(120));
            stop_c.store(true, Ordering::Relaxed);
        });
        let out = PollMock.capture(&cfg, stop).unwrap();
        assert!(
            out.events.duration_ms < 2_000,
            "early stop should end the capture ~120ms in, far below the 10s cap; ran {}ms",
            out.events.duration_ms
        );
    }
}

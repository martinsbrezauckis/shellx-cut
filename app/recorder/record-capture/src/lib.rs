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
//!   Windows Media Foundation and macOS AVFoundation camera adapters remain
//!   behind crate-private construction. Their sole public admission is the
//!   screen-recording owner's explicit selected-camera Start path.
//!
//! `doctor()` reports capability cards (mirrors ShellX Cut's system.doctor) so the
//! UI/agent can tell what's present vs needs install/permission.

pub mod camera;
// REC-CAMERA-01a is deliberately crate-private until a durable server owner
// wires it to real native finalization. Its test-injected lifecycle must still
// compile in ordinary builds so that boundary remains explicit and reviewable.
#[allow(
    dead_code,
    reason = "private staged camera finalization awaits one reviewed native adapter"
)]
mod camera_finalization;
mod camera_finalization_anchored;
mod camera_finalization_durability;
mod camera_finalization_error;
mod camera_finalization_identity;
mod camera_finalization_owner;
mod camera_finalization_paths;
mod camera_finalization_publication;
#[allow(
    dead_code,
    reason = "private camera-session spine awaits server wiring"
)]
mod camera_session;
mod camera_session_evidence;
mod camera_session_start;
mod camera_timing;
// REC-CAMERA-01b provides the private adapter registry and explicit-use/Stop
// ownership seam. Platform adapters remain crate-private; the server reaches
// them only through the selected-camera screen-recording owner.
#[cfg(all(test, target_os = "linux"))]
mod camera_finalization_post_sync_tests;
#[cfg(all(test, target_os = "linux"))]
mod camera_finalization_race_tests;
#[cfg(all(test, target_os = "linux"))]
mod camera_finalization_test_support;
#[cfg(all(test, target_os = "linux"))]
mod camera_finalization_tests;
#[allow(
    dead_code,
    reason = "private camera runtime stays crate-private behind the platform camera owner"
)]
mod camera_runtime;
#[cfg(test)]
mod camera_runtime_tests;
#[cfg(test)]
mod camera_session_lifecycle_tests;
#[cfg(test)]
mod camera_session_tamper_tests;
#[cfg(test)]
mod camera_session_test_support;
#[cfg(test)]
mod camera_session_tests;
#[cfg(test)]
mod camera_session_timing_tests;
mod capture_clock;
mod capture_output;
mod checkpoint;
mod controller_placement;
pub mod doctor;
mod doctor_portal;
mod doctor_probe;
mod doctor_process;
mod doctor_system_audio;
mod source_lifecycle;
// REC-REGION-01 first fixes the cross-host crop contract; the native backends
// consume it in the next slice. Keep that bounded foundation compiled on native
// feature builds without weakening dead-code diagnostics elsewhere.
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod macos_capture_target;
#[cfg(any(test, all(target_os = "macos", feature = "capture-macos")))]
mod macos_region_plan;
mod pause_stream_coordinator;
#[cfg(test)]
mod pause_stream_coordinator_tests;
mod pause_stream_selection;
mod pause_stream_types;
// REC-PAUSE-01 macOS pause ownership is composed with the server's private
// journal/projection path. Public admission, UI, and installed capability
// remain closed until native qualification and passive-input parity exist.
#[cfg(any(test, all(target_os = "macos", feature = "capture-macos")))]
#[allow(
    dead_code,
    reason = "macOS pause integration remains behind private server admission"
)]
mod macos_pause_pilot;
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
#[doc(hidden)]
pub mod private_macos_pause_owner;
mod recording_scenes;
mod recording_scenes_config;
mod recording_scenes_engine;
mod recording_scenes_projection;
#[cfg(test)]
mod recording_scenes_tests;
#[allow(dead_code)]
#[cfg(any(
    test,
    all(windows, feature = "capture-windows"),
    all(target_os = "macos", feature = "capture-macos"),
    all(target_os = "linux", feature = "capture-linux")
))]
mod region_geometry;
#[cfg(test)]
mod region_geometry_tests;
mod replay;
mod session_clock;
#[cfg(test)]
mod session_clock_tests;
// REC-SOURCE-PREVIEW-01 keeps preview ownership separate from recording output.
// Native adapters receive exact opaque selections and explicit lifecycle commands;
// this core never accepts paths, ordinals, titles, or artifact directories.
pub mod source_preview;
mod source_preview_bitmap;
#[cfg(test)]
mod source_preview_bitmap_tests;
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
mod source_preview_linux;
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
mod source_preview_linux_pipewire;
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod source_preview_macos;
pub mod source_preview_native;
#[cfg(test)]
mod source_preview_native_tests;
#[cfg(test)]
mod source_preview_tests;
#[cfg(all(windows, feature = "capture-windows"))]
mod source_preview_windows;
#[cfg(all(windows, feature = "capture-windows"))]
mod windows_controller_exclusion;
// REC-SCENES-01 owns only a private, capture-contained scene receipt journal.
// It intentionally has no server verb, UI registration, device access, or live
// switch behavior until an explicit capture coordinator consumes it.
#[allow(
    dead_code,
    reason = "private scene journal awaits capture-coordinator wiring"
)]
mod scene_journal;
#[cfg(test)]
mod scene_journal_fault_tests;
mod scene_journal_io;
mod scene_journal_parse;
mod scene_journal_test_hooks;
#[cfg(test)]
mod scene_journal_tests;
#[cfg(windows)]
mod scene_journal_windows;
// REC-SCENES-02 connects the receipt to a private live owner, but only for
// Screen-only snapshots. Camera layouts and all UI/verb surfaces remain absent.
mod scene_live_coordinator;
#[cfg(test)]
mod scene_live_coordinator_tests;
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
#[cfg(any(
    all(windows, feature = "capture-windows"),
    all(target_os = "macos", feature = "capture-macos"),
    all(target_os = "linux", feature = "capture-linux")
))]
mod input_listener;

#[cfg(feature = "mic")]
mod macos_system_audio;
#[cfg(feature = "mic")]
mod mic;
mod mic_endpoint;
#[cfg(feature = "mic")]
mod mic_timing;
#[cfg(feature = "mic")]
mod microphone_result;
// REC-VOICEOVER-01 is a reusable microphone-only capture/session owner. It is
// deliberately absent from server verbs and UI until a project-owned atomic
// materialization/import/placement owner can make its artifact one Undoable
// revision instead of exposing a misleading record affordance.
#[cfg(any(
    test,
    all(windows, feature = "capture-windows"),
    all(target_os = "macos", feature = "capture-macos")
))]
mod monitor_identity;
mod system_audio_probe;
#[cfg(feature = "mic")]
mod system_audio_timing;
#[cfg(feature = "mic")]
mod voiceover_capture;
mod window_target;

// REC-PAUSE-01 Windows-only native seam. It owns neutral command/event values
// and WGC run ordering, but no Cut server journal, verb, UI, or installed claim.
// The server consumes it through a separate private adapter.
#[doc(hidden)]
pub mod windows_pause_pilot;

// The private WGC run owner is platform-neutral enough to exercise its control
// ordering on the host test target; only the live adapter below is Windows-only.
#[cfg(any(test, all(windows, feature = "capture-windows")))]
mod windows_wgc_run;
#[cfg(test)]
mod windows_wgc_run_tests;
mod windows_wgc_run_types;

#[cfg(all(windows, feature = "capture-windows"))]
mod windows;
#[cfg(all(windows, feature = "capture-windows"))]
mod windows_gpu_crop;
#[cfg(all(windows, feature = "capture-windows"))]
mod windows_monitor_target;
#[cfg(all(windows, feature = "capture-windows"))]
mod windows_picker;
#[cfg(all(windows, feature = "capture-windows"))]
mod windows_region_capture;
#[cfg(all(windows, feature = "capture-windows"))]
mod windows_runtime;
#[cfg(all(windows, feature = "capture-windows"))]
mod windows_wgc_handler;

// REC-CAMERA-01c is a private Media Foundation Capture Engine adapter. It has
// no server verb, UI registration, Doctor card, or public device identity.
// The constructor requires the capture owner's already-reserved directory, so
// a future server owner must make that ownership hand-off explicit.
#[cfg(all(windows, feature = "capture-windows"))]
mod windows_camera;
// The only cross-crate shape for the reviewed Windows Camera Capture Engine.
// It is doc-hidden and server-only: no generic runtime, enumeration, device
// label, verb, Doctor card, or UI registration is exported from this crate.
#[cfg(all(windows, feature = "capture-windows"))]
#[doc(hidden)]
pub mod private_windows_camera_owner;

// Public Recorder wiring consumes only this explicit-use, opaque-device owner.
// The module name is hidden from generated docs; native identities stay inside
// the platform adapters and camera permission is never requested by Doctor.
#[cfg(any(
    all(windows, feature = "capture-windows"),
    all(target_os = "macos", feature = "capture-macos")
))]
#[doc(hidden)]
pub mod private_camera_owner;

#[cfg(all(windows, feature = "capture-windows"))]
mod windows_probe;

#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod macos;
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod macos_camera;
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod macos_camera_finalization;
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod macos_camera_native;
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod macos_checkpoint;
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod macos_finalization;
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod macos_monitor_target;
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod macos_readiness;
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod macos_region_capture;
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod macos_system_tap;

#[cfg(all(target_os = "macos", feature = "capture-macos"))]
mod macos_probe;

#[cfg(all(target_os = "linux", feature = "capture-linux"))]
mod linux;
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
mod linux_capture_state;
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
mod linux_gst_capture;
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
mod linux_input;
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
mod linux_portal;
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
mod linux_runtime;
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

pub(crate) use camera::CameraUseIntent;
pub use camera::{CameraBackend, CameraDevice, CameraReadiness, CameraRequest, ReplayCamera};
pub(crate) use camera_timing::CameraFrameObservation;
pub use capture_clock::CaptureClock;
pub use doctor::{doctor, Card};
pub use doctor_portal::{is_linux_portal_prompt_deferred, LINUX_PORTAL_PROMPT_DEFERRED_DETAIL};
pub use mic_endpoint::{
    list_microphone_endpoints, resolve_microphone_source, MicrophoneEndpoint,
    MicrophoneEndpointRef, MicrophoneSource,
};
pub use pause_stream_coordinator::PauseStreamCoordinator;
pub use pause_stream_selection::SelectedCaptureStreams;
pub use pause_stream_types::{
    AcknowledgementRejection, AcknowledgementResult, BoundaryRequestResult,
    PauseStreamCoordinatorStatus, PendingStreamBoundary, StreamAcknowledgement, StreamBoundary,
    StreamBoundaryKind, StreamRefusal,
};
pub use record_recovery::RecordingStream;
pub use recording_scenes::{
    RecordingSceneConfig, RecordingSceneEngine, RecordingSceneEngineError, RecordingSceneLayout,
    RecordingScenePipCorner, RecordingScenePipShape, RecordingScenePresetConfig,
    RecordingSceneProjection, RecordingSceneTimerAction, RecordingSceneTimerConfig,
    RECORDING_SCENE_PROJECTION_SCHEMA,
};
pub use replay::ReplayCapture;
#[doc(hidden)]
pub use scene_live_coordinator::{
    PrivateSceneCoordinator, PrivateSceneCoordinatorError, PrivateSceneProjection,
};
pub use session_clock::{
    LogicalSessionClock, SessionPhase, SessionTransition, SessionTransitionIgnored,
    SessionTransitionResult,
};
pub use system_audio_probe::{
    probe_system_audio, reserve_system_audio, SystemAudioLease, SystemAudioProbe, DEFAULT_WINDOW_MS,
};
#[cfg(feature = "mic")]
pub use voiceover_capture::{
    VoiceoverArtifact, VoiceoverCaptureOutcome, VoiceoverCaptureSession, VoiceoverFinalize,
};

/// Convert a CoreGraphics display number obtained inside the private macOS
/// visual-picker bridge into the same opaque identity used by the capture
/// backend. This is deliberately not a public picker API: it exists solely so
/// the in-process server can immediately replace the native value before it
/// enters its one-use selection registry.
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
#[doc(hidden)]
pub fn private_macos_monitor_identity_from_display_id(display_id: u32) -> Option<String> {
    macos_monitor_target::monitor_id_from_native_display_id(display_id)
}

/// Re-enumerate one exact macOS display and prove that a private picker crop
/// still names its original physical-pixel parent before cutd consumes the
/// one-use ticket. This is intentionally not a general region API: callers
/// receive only a boolean and must keep the crop in a typed private boundary.
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
#[doc(hidden)]
pub fn private_macos_region_selection_is_current(identity: &str, crop: CaptureRegion) -> bool {
    macos_region_capture::selection_is_current(identity, crop)
}

/// Convert the physical target path captured in the private Windows overlay's
/// atomic DisplayConfig snapshot into the opaque identity used by WGC. A later
/// ticket owner must first revalidate the snapshot's topology fingerprint; it
/// must never resolve a mutable GDI source name after crop selection.
#[cfg(all(windows, feature = "capture-windows"))]
#[doc(hidden)]
pub fn private_windows_monitor_identity_from_target_path(target_path: &str) -> Option<String> {
    windows_monitor_target::monitor_id_from_target_path(target_path)
}

/// Re-enumerate the exact active Windows DisplayConfig topology captured by
/// the foreground picker, then verify that its physical target still hashes to
/// the child ticket's opaque WGC monitor identity. This is private bridge
/// plumbing only: a successful check does not claim that the pinned encoder
/// can crop frames, and callers must still refuse before reservation until an
/// exact GPU-side WGC crop consumer exists.
#[cfg(all(windows, feature = "capture-windows"))]
#[doc(hidden)]
pub fn private_windows_region_selection_is_current(
    source_gdi: &str,
    target_path: &str,
    topology_digest: [u8; 32],
    monitor_id: &str,
    crop: CaptureRegion,
) -> bool {
    windows_region_capture::selection_is_current(
        source_gdi,
        target_path,
        topology_digest,
        monitor_id,
        crop,
    )
}

/// Construct the private Windows Camera runtime for one already-reserved
/// capture directory. This is intentionally not a general camera API: it is
/// unavailable to server verbs and UI until installed Windows proof admits a
/// separate owner. Native device identities stay inside `windows_camera`.
#[cfg(all(windows, feature = "capture-windows"))]
#[doc(hidden)]
#[allow(
    dead_code,
    reason = "private Windows camera construction awaits its reviewed server owner"
)]
pub(crate) fn private_windows_camera_runtime(
    capture_directory: &std::path::Path,
) -> record_core::Result<camera_runtime::CameraRuntime> {
    windows_camera::private_runtime(capture_directory)
}

/// One physical display the user can pick as the capture target.
///
/// Returned by [`list_monitors`] so the UI / agent can offer a monitor PICKER on a
/// multi-display setup. The `id` is an opaque native identity. The existing
/// `index` is the 1-based value the current capture backend wants in
/// [`CaptureConfig::monitor`]
/// (`WcMonitor::from_index` on Windows). When `id` is present, the UI also
/// passes it unchanged as `screen_record.start{monitor_id}` so the native
/// backend can re-resolve the exact target without ordinal fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorInfo {
    /// Opaque, versioned identity for this exact native display when the
    /// platform can derive one. When selected, it is re-resolved exactly at
    /// capture start; an unavailable identity fails closed rather than falling
    /// back to display order, title, primary state, or geometry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
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
/// just one app, not the whole screen). `id` is an opaque native identity returned
/// by the current enumeration and consumed unchanged by [`CaptureConfig::window`].
/// The title is display copy only and never selects a capture target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInfo {
    /// Opaque native window identity, valid only while that exact target remains live.
    pub id: String,
    /// Current display title. It may change without changing target identity.
    pub title: String,
    /// Owning process / app name, best-effort (for display, e.g. "chrome.exe").
    pub app: String,
}

/// Enumerate the displays available as capture targets, for the in-app monitor
/// PICKER. The list mirrors the backend's 1-based indexing so a chosen
/// `MonitorInfo.index` is passed straight back as `CaptureConfig.monitor` when
/// no exact id is available. `MonitorInfo.id` is otherwise passed unchanged as
/// `CaptureConfig.monitor_id` and intentionally has no ordinal fallback.
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
///   (`Window::enumerate()`), filtered to valid, titled, non-trivial windows. The
///   opaque HWND + process identity is revalidated exactly at capture start.
/// - **macOS** (`capture-macos`): real ScreenCaptureKit enumeration with an opaque
///   native window id that is revalidated exactly at capture start.
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

/// The result of a bounded microphone test.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicWarm {
    /// True iff the mic produced at least one audio callback within the window.
    pub live: bool,
    /// Highest real sample peak heard during the bounded warm/test window, in
    /// dBFS. `None` means no measurable signal was delivered; it is never a
    /// synthetic floor or a continuous recording meter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_dbfs: Option<i16>,
    /// True when this build was compiled with the `mic` feature (a mic backend).
    pub supported: bool,
}

/// Warm the default microphone for up to `max_ms` (see [`MicWarm`]).
pub fn warm_mic(max_ms: u64) -> MicWarm {
    warm_microphone(&MicrophoneSource::SystemDefault, max_ms)
}

/// Warm/test a source after the server has resolved its private selection.
pub fn warm_microphone(source: &MicrophoneSource, max_ms: u64) -> MicWarm {
    mic_endpoint::warm_microphone(source, max_ms)
}

pub use controller_placement::{
    CaptureControllerPlacement, CaptureControllerPlacementState, CaptureControllerPlacementStatus,
};
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
pub use linux_system_audio::{capture_system_pipewire, capture_system_pipewire_with_level};
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
pub use mic::{capture_system_loopback, capture_system_loopback_with_level};
#[cfg(feature = "mic")]
pub use mic::{AudioLevelLifecycle, AudioLevelSnapshot, RollingAudioLevel};
pub use source_lifecycle::{
    CaptureSourceLifecycle, CaptureSourceLifecycleState, CaptureSourceLifecycleStatus,
};
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

use record_core::{
    error_codes, CameraArtifact, CaptureOutputSize, CaptureQualityProfile, CaptureQualityRequest,
    CaptureQualityResolution, EventTrack, RecordError, Result, Settings,
};
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
    let listener = input_evdev::spawn_evdev_listener(start, stop.clone(), capture_keys, 1920, 1080);
    eprintln!(">>> evdev probe: generate input NOW for {seconds}s <<<");
    std::thread::sleep(std::time::Duration::from_secs(seconds));
    stop.store(true, Ordering::Relaxed);
    match listener.seal(start.elapsed().as_millis() as u64) {
        Ok((cursor, clicks, scrolls, keys)) => {
            Some((clicks.len(), keys.len(), cursor.len(), scrolls.len()))
        }
        Err(error) => {
            eprintln!("evdev probe failed to seal input: {error}");
            None
        }
    }
}
#[cfg(not(all(target_os = "linux", feature = "capture-linux")))]
pub fn evdev_probe(_seconds: u64, _capture_keys: bool) -> Option<(usize, usize, usize, usize)> {
    None
}

/// The small output-quality surface is advertised only where the final source
/// normalizer owns its scale and encoder configuration. Other native backends
/// must refuse a direct request rather than letting a client imply support.
#[derive(Debug, Clone, Serialize)]
pub struct CaptureQualityCapability {
    pub supported: bool,
    pub output_sizes: Vec<CaptureOutputSize>,
    pub profiles: Vec<CaptureQualityProfile>,
    pub detail: String,
}

pub fn capture_quality_capability() -> CaptureQualityCapability {
    if cfg!(all(target_os = "linux", feature = "capture-linux")) {
        CaptureQualityCapability {
            supported: true,
            output_sizes: vec![
                CaptureOutputSize::Source,
                CaptureOutputSize::P1080,
                CaptureOutputSize::P720,
            ],
            profiles: vec![CaptureQualityProfile::Standard, CaptureQualityProfile::High],
            detail: "Linux final-source normalization resolves output dimensions, cadence, and the libx264 encoder after final verification.".into(),
        }
    } else {
        CaptureQualityCapability {
            supported: false,
            output_sizes: Vec::new(),
            profiles: Vec::new(),
            detail: "This recorder backend does not yet have a verified output-size and quality-profile path.".into(),
        }
    }
}

/// Admit only the request a live backend can actually fulfill. This remains a
/// server-side guard as clients can be older, handcrafted, or stale.
pub fn admit_capture_quality(
    quality: Option<CaptureQualityRequest>,
) -> Result<Option<CaptureQualityRequest>> {
    if quality.is_none() || capture_quality_capability().supported {
        return Ok(quality);
    }
    Err(RecordError::new(
        error_codes::UNIMPLEMENTED,
        "output quality is unavailable on this recorder backend",
        "the active backend has no verified output-size and encoder-profile admission path",
    )
    .with_action(
        "record at Source quality, or use a Linux capture backend that reports output quality support",
    ))
}

#[cfg(test)]
mod capture_quality_tests {
    use super::*;

    #[test]
    fn direct_quality_requests_are_admitted_only_by_the_advertised_backend() {
        let request =
            CaptureQualityRequest::new(CaptureOutputSize::P720, CaptureQualityProfile::High);
        let capability = capture_quality_capability();
        let admitted = admit_capture_quality(Some(request.clone()));
        if capability.supported {
            assert_eq!(admitted.unwrap(), Some(request));
        } else {
            let error = admitted.unwrap_err();
            assert_eq!(error.code, error_codes::UNIMPLEMENTED);
            assert!(error.message.contains("unavailable"));
        }
    }
}

/// What to capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureConfig {
    /// Stop after this many ms (None = until `stop`/Ctrl-C; live backends only).
    pub duration_ms: Option<u64>,
    pub fps: f64,
    /// A server-admitted simple output request. Native backends that do not
    /// own a verified resize/profile path never receive a non-None value.
    #[serde(skip, default)]
    pub quality: Option<CaptureQualityRequest>,
    /// Capture the OS cursor into the video? Default false — we HIDE it and
    /// re-render a synthetic cursor during capture polish.
    pub capture_cursor: bool,
    /// Which monitor (None = primary).
    pub monitor: Option<u32>,
    /// Opaque exact native identity from a fresh [`list_monitors`] row. When
    /// present, native backends re-enumerate and select only this identity; the
    /// legacy [`Self::monitor`] ordinal is not a fallback. Never serialized into
    /// recording artifacts or receipts.
    #[serde(skip, default)]
    pub monitor_id: Option<String>,
    /// A native-picker-admitted crop for the exact monitor in [`Self::monitor_id`].
    /// It is private transport only and is never serialized into an argument,
    /// receipt, or public recorder response.
    #[serde(skip, default)]
    pub region: Option<CaptureRegion>,
    /// Capture just ONE application window by opaque id from [`list_windows`]
    /// (None = whole monitor/screen). Takes precedence over `monitor` when set.
    /// The backend revalidates that exact native identity immediately before use.
    pub window: Option<String>,
    pub audio: bool,
    /// The microphone endpoint is resolved once before a capture starts and is
    /// intentionally not serialized into manifests, logs, or public receipts.
    /// Existing callers that omit it preserve the OS-system-default behavior.
    #[serde(skip, default)]
    pub microphone_source: MicrophoneSource,
    /// Caller-owned continuous level state fed by the same admitted microphone
    /// stream. `None` preserves capture without a public meter; no second input
    /// stream is opened for monitoring.
    #[cfg(feature = "mic")]
    #[serde(skip, default)]
    pub microphone_level: Option<std::sync::Arc<RollingAudioLevel>>,
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
    /// Per-capture readiness proof owned by the live server reservation. Native
    /// backends may set it only after receiving an actual screen frame; it is
    /// deliberately absent from replay, screenshot, and serialized configs.
    #[serde(skip, default)]
    pub readiness: Option<CaptureReadiness>,
    /// Capture-owned observed placement for recorder controls. Native backends
    /// may update it only after a concrete platform exclusion/hide decision;
    /// it is never serialized into media, requests, or artifacts.
    #[serde(skip, default)]
    pub controller_placement: Option<CaptureControllerPlacement>,
    /// Capture-owned selected-source closure evidence. Native adapters may arm
    /// it only for a concrete source-close callback; it is never serialized.
    #[serde(skip, default)]
    pub source_lifecycle: Option<CaptureSourceLifecycle>,
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
            quality: None,
            capture_cursor: false,
            monitor: None,
            monitor_id: None,
            region: None,
            window: None,
            audio: false,
            microphone_source: MicrophoneSource::SystemDefault,
            #[cfg(feature = "mic")]
            microphone_level: None,
            system_audio: false,
            capture_keys: false,
            out_dir: ".".to_string(),
            checkpoint: None,
            clock: None,
            readiness: None,
            controller_placement: None,
            source_lifecycle: None,
        }
    }
}

/// Monotonic proof that an active live capture has received a real screen
/// frame. This is intentionally separate from the capture clock: opening a
/// native backend, reserving an output, or starting a process is not media
/// delivery.
#[derive(Debug, Clone)]
pub struct CaptureReadiness(std::sync::Arc<std::sync::atomic::AtomicU8>);

/// The public, read-only projection of [`CaptureReadiness`]. `ready` is an
/// admission fact, so it is false after terminalization even if a real frame
/// arrived earlier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureReadinessStatus {
    pub ready: bool,
    pub terminal: bool,
    pub state: CaptureReadinessState,
}

/// A capture never returns from a terminal readiness state to `Ready`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureReadinessState {
    AwaitingFirstScreenFrame,
    Ready,
    TerminalBeforeFirstScreenFrame,
    TerminalAfterFirstScreenFrame,
}

impl CaptureReadinessState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AwaitingFirstScreenFrame => "awaiting_first_screen_frame",
            Self::Ready => "ready",
            Self::TerminalBeforeFirstScreenFrame => "terminal_before_first_screen_frame",
            Self::TerminalAfterFirstScreenFrame => "terminal_after_first_screen_frame",
        }
    }
}

impl Default for CaptureReadiness {
    fn default() -> Self {
        Self(std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0)))
    }
}

impl CaptureReadiness {
    const AWAITING: u8 = 0;
    const READY: u8 = 1;
    const TERMINAL_BEFORE_FRAME: u8 = 2;
    const TERMINAL_AFTER_FRAME: u8 = 3;

    /// Mark a real native screen frame as delivered to the active capture
    /// path. A late callback cannot revive an already-terminal capture.
    pub fn mark_first_screen_frame_delivered(&self) {
        let _ = self.0.compare_exchange(
            Self::AWAITING,
            Self::READY,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        );
    }

    /// Close readiness at the same boundary as the live capture Stop signal.
    /// Preserve whether a frame was observed for diagnostics, but never leave
    /// a terminal capture admitted as ready.
    pub fn mark_terminal(&self) {
        loop {
            let current = self.0.load(std::sync::atomic::Ordering::Acquire);
            let next = match current {
                Self::AWAITING => Self::TERMINAL_BEFORE_FRAME,
                Self::READY => Self::TERMINAL_AFTER_FRAME,
                Self::TERMINAL_BEFORE_FRAME | Self::TERMINAL_AFTER_FRAME => return,
                _ => return,
            };
            if self
                .0
                .compare_exchange(
                    current,
                    next,
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire,
                )
                .is_ok()
            {
                return;
            }
        }
    }

    pub fn status(&self) -> CaptureReadinessStatus {
        let state = match self.0.load(std::sync::atomic::Ordering::Acquire) {
            Self::READY => CaptureReadinessState::Ready,
            Self::TERMINAL_BEFORE_FRAME => CaptureReadinessState::TerminalBeforeFirstScreenFrame,
            Self::TERMINAL_AFTER_FRAME => CaptureReadinessState::TerminalAfterFirstScreenFrame,
            _ => CaptureReadinessState::AwaitingFirstScreenFrame,
        };
        CaptureReadinessStatus {
            ready: matches!(state, CaptureReadinessState::Ready),
            terminal: matches!(
                state,
                CaptureReadinessState::TerminalBeforeFirstScreenFrame
                    | CaptureReadinessState::TerminalAfterFirstScreenFrame
            ),
            state,
        }
    }
}

#[cfg(test)]
mod capture_readiness_tests {
    use super::{CaptureReadiness, CaptureReadinessState};

    #[test]
    fn real_frame_is_the_only_transition_that_admits_capture_progress() {
        let readiness = CaptureReadiness::default();
        assert_eq!(
            readiness.status().state,
            CaptureReadinessState::AwaitingFirstScreenFrame
        );
        assert!(!readiness.status().ready);

        readiness.mark_first_screen_frame_delivered();
        assert_eq!(readiness.status().state, CaptureReadinessState::Ready);
        assert!(readiness.status().ready);
        assert!(!readiness.status().terminal);
    }

    #[test]
    fn terminalization_wins_over_a_late_frame_and_revokes_ready_admission() {
        let before_frame = CaptureReadiness::default();
        before_frame.mark_terminal();
        before_frame.mark_first_screen_frame_delivered();
        assert_eq!(
            before_frame.status().state,
            CaptureReadinessState::TerminalBeforeFirstScreenFrame
        );
        assert!(!before_frame.status().ready);

        let after_frame = CaptureReadiness::default();
        after_frame.mark_first_screen_frame_delivered();
        after_frame.mark_terminal();
        assert_eq!(
            after_frame.status().state,
            CaptureReadinessState::TerminalAfterFirstScreenFrame
        );
        assert!(!after_frame.status().ready);
        assert!(after_frame.status().terminal);
    }
}

/// Exact H.264-compatible native crop selected against one current monitor
/// frame. The retained parent dimensions make a backend refuse a stale display
/// mode or DPI scale rather than applying the rectangle to a changed frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureRegion {
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    parent_width: u32,
    parent_height: u32,
}

impl CaptureRegion {
    pub fn new(
        left: u32,
        top: u32,
        width: u32,
        height: u32,
        parent_width: u32,
        parent_height: u32,
    ) -> Option<Self> {
        let right = left.checked_add(width)?;
        let bottom = top.checked_add(height)?;
        (parent_width > 0
            && parent_height > 0
            && left.is_multiple_of(2)
            && top.is_multiple_of(2)
            && width >= 2
            && height >= 2
            && width.is_multiple_of(2)
            && height.is_multiple_of(2)
            && right <= parent_width
            && bottom <= parent_height)
            .then_some(Self {
                left,
                top,
                width,
                height,
                parent_width,
                parent_height,
            })
    }

    /// Native bridge only. Selected desktop coordinates must never appear in a
    /// public argument, response, or receipt.
    pub fn native_parts(self) -> (u32, u32, u32, u32, u32, u32) {
        (
            self.left,
            self.top,
            self.width,
            self.height,
            self.parent_width,
            self.parent_height,
        )
    }
}

/// The artifacts a capture produced.
#[derive(Debug, Clone)]
pub struct CaptureOutput {
    pub source_video: String,
    pub events: EventTrack,
    /// Authoritative CameraArtifact@1 for a synchronized camera stream. Native
    /// backends leave this absent until they implement the camera contract.
    pub camera_artifact: Option<CameraArtifact>,
    /// Legacy presentation-only path retained for existing projects/autoedit.
    pub webcam_video: Option<String>,
    pub audio: Option<String>,
    /// Sanitized microphone result. Endpoint identity and local paths remain private.
    pub microphone_outcome: MicrophoneCaptureOutcome,
    pub settings: Settings,
    /// Final quality evidence, present only when a backend both accepted the
    /// request and verified the emitted source against that request.
    pub capture_quality: Option<CaptureQualityResolution>,
    /// Facts from the mandatory final-source verification pass, when this
    /// backend has one. A direct/unverified backend leaves this absent instead
    /// of causing a second probe solely for presentation metadata.
    pub verified_media: Option<record_recovery::MediaFacts>,
}

/// What happened to the microphone track for a capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicrophoneCaptureOutcome {
    NotRequested,
    Saved,
    MicrophoneLostSavedPrefix,
    MicrophoneLostNoTrack,
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

    /// Whether this server-private backend has already atomically published the
    /// exact `RecordingProject` represented by its [`CaptureOutput`].
    ///
    /// Normal backends leave this `false` and the server writes `project.json`
    /// after capture returns. A pause-aware backend may need to publish its
    /// source, events, project, and private receipt as one validated projection
    /// before the ordinary recovery receipt is appended. Such a backend may set
    /// this only when the server can byte-verify the prepublished project against
    /// the returned output; it is not a public recording mode.
    fn prepublished_project(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod monitor_tests {
    //! Shape + cross-platform contract for the monitor PICKER list.

    use super::*;

    /// `MonitorInfo` serializes to the exact `{id?,index,name,width,height,primary}`
    /// shape the cutd `screen_record.doctor` result and the UI `<select>` consume.
    #[test]
    fn monitor_info_serializes_to_the_expected_shape() {
        let m = MonitorInfo {
            id: Some("shellx-monitor-v1:windows:0123456789abcdef".into()),
            index: 1,
            name: "Monitor 1".into(),
            width: 3840,
            height: 2160,
            primary: true,
        };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["id"], "shellx-monitor-v1:windows:0123456789abcdef");
        assert_eq!(v["index"], 1);
        assert_eq!(v["name"], "Monitor 1");
        assert_eq!(v["width"], 3840);
        assert_eq!(v["height"], 2160);
        assert_eq!(v["primary"], true);
        // Round-trips back to an equal value (Deserialize contract).
        let back: MonitorInfo = serde_json::from_value(v).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn unavailable_native_identity_does_not_hide_the_legacy_picker_row() {
        let m = MonitorInfo {
            id: None,
            index: 2,
            name: "Legacy display".into(),
            width: 1_280,
            height: 720,
            primary: false,
        };
        let value = serde_json::to_value(&m).unwrap();
        assert!(value.get("id").is_none());
        assert_eq!(value["index"], 2);
        assert_eq!(value["name"], "Legacy display");
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
                camera_artifact: None,
                webcam_video: None,
                audio: None,
                microphone_outcome: MicrophoneCaptureOutcome::NotRequested,
                settings: Settings {
                    width: 1920,
                    height: 1080,
                    fps: 30.0,
                    audio_rate: 48_000,
                },
                capture_quality: None,
                verified_media: None,
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

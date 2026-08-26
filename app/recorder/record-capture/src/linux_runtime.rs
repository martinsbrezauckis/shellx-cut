//! Process-lifetime runtime and executable resolution for Linux live capture.

use record_core::{error_codes, RecordError, Result};

pub(super) fn cap_err(ctx: &str, error: impl std::fmt::Display) -> RecordError {
    RecordError::new(error_codes::CAPTURE, ctx, error.to_string()).with_action(
        "ensure a desktop session is logged in with xdg-desktop-portal + PipeWire, \
         the user session bus is reachable (XDG_RUNTIME_DIR/DBUS_SESSION_BUS_ADDRESS), \
         and gst-launch-1.0 + the pipewire plugin are installed (gstreamer1.0-pipewire)",
    )
}

pub(super) fn ffmpeg_bin() -> String {
    std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string())
}

pub(super) fn ffprobe_bin() -> String {
    std::env::var("SHELLX_RECORD_FFPROBE").unwrap_or_else(|_| "ffprobe".to_string())
}

pub(super) fn gst_bin() -> String {
    std::env::var("SHELLX_RECORD_GST").unwrap_or_else(|_| "gst-launch-1.0".to_string())
}

/// Share one Tokio runtime across all portal work. ashpd caches its session-bus
/// connection globally, so dropping the runtime that spawned its reader can
/// wedge the next ScreenCast request in the same process.
pub(super) fn shared_runtime() -> Result<&'static tokio::runtime::Runtime> {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    if let Some(runtime) = RT.get() {
        return Ok(runtime);
    }
    // Build outside `get_or_init`: a loser in the initialization race is unused.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| cap_err("tokio runtime", error))?;
    Ok(RT.get_or_init(|| runtime))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SessionInputMode {
    pub wayland: bool,
    pub use_evdev: bool,
    pub use_pipewire_metadata: bool,
}

pub(crate) fn session_input_mode() -> SessionInputMode {
    session_input_mode_from(
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        std::env::var_os("DISPLAY").is_some(),
        std::env::var("SHELLX_RECORD_INPUT").ok().as_deref(),
        std::env::var("SHELLX_RECORD_WAYLAND_CAPTURE")
            .ok()
            .as_deref(),
    )
}

pub(crate) fn session_input_mode_from(
    session_type: Option<&str>,
    wayland_display: bool,
    _display: bool,
    input_override: Option<&str>,
    capture_override: Option<&str>,
) -> SessionInputMode {
    // A real GNOME Wayland desktop exports both WAYLAND_DISPLAY and DISPLAY for
    // Xwayland clients.  Controller-launched applications do not necessarily
    // inherit XDG_SESSION_TYPE, so treating `DISPLAY` as proof of an X11
    // *session* incorrectly selects the GStreamer fallback and loses the
    // PipeWire metadata path.  Prefer an explicit session type when present;
    // otherwise the reachable Wayland socket is the reliable fallback.
    let wayland = match session_type {
        Some(value) if value.eq_ignore_ascii_case("wayland") => true,
        Some(value) if value.eq_ignore_ascii_case("x11") => false,
        _ => wayland_display,
    };
    let use_evdev = match input_override {
        Some("evdev") => true,
        Some("rdevin") => false,
        _ => wayland,
    };
    let use_pipewire_metadata = match capture_override {
        Some("gst") => false,
        Some("pipewire") => true,
        _ => wayland,
    };
    SessionInputMode {
        wayland,
        use_evdev,
        use_pipewire_metadata,
    }
}

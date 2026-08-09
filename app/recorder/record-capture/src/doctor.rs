//! doctor.rs — capability detection (mirrors ShellX Cut's `system.doctor`).
//!
//! Reports `Card`s for the things live capture needs: ffmpeg, the screen-capture
//! backend, the input hook, and webcam — each with status ok/missing/degraded/unknown and
//! an actionable detail. Honest about what is COMPILED (feature/cfg gated) vs what
//! is merely present at runtime. The UI/agent uses this to drive install/permission.

use crate::{doctor_portal::LINUX_PORTAL_BACKEND_DETAIL, doctor_probe, doctor_process};
use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub id: String,
    pub kind: String,
    /// "ok" | "missing" | "degraded" | "unknown"
    pub status: String,
    pub detail: String,
}

impl Card {
    fn new(id: &str, kind: &str, status: &str, detail: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            status: status.into(),
            detail: detail.into(),
        }
    }
}

fn ffmpeg_bin() -> String {
    std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string())
}

fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .next()
        .unwrap_or("")
        .to_string()
}

/// Parse `loginctl show-session <id> -p LockedHint` output. Returns `Some(true)` for
/// `LockedHint=yes`, `Some(false)` for `LockedHint=no`, and `None` when the property is
/// absent or unparseable — so a caller NEVER false-degrades on a missing/odd answer.
/// Pure (no I/O) so it can be unit-tested without a live session.
fn parse_locked_hint(output: &str) -> Option<bool> {
    for line in output.lines() {
        if let Some(v) = line.trim().strip_prefix("LockedHint=") {
            return match v.trim() {
                "yes" => Some(true),
                "no" => Some(false),
                _ => None,
            };
        }
    }
    None
}

/// Parse `gdbus` screen-saver output. `Some(true)` = active, `Some(false)` = inactive, and
/// `None` keeps a gdbus error or unexpected text from falsely degrading capture.
fn parse_screensaver_active(output: &str) -> Option<bool> {
    let t = output.trim();
    if t.contains("true") {
        Some(true)
    } else if t.contains("false") {
        Some(false)
    } else {
        None
    }
}

/// Best-effort desktop LOCK detection. Returns `Some(true)` ONLY on a positive locked
/// signal, `Some(false)` on a positive unlocked signal, and `None` when nothing answers
/// (loginctl/gdbus absent or silent). Callers MUST treat `None` as "not locked" so a
/// missing tool never false-degrades a working desktop. GNOME inhibits ScreenCast
/// while locked, so the locked state must be reported explicitly.
fn session_locked() -> Option<bool> {
    // 1) systemd-logind LockedHint — authoritative, present on most modern desktops.
    if let Ok(sid) = std::env::var("XDG_SESSION_ID") {
        let mut command = Command::new("loginctl");
        command.args(["show-session", &sid, "-p", "LockedHint"]);
        if let Some(o) = doctor_process::output(&mut command, "probe login session lock") {
            if o.status.success() {
                if let Some(locked) = parse_locked_hint(&String::from_utf8_lossy(&o.stdout)) {
                    return Some(locked);
                }
            }
        }
    }
    // 2) GNOME ScreenSaver active — covers the GNOME lock screen even if LockedHint is
    //    unset. Positive signal only; a failure / weird reply falls through to None.
    let mut command = Command::new("gdbus");
    command.args([
        "call",
        "-e",
        "-d",
        "org.gnome.ScreenSaver",
        "-o",
        "/org/gnome/ScreenSaver",
        "-m",
        "org.gnome.ScreenSaver.GetActive",
    ]);
    if let Some(o) = doctor_process::output(&mut command, "probe screen-saver lock") {
        if o.status.success() {
            if let Some(active) = parse_screensaver_active(&String::from_utf8_lossy(&o.stdout)) {
                return Some(active);
            }
        }
    }
    None
}

/// The compiled screen-capture backend for this build (cfg + feature gated).
fn screen_backend() -> (&'static str, &'static str) {
    if cfg!(all(windows, feature = "capture-windows")) {
        ("ok", "windows-capture (WGC + DXGI fallback)")
    } else if cfg!(all(target_os = "macos", feature = "capture-macos")) {
        (
            "ok",
            "ScreenCaptureKit (cursor hidden, system audio supported)",
        )
    } else if cfg!(all(target_os = "linux", feature = "capture-linux")) {
        // Compiled — but the XDG ScreenCast portal only has a BACKEND inside a live
        // graphical session. At a display-manager greeter or over headless SSH (this
        // process has neither WAYLAND_DISPLAY nor DISPLAY) the portal yields ZERO frames,
        // so report honestly instead of a false "ok". cutd run from the desktop
        // inherits the session environment.
        if std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some() {
            // A graphical session IS present — but GNOME INHIBITS ScreenCast while the
            // screen is LOCKED, so a logged-in-but-locked session still captures ZERO
            // frames. Degrade ONLY on a positive locked signal (loginctl LockedHint or
            // org.gnome.ScreenSaver.GetActive); if neither tool answers, stay "ok" rather
            // than false-degrade a working desktop.
            if session_locked() == Some(true) {
                (
                    "degraded",
                    "desktop session is LOCKED — GNOME inhibits ScreenCast while locked; unlock the screen to record",
                )
            } else {
                ("ok", LINUX_PORTAL_BACKEND_DETAIL)
            }
        } else {
            (
                "degraded",
                "no graphical session (WAYLAND_DISPLAY/DISPLAY unset) — at a login greeter or headless SSH the ScreenCast portal has no backend; log into a desktop first",
            )
        }
    } else {
        (
            "missing",
            "live screen capture not compiled (ReplayCapture/import available)",
        )
    }
}

/// Report the native backend as `ok` only after its first real frame reached Cut.
/// Linux keeps its portal picker strictly user-initiated, so an otherwise usable
/// desktop is `unknown` rather than a false green card.
fn screen_capture_card() -> Card {
    let (status, detail) = screen_backend();
    if status != "ok" {
        return Card::new("screen_capture", "capture", status, detail);
    }
    let (status, detail) = doctor_probe::screen_probe().card(detail);
    Card::new("screen_capture", "capture", status, detail)
}

/// The compiled input-hook backend (cfg + feature gated).
fn input_backend() -> (&'static str, &'static str) {
    if cfg!(all(target_os = "linux", feature = "capture-linux")) {
        // rdevin works on X11; Wayland global input requires libei + the RemoteDesktop portal.
        (
            "ok",
            "rdevin global input hook (X11; Wayland global input is unavailable)",
        )
    } else if cfg!(any(
        all(windows, feature = "capture-windows"),
        all(target_os = "macos", feature = "capture-macos")
    )) {
        ("ok", "rdevin global input hook")
    } else {
        (
            "missing",
            "input hook not compiled (live capture feature off)",
        )
    }
}

/// Assemble cards around injected screen evidence so tests do not open capture.
fn doctor_with_screen_card(screen_card: Card) -> Vec<Card> {
    let mut cards = Vec::new();

    // ffmpeg (required for encode/decode).
    let mut ffmpeg = Command::new(ffmpeg_bin());
    ffmpeg.arg("-version");
    match doctor_process::output(&mut ffmpeg, "probe recording ffmpeg") {
        Some(o) if o.status.success() => {
            cards.push(Card::new("ffmpeg", "tool", "ok", first_line(&o.stdout)))
        }
        _ => cards.push(Card::new(
            "ffmpeg",
            "tool",
            "missing",
            "ffmpeg not found — install it or set SHELLX_RECORD_FFMPEG to its path",
        )),
    }

    cards.push(screen_card);

    let (in_status, in_detail) = input_backend();
    cards.push(Card::new("input_hook", "capture", in_status, in_detail));

    cards.push(Card::new(
        "webcam",
        "capture",
        "missing",
        "camera capture is not available in this release",
    ));

    // Linux capture needs the GStreamer pipewire plugin (the portal encode sink).
    #[cfg(all(target_os = "linux", feature = "capture-linux"))]
    {
        let gst = std::env::var("SHELLX_RECORD_GST").unwrap_or_else(|_| "gst-launch-1.0".into());
        let mut gst_command = Command::new(&gst);
        gst_command.arg("--version");
        let has_gst = doctor_process::output(&mut gst_command, "probe GStreamer")
            .is_some_and(|o| o.status.success());
        let mut pipewire_command = Command::new("gst-inspect-1.0");
        pipewire_command.arg("pipewiresrc");
        let has_pw = doctor_process::output(&mut pipewire_command, "probe GStreamer PipeWire")
            .is_some_and(|o| o.status.success());
        let (s, d) = match (has_gst, has_pw) {
            (true, true) => ("ok", "gst-launch-1.0 + pipewiresrc present".to_string()),
            (true, false) => (
                "degraded",
                "gst present but pipewiresrc missing — install gstreamer1.0-pipewire".to_string(),
            ),
            _ => (
                "missing",
                "gst-launch-1.0 not found — install gstreamer1.0-tools + gstreamer1.0-pipewire"
                    .to_string(),
            ),
        };
        cards.push(Card::new("gstreamer", "tool", s, d));

        // Input backend depends on session: X11 → rdevin (absolute); Wayland → evdev
        // (/dev/input), which needs `input`-group read access.
        let wayland = std::env::var("XDG_SESSION_TYPE")
            .map(|v| v.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false)
            || (std::env::var("WAYLAND_DISPLAY").is_ok() && std::env::var("DISPLAY").is_err());
        let (s, d) = if !wayland {
            (
                "ok",
                "X11 session → rdevin input hook (absolute coords)".to_string(),
            )
        } else if crate::input_evdev::evdev_readable() {
            (
                "ok",
                "Wayland session → evdev /dev/input readable (input group OK)".to_string(),
            )
        } else {
            ("degraded", "Wayland session but /dev/input not readable — add user to the 'input' group: sudo usermod -aG input $USER, then re-login".to_string())
        };
        cards.push(Card::new("wayland_input", "capture", s, d));
    }

    cards
}

pub fn doctor() -> Vec<Card> {
    doctor_with_screen_card(screen_capture_card())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_reports_all_cards() {
        let cards = doctor_with_screen_card(Card::new(
            "screen_capture",
            "capture",
            "unknown",
            "test avoids a native capture session",
        ));
        let ids: Vec<&str> = cards.iter().map(|c| c.id.as_str()).collect();
        assert!(ids.contains(&"ffmpeg"));
        assert!(ids.contains(&"screen_capture"));
        assert!(ids.contains(&"input_hook"));
        assert!(ids.contains(&"webcam"));
        let webcam = cards.iter().find(|card| card.id == "webcam").unwrap();
        assert_eq!(webcam.status, "missing");
        assert!(webcam.detail.contains("not available"));
        for c in &cards {
            assert!(["ok", "missing", "degraded", "unknown"].contains(&c.status.as_str()));
            assert!(!c.detail.is_empty());
        }
    }

    #[test]
    fn lock_hint_parse() {
        // Definite signals.
        assert_eq!(parse_locked_hint("LockedHint=yes\n"), Some(true));
        assert_eq!(parse_locked_hint("LockedHint=no\n"), Some(false));
        // Embedded in a multi-property dump (loginctl -p emits one line, but be robust).
        assert_eq!(
            parse_locked_hint("Id=3\nLockedHint=yes\nType=wayland\n"),
            Some(true)
        );
        // Whitespace tolerance.
        assert_eq!(parse_locked_hint("  LockedHint=no  "), Some(false));
        // Absent / empty / unparseable → None (must NOT false-degrade).
        assert_eq!(parse_locked_hint("Id=3\nType=wayland\n"), None);
        assert_eq!(parse_locked_hint(""), None);
        assert_eq!(parse_locked_hint("LockedHint=maybe"), None);
    }

    #[test]
    fn screensaver_active_parse() {
        assert_eq!(parse_screensaver_active("(true,)\n"), Some(true));
        assert_eq!(parse_screensaver_active("(false,)\n"), Some(false));
        assert_eq!(parse_screensaver_active(" (true,) "), Some(true));
        // Unknown / error text → None.
        assert_eq!(parse_screensaver_active(""), None);
        assert_eq!(parse_screensaver_active("Error: service unknown"), None);
    }

    #[test]
    fn unknown_screen_delivery_blocks_green_ready_state() {
        let cards = doctor_with_screen_card(Card::new(
            "screen_capture",
            "capture",
            "unknown",
            "backend exists but no frame was delivered",
        ));
        let card = cards
            .iter()
            .find(|card| card.id == "screen_capture")
            .unwrap();
        assert_eq!(card.status, "unknown");
    }
}

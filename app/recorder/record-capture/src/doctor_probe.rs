//! Bounded, privacy-preserving screen-delivery probe for `screen_record.doctor`.
//!
//! A linked capture backend or an enumerated display does not prove that Cut can
//! receive a frame. Platform implementations either deliver one discarded frame
//! within the fixed bound, or return an explicit observation. No probe writes
//! image data to disk or keeps a capture session running after it returns.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Maximum time a doctor may wait for a first frame.
pub(crate) const SCREEN_PROBE_TIMEOUT: Duration = Duration::from_millis(750);
const SCREEN_PROBE_CACHE_TTL: Duration = Duration::from_secs(30);
static SCREEN_PROBE_CACHE: OnceLock<Mutex<Option<(Instant, ScreenProbe)>>> = OnceLock::new();

/// The evidence gathered by the platform-specific delivery probe.
// Individual variants are constructed only by their platform backend, so a
// single-target release build legitimately leaves the other target's evidence
// paths unused. The deterministic tests exercise every public-status mapping.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScreenProbe {
    /// A callback delivered a real video frame to Cut; the frame was discarded.
    FrameDelivered,
    /// The backend started but no frame arrived within [`SCREEN_PROBE_TIMEOUT`].
    TimedOut,
    /// The OS positively denied access without Cut requesting a new prompt.
    PermissionDenied(String),
    /// A real probe would risk showing a picker or a new permission prompt.
    NotSafeToProbe(String),
    /// Starting the backend failed after its presence was established.
    Failed(String),
    /// The backend could not be cleanly stopped and reaped.
    CleanupFailed(String),
    /// No delivery check has run yet; a status read never starts one.
    NotChecked,
    /// The OS has not given this app screen-capture permission (read without
    /// asking). The reason names where the person grants it.
    PermissionNotGranted(String),
}

/// What a status read knows about screen capture without starting a capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenCheck {
    /// The card reflects a delivery check that already ran in this process.
    Checked,
    /// No delivery check has run yet. It runs when the person presses Check or
    /// starts a recording.
    NotChecked,
    /// The OS has not given screen-capture permission to this app.
    PermissionNeeded,
    /// This platform or build never runs a delivery check from Doctor.
    NotApplicable,
}

/// Plain reason shown when macOS has not given screen-capture permission.
#[cfg(all(target_os = "macos", feature = "capture-macos"))]
pub(crate) const MACOS_SCREEN_PERMISSION_NEEDED: &str = "screen recording permission is needed for ShellX Cut: turn it on in System Settings > Privacy & Security > Screen & System Audio Recording, then quit and reopen ShellX Cut";

impl ScreenProbe {
    /// Turn raw probe evidence into the public screen-capture card fields.
    pub(crate) fn card(&self, backend_detail: &str) -> (&'static str, String) {
        match self {
            Self::FrameDelivered => (
                "ok",
                format!(
                    "{backend_detail}; delivered one discarded frame to Cut within {} ms",
                    SCREEN_PROBE_TIMEOUT.as_millis()
                ),
            ),
            Self::TimedOut => (
                "degraded",
                format!(
                    "{backend_detail}; backend started but delivered no frame within {} ms",
                    SCREEN_PROBE_TIMEOUT.as_millis()
                ),
            ),
            Self::PermissionDenied(reason) => (
                "degraded",
                format!("{backend_detail}; capture permission denied: {reason}"),
            ),
            Self::NotSafeToProbe(reason) => (
                "unknown",
                format!("{backend_detail}; capture delivery is not verified: {reason}"),
            ),
            Self::Failed(reason) => (
                "degraded",
                format!("{backend_detail}; capture delivery probe failed: {reason}"),
            ),
            Self::CleanupFailed(reason) => (
                "degraded",
                format!("{backend_detail}; capture probe cleanup failed: {reason}"),
            ),
            Self::NotChecked => (
                "unknown",
                format!("{backend_detail}; not checked yet: screen capture is checked when you press Check or start a recording"),
            ),
            Self::PermissionNotGranted(reason) => (
                "unknown",
                format!("{backend_detail}; {reason}"),
            ),
        }
    }
}

/// Deliverability evidence for the compiled live-capture backend. On Windows
/// and macOS this starts a short capture, so only an action the person asked
/// for (Check, Test, or starting a recording) may call it.
pub(crate) fn screen_probe() -> ScreenProbe {
    crate::screen_access::count_screen_probe();
    if !crate::screen_access::admit_native_screen_access() {
        return ScreenProbe::NotSafeToProbe(NATIVE_ACCESS_REFUSED.into());
    }
    let mut cache = probe_cache();
    if let Some((at, observation)) = cache.as_ref() {
        if at.elapsed() < SCREEN_PROBE_CACHE_TTL {
            return observation.clone();
        }
    }
    let observation = screen_probe_uncached();
    *cache = Some((Instant::now(), observation.clone()));
    observation
}

const NATIVE_ACCESS_REFUSED: &str = "native screen access is turned off in this process";

fn probe_cache() -> std::sync::MutexGuard<'static, Option<(Instant, ScreenProbe)>> {
    let cache = SCREEN_PROBE_CACHE.get_or_init(|| Mutex::new(None));
    match cache.lock() {
        Ok(cache) => cache,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Screen-capture readiness for a status display. It never starts a capture
/// and never shows a system prompt: macOS reads the permission with the
/// non-prompting preflight, and the delivery result is the last check that
/// ran in this process, or "not checked yet".
pub(crate) fn screen_status() -> (ScreenProbe, ScreenCheck) {
    #[cfg(all(target_os = "linux", feature = "capture-linux"))]
    {
        // Linux Doctor never opens the portal, so its full answer is passive.
        return (screen_probe_uncached(), ScreenCheck::NotApplicable);
    }
    #[cfg(all(target_os = "macos", feature = "capture-macos"))]
    {
        match crate::screen_access::screen_capture_access_preflight() {
            Some(false) => {
                return (
                    ScreenProbe::PermissionNotGranted(MACOS_SCREEN_PERMISSION_NEEDED.into()),
                    ScreenCheck::PermissionNeeded,
                )
            }
            Some(true) => {}
            None => {
                return (
                    ScreenProbe::NotSafeToProbe(NATIVE_ACCESS_REFUSED.into()),
                    ScreenCheck::NotChecked,
                )
            }
        }
    }
    #[cfg(any(
        all(windows, feature = "capture-windows"),
        all(target_os = "macos", feature = "capture-macos")
    ))]
    {
        if let Some((_, observation)) = probe_cache().as_ref() {
            return (observation.clone(), ScreenCheck::Checked);
        }
        return (ScreenProbe::NotChecked, ScreenCheck::NotChecked);
    }
    #[allow(unreachable_code)]
    (
        ScreenProbe::NotSafeToProbe("no live screen-capture backend is compiled".into()),
        ScreenCheck::NotApplicable,
    )
}

fn screen_probe_uncached() -> ScreenProbe {
    #[cfg(all(windows, feature = "capture-windows"))]
    {
        return crate::windows_probe::screen_probe();
    }
    #[cfg(all(target_os = "macos", feature = "capture-macos"))]
    {
        return crate::macos_probe::screen_probe();
    }
    #[cfg(all(target_os = "linux", feature = "capture-linux"))]
    {
        // The XDG ScreenCast API's source selection is permission-sensitive. Even a
        // restore token may be rejected and reopen the portal picker, so doctor never
        // invokes it. `screen_record.start` remains the user-initiated consent path.
        return ScreenProbe::NotSafeToProbe(
            crate::doctor_portal::LINUX_PORTAL_PROMPT_DEFERRED_REASON.into(),
        );
    }
    #[allow(unreachable_code)]
    ScreenProbe::NotSafeToProbe("no live screen-capture backend is compiled".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_evidence_never_turns_backend_presence_into_green() {
        let backend = "test backend";
        let cases = [
            (ScreenProbe::FrameDelivered, "ok"),
            (ScreenProbe::TimedOut, "degraded"),
            (ScreenProbe::PermissionDenied("denied".into()), "degraded"),
            (ScreenProbe::NotSafeToProbe("picker risk".into()), "unknown"),
            (
                ScreenProbe::Failed("backend was present".into()),
                "degraded",
            ),
            (ScreenProbe::CleanupFailed("join failed".into()), "degraded"),
            (ScreenProbe::NotChecked, "unknown"),
            (
                ScreenProbe::PermissionNotGranted("permission is needed".into()),
                "unknown",
            ),
        ];
        for (observation, expected_status) in cases {
            let (status, detail) = observation.card(backend);
            assert_eq!(status, expected_status, "{observation:?}");
            assert!(detail.contains(backend));
        }
    }

    #[test]
    fn delivered_frame_is_explicitly_discarded() {
        let (_, detail) = ScreenProbe::FrameDelivered.card("WGC");
        assert!(detail.contains("discarded frame"));
        assert!(detail.contains("750 ms"));
    }
}

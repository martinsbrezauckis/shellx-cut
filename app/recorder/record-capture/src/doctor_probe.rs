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
}

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
        }
    }
}

/// Deliverability evidence for the compiled live-capture backend.
pub(crate) fn screen_probe() -> ScreenProbe {
    let cache = SCREEN_PROBE_CACHE.get_or_init(|| Mutex::new(None));
    let mut cache = match cache.lock() {
        Ok(cache) => cache,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some((at, observation)) = cache.as_ref() {
        if at.elapsed() < SCREEN_PROBE_CACHE_TTL {
            return observation.clone();
        }
    }
    let observation = screen_probe_uncached();
    *cache = Some((Instant::now(), observation.clone()));
    observation
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

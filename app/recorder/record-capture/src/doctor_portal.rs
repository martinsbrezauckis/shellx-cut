//! Linux's deliberately prompt-deferred ScreenCast Doctor observation.

pub(crate) const LINUX_PORTAL_BACKEND_DETAIL: &str =
    "XDG ScreenCast portal + GStreamer (cursor hidden, CFR normalize)";
#[cfg(any(all(target_os = "linux", feature = "capture-linux"), test))]
pub(crate) const LINUX_PORTAL_PROMPT_DEFERRED_REASON: &str =
    "the XDG ScreenCast portal would need source selection; doctor will not open its picker or request consent";
pub const LINUX_PORTAL_PROMPT_DEFERRED_DETAIL: &str =
    "XDG ScreenCast portal + GStreamer (cursor hidden, CFR normalize); capture delivery is not verified: the XDG ScreenCast portal would need source selection; doctor will not open its picker or request consent";

/// True only for Linux's deliberate, user-action-owned portal-picker state.
///
/// This is intentionally stricter than `status == "unknown"`: any unexpected
/// probe observation remains non-admissible until its own action policy exists.
pub fn is_linux_portal_prompt_deferred(status: &str, detail: &str) -> bool {
    cfg!(all(target_os = "linux", feature = "capture-linux"))
        && status == "unknown"
        && detail == LINUX_PORTAL_PROMPT_DEFERRED_DETAIL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_deferred_portal_contract_is_exact_and_fail_closed() {
        let (status, detail) = crate::doctor_probe::ScreenProbe::NotSafeToProbe(
            LINUX_PORTAL_PROMPT_DEFERRED_REASON.into(),
        )
        .card(LINUX_PORTAL_BACKEND_DETAIL);
        assert_eq!(status, "unknown");
        assert_eq!(detail, LINUX_PORTAL_PROMPT_DEFERRED_DETAIL);
        assert_eq!(
            is_linux_portal_prompt_deferred(status, &detail),
            cfg!(all(target_os = "linux", feature = "capture-linux"))
        );
        assert!(!is_linux_portal_prompt_deferred("degraded", &detail));
        assert!(!is_linux_portal_prompt_deferred(
            "unknown",
            "an unrelated prompt-deferred backend"
        ));
    }
}

//! Passive system-audio capability card for recorder Doctor.
//!
//! Doctor must not start an audio stream. On macOS, starting the Core Audio
//! aggregate tap is the action that may request Audio Capture permission; on
//! Linux and Windows, a live stream is still required to prove packet delivery.

use crate::doctor::Card;

pub(crate) fn card() -> Card {
    let (status, detail) = backend();
    Card {
        id: "system_audio".into(),
        kind: "capture".into(),
        status: status.into(),
        detail: detail.into(),
    }
}

fn backend() -> (&'static str, &'static str) {
    if cfg!(all(target_os = "macos", feature = "capture-macos")) {
        (
            "unknown",
            "Core Audio system capture is compiled; use Test system audio to check Audio Capture permission and packet delivery because Doctor will not start an aggregate tap or trigger the system prompt",
        )
    } else if cfg!(all(windows, feature = "capture-windows")) {
        (
            "unknown",
            "WASAPI process loopback is compiled; use Test system audio to check first-packet delivery because Doctor will not start a loopback stream",
        )
    } else if cfg!(all(target_os = "linux", feature = "capture-linux")) {
        (
            "unknown",
            "PipeWire default-sink monitor capture is compiled; use Test system audio to check first-packet delivery because Doctor will not open the monitor stream",
        )
    } else {
        (
            "missing",
            "system-audio capture is not compiled for this build",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::card;

    #[test]
    fn passive_card_never_claims_unobserved_delivery() {
        let card = card();
        assert_eq!(card.id, "system_audio");
        assert_eq!(card.kind, "capture");
        assert!(matches!(card.status.as_str(), "unknown" | "missing"));
        assert!(!card.detail.is_empty());
        if card.status == "unknown" {
            assert!(card.detail.contains("Test system audio"));
        }
    }
}

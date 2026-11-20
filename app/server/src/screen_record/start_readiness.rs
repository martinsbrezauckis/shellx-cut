//! Action-specific admission for `screen_record.start`.
//!
//! Doctor is deliberately non-consenting: on Linux it leaves a usable XDG
//! ScreenCast portal `unknown` rather than opening its source picker. Starting a
//! capture is the user-initiated path, so it may admit that one exact observation
//! while keeping the public Doctor rollup strictly non-ready.

use super::{required_capture_card, RecordCard, RecordStartAdmission};
use cut_core::{error_codes, CutError};

/// Whether Start may be attempted. A status read that did not check screen
/// delivery still admits the attempt, because Start runs that check itself.
pub(super) fn start_allowed(cards: &[RecordCard]) -> bool {
    start_allowed_for_platform(cards, cfg!(target_os = "linux"))
}

pub(super) fn ensure_start_ready(cards: &[RecordCard]) -> Result<(), CutError> {
    ensure_start_ready_for_platform(cards, cfg!(target_os = "linux"))
}

fn ensure_start_ready_for_platform(cards: &[RecordCard], is_linux: bool) -> Result<(), CutError> {
    // Admission always uses a full check, so a "checked at start" card never
    // admits here.
    let blocked = blocked_cards(cards, is_linux, false);
    if blocked.is_empty() {
        return Ok(());
    }
    // Name the first missing requirement for the registry (the cause lists all).
    let requirement = if blocked.iter().any(|item| item.starts_with("ffmpeg=")) {
        "ffmpeg"
    } else {
        "screen_permission"
    };
    Err(CutError::new(
        error_codes::NOT_FOUND,
        "screen recording is not ready on this system",
        blocked.join("; "),
    )
    .with_requirement(requirement)
    .with_suggested_action(
        "Tell the person what the cause names and where to provide it (for screen recording permission: System Settings > Privacy & Security > Screen & System Audio Recording, then quit and reopen ShellX Cut), then start the recording again",
    ))
}

fn start_allowed_for_platform(cards: &[RecordCard], is_linux: bool) -> bool {
    blocked_cards(cards, is_linux, true).is_empty()
}

fn blocked_cards(cards: &[RecordCard], is_linux: bool, attempt: bool) -> Vec<String> {
    [
        "ffmpeg",
        "screen_capture",
        "input_hook",
        "gstreamer",
        "wayland_input",
    ]
    .into_iter()
    .filter(|name| required_capture_card(cards, name))
    .filter_map(|name| {
        let card = cards.iter().find(|card| card.name == name);
        match card {
            Some(card)
                if card.status == "ok"
                    || linux_portal_prompt_deferred(card, is_linux)
                    || (attempt && checked_at_start(card)) =>
            {
                None
            }
            Some(card) => Some(format!("{}={} ({})", card.name, card.status, card.detail)),
            None => Some(format!("{name}=missing")),
        }
    })
    .collect()
}

fn checked_at_start(card: &RecordCard) -> bool {
    card.name == "screen_capture"
        && card.status == "unknown"
        && card.start_admission == RecordStartAdmission::CheckedAtStart
}

fn linux_portal_prompt_deferred(card: &RecordCard, is_linux: bool) -> bool {
    is_linux
        && card.name == "screen_capture"
        && card.status == "unknown"
        && card.start_admission == RecordStartAdmission::LinuxPortalPromptDeferred
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(name: &str, status: &str, admission: RecordStartAdmission) -> RecordCard {
        RecordCard {
            name: name.into(),
            status: status.into(),
            detail: String::new(),
            start_admission: admission,
        }
    }

    fn linux_cards(screen_status: &str, admission: RecordStartAdmission) -> Vec<RecordCard> {
        vec![
            card("ffmpeg", "ok", RecordStartAdmission::Strict),
            card("screen_capture", screen_status, admission),
            card("input_hook", "ok", RecordStartAdmission::Strict),
            card("gstreamer", "ok", RecordStartAdmission::Strict),
            card("wayland_input", "ok", RecordStartAdmission::Strict),
        ]
    }

    #[test]
    fn linux_prompt_deferred_portal_unknown_allows_only_start() {
        let cards = linux_cards("unknown", RecordStartAdmission::LinuxPortalPromptDeferred);
        assert!(start_allowed_for_platform(&cards, true));
        assert!(ensure_start_ready_for_platform(&cards, true).is_ok());
        assert!(!start_allowed_for_platform(&cards, false));
        assert!(!super::super::ready_rollup(&cards));
    }

    #[test]
    fn degraded_missing_and_arbitrary_unknown_screen_capture_remain_blocked() {
        for (status, admission) in [
            ("degraded", RecordStartAdmission::LinuxPortalPromptDeferred),
            ("missing", RecordStartAdmission::Strict),
            ("unknown", RecordStartAdmission::Strict),
        ] {
            let cards = linux_cards(status, admission);
            assert!(
                !start_allowed_for_platform(&cards, true),
                "screen_capture={status} must not reach native start"
            );
            assert!(ensure_start_ready_for_platform(&cards, true).is_err());
        }
    }

    #[test]
    fn another_required_unknown_still_blocks_linux_portal_start() {
        let mut cards = linux_cards("unknown", RecordStartAdmission::LinuxPortalPromptDeferred);
        cards[0].status = "unknown".into();
        assert!(!start_allowed_for_platform(&cards, true));
        assert!(ensure_start_ready_for_platform(&cards, true).is_err());
    }

    #[test]
    fn unchecked_screen_admits_an_attempt_but_never_a_start() {
        let cards = vec![
            card("ffmpeg", "ok", RecordStartAdmission::Strict),
            card(
                "screen_capture",
                "unknown",
                RecordStartAdmission::CheckedAtStart,
            ),
            card("input_hook", "ok", RecordStartAdmission::Strict),
        ];
        assert!(start_allowed_for_platform(&cards, false));
        let error = ensure_start_ready_for_platform(&cards, false).unwrap_err();
        assert!(error.suggested_action.unwrap().contains("System Settings"));
    }

    #[test]
    fn optional_system_audio_unknown_does_not_gate_screen_only_start() {
        let mut cards = linux_cards("ok", RecordStartAdmission::Strict);
        cards.push(card(
            "system_audio",
            "unknown",
            RecordStartAdmission::Strict,
        ));
        assert!(super::super::ready_rollup(&cards));
        assert!(start_allowed_for_platform(&cards, true));
    }
}

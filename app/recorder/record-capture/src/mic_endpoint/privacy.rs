//! Microphone privacy state and plain failure reasons.
//!
//! Windows keeps three microphone switches on its Microphone privacy page
//! (Settings, Privacy and security, Microphone). Each one is a `Value` (`Allow`
//! or `Deny`) in the capability consent store: the device-wide "Microphone
//! access" (machine key), "Let apps access your microphone" (user key) and
//! "Let desktop apps access your microphone" (the user key's `NonPackaged`
//! subkey). A classic desktop app cannot ask the WinRT access APIs for this
//! state, so the consent store is read behind [`ConsentSource`] and classified
//! here without any OS call.
//!
//! The same module turns a failed microphone open into one of a few plain
//! problems, so a block, a missing device and a busy device are never reported
//! as one generic failure.

// The consent-store reading is called on Windows only; the other systems build
// this module for its shared types and tests.
#![cfg_attr(not(any(windows, test)), allow(dead_code))]

use record_core::{error_codes, RecordError};
use serde::{Deserialize, Serialize};

use super::MicrophoneAccess;

/// One of the three Windows microphone switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MicrophoneSwitch {
    /// "Microphone access": the device-wide switch.
    Device,
    /// "Let apps access your microphone": the per-user switch for all apps.
    Apps,
    /// "Let desktop apps access your microphone".
    DesktopApps,
}

/// Which stored value to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentScope {
    Device,
    Apps,
    DesktopApps,
}

/// Source of the consent-store values. The real one reads the registry; tests
/// supply fixed values.
pub trait ConsentSource {
    /// The `Value` text of the scope's key, `None` when it is absent or unreadable.
    fn value(&self, scope: ConsentScope) -> Option<String>;
}

fn denies(value: Option<String>) -> bool {
    value.is_some_and(|text| text.eq_ignore_ascii_case("deny"))
}

/// The first switch that is off, checked from the widest to the narrowest.
pub fn blocking_switch(source: &dyn ConsentSource) -> Option<MicrophoneSwitch> {
    if denies(source.value(ConsentScope::Device)) {
        Some(MicrophoneSwitch::Device)
    } else if denies(source.value(ConsentScope::Apps)) {
        Some(MicrophoneSwitch::Apps)
    } else if denies(source.value(ConsentScope::DesktopApps)) {
        Some(MicrophoneSwitch::DesktopApps)
    } else {
        None
    }
}

/// The permission answer for the consent store: `Denied` when a switch is off,
/// `Granted` when the store was readable and none is, `NotApplicable` when no
/// value could be read at all (the state is then unknown, never a block).
pub fn access_from(source: &dyn ConsentSource) -> MicrophoneAccess {
    if blocking_switch(source).is_some() {
        return MicrophoneAccess::Denied;
    }
    let readable = [
        ConsentScope::Device,
        ConsentScope::Apps,
        ConsentScope::DesktopApps,
    ]
    .into_iter()
    .any(|scope| source.value(scope).is_some());
    if readable {
        MicrophoneAccess::Granted
    } else {
        MicrophoneAccess::NotApplicable
    }
}

/// Why the microphone cannot be used, in plain words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MicrophoneProblem {
    /// A Windows privacy switch is off.
    Blocked(MicrophoneSwitch),
    /// Windows refused the open with "access denied" but no switch is readable.
    AccessDenied,
    /// No microphone device is present.
    NoDevice,
    /// The microphone is used by another recording or another app.
    InUse,
    /// The chosen microphone is missing or was unplugged.
    Unavailable,
    /// Any other failure; the system's text stays in the technical detail.
    Other,
}

impl MicrophoneProblem {
    /// Stable machine-readable code for agents and tests.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Blocked(MicrophoneSwitch::Device) => "windows_microphone_access_off",
            Self::Blocked(MicrophoneSwitch::Apps) => "windows_apps_microphone_off",
            Self::Blocked(MicrophoneSwitch::DesktopApps) => "windows_desktop_apps_microphone_off",
            Self::AccessDenied => "windows_microphone_access_denied",
            Self::NoDevice => "no_microphone_device",
            Self::InUse => "microphone_in_use",
            Self::Unavailable => "microphone_unavailable",
            Self::Other => "microphone_open_failed",
        }
    }

    /// One sentence saying what is wrong.
    pub fn message(self) -> &'static str {
        match self {
            Self::Blocked(MicrophoneSwitch::Device) => {
                "Windows is blocking microphone access on this PC"
            }
            Self::Blocked(MicrophoneSwitch::Apps) => {
                "Windows is blocking microphone access for apps"
            }
            Self::Blocked(MicrophoneSwitch::DesktopApps) => {
                "Windows is blocking microphone access for desktop apps"
            }
            Self::AccessDenied => "Windows denied ShellX Cut access to the microphone",
            Self::NoDevice => "No microphone was found",
            Self::InUse => "The microphone is in use by another recording or app",
            Self::Unavailable => "The chosen microphone is missing or was unplugged",
            Self::Other => "ShellX Cut could not open the microphone",
        }
    }

    /// What the person (or an agent speaking to the person) can do about it.
    pub fn suggested_action(self) -> &'static str {
        match self {
            Self::Blocked(MicrophoneSwitch::Device) => {
                "Turn on Settings > Privacy & security > Microphone > Microphone access (it needs an administrator), then try again"
            }
            Self::Blocked(MicrophoneSwitch::Apps) => {
                "Turn on Settings > Privacy & security > Microphone > Let apps access your microphone, then try again"
            }
            Self::Blocked(MicrophoneSwitch::DesktopApps) => {
                "Turn on Settings > Privacy & security > Microphone > Let desktop apps access your microphone, then try again"
            }
            Self::AccessDenied => {
                "In Settings > Privacy & security > Microphone, turn on Microphone access, Let apps access your microphone and Let desktop apps access your microphone, then try again"
            }
            Self::NoDevice => {
                "Connect a microphone, or turn on a disabled one in Settings > System > Sound > Input, then try again"
            }
            Self::InUse => {
                "Stop the other recording or close the app that uses the microphone, then try again"
            }
            Self::Unavailable => {
                "Plug the microphone back in, or choose another microphone or System Default, then try again"
            }
            Self::Other => {
                "Check that the microphone is connected and enabled, then try again; the technical detail names the system error"
            }
        }
    }

    /// The recorder error for this problem. The original error's text is kept
    /// in the technical cause so no system detail is lost.
    pub fn into_error(self, original: Option<&RecordError>) -> RecordError {
        let cause = match original {
            Some(original) => format!("{}: {}", original.message, original.cause),
            None => self.reason().to_string(),
        };
        RecordError::new(error_codes::CAPTURE, self.message(), cause)
            .with_action(self.suggested_action())
    }

    /// Recognise an error built by [`Self::into_error`].
    pub fn from_error(error: &RecordError) -> Option<Self> {
        let mut known = vec![
            Self::AccessDenied,
            Self::NoDevice,
            Self::InUse,
            Self::Unavailable,
            Self::Other,
            Self::Blocked(MicrophoneSwitch::Device),
            Self::Blocked(MicrophoneSwitch::Apps),
            Self::Blocked(MicrophoneSwitch::DesktopApps),
        ];
        known.retain(|problem| problem.message() == error.message);
        known.pop()
    }
}

/// True for the errors raised while resolving or opening the microphone. Other
/// failures (writing the WAV, publishing it) keep their own text.
fn is_open_stage(error: &RecordError) -> bool {
    matches!(
        error.message.as_str(),
        "mic config"
            | "build audio stream"
            | "start microphone audio"
            | "no input device"
            | "selected microphone is unavailable"
            | "selected microphone cannot be opened"
            | "enumerate microphone inputs"
    )
}

/// Classify a failed open. `block` is the consent-store answer: when a switch
/// is off it names the cause, whatever error the audio stack returned.
pub fn classify_open_failure(
    error: &RecordError,
    block: Option<MicrophoneSwitch>,
) -> MicrophoneProblem {
    if let Some(switch) = block {
        return MicrophoneProblem::Blocked(switch);
    }
    let text = format!("{} {}", error.message, error.cause).to_ascii_lowercase();
    let has = |needle: &str| text.contains(needle);
    if has("0x80070005") || has("access is denied") || has("e_accessdenied") {
        MicrophoneProblem::AccessDenied
    } else if has("0x8889000a") || has("device_in_use") || has("already in use") || has("exclusive")
    {
        MicrophoneProblem::InUse
    } else if has("no input device") || has("no default microphone") || has("no microphone found") {
        MicrophoneProblem::NoDevice
    } else if has("selected microphone")
        || has("0x88890004")
        || has("device_not_available")
        || has("no longer available")
    {
        MicrophoneProblem::Unavailable
    } else {
        MicrophoneProblem::Other
    }
}

/// Rewrite an open-stage error into its plain problem; any other error is
/// returned unchanged.
pub fn explain_with(error: RecordError, block: Option<MicrophoneSwitch>) -> RecordError {
    if !is_open_stage(&error) {
        return error;
    }
    classify_open_failure(&error, block).into_error(Some(&error))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed {
        device: Option<&'static str>,
        apps: Option<&'static str>,
        desktop: Option<&'static str>,
    }

    impl ConsentSource for Fixed {
        fn value(&self, scope: ConsentScope) -> Option<String> {
            match scope {
                ConsentScope::Device => self.device,
                ConsentScope::Apps => self.apps,
                ConsentScope::DesktopApps => self.desktop,
            }
            .map(str::to_string)
        }
    }

    fn all(value: &'static str) -> Fixed {
        Fixed {
            device: Some(value),
            apps: Some(value),
            desktop: Some(value),
        }
    }

    #[test]
    fn every_switch_on_is_granted() {
        assert_eq!(blocking_switch(&all("Allow")), None);
        assert_eq!(access_from(&all("Allow")), MicrophoneAccess::Granted);
    }

    #[test]
    fn each_switch_off_names_itself_widest_first() {
        let device = Fixed {
            device: Some("Deny"),
            ..all("Allow")
        };
        let apps = Fixed {
            apps: Some("Deny"),
            ..all("Allow")
        };
        let desktop = Fixed {
            desktop: Some("deny"),
            ..all("Allow")
        };
        assert_eq!(blocking_switch(&device), Some(MicrophoneSwitch::Device));
        assert_eq!(blocking_switch(&apps), Some(MicrophoneSwitch::Apps));
        assert_eq!(
            blocking_switch(&desktop),
            Some(MicrophoneSwitch::DesktopApps)
        );
        assert_eq!(
            blocking_switch(&all("Deny")),
            Some(MicrophoneSwitch::Device)
        );
        assert_eq!(access_from(&desktop), MicrophoneAccess::Denied);
    }

    #[test]
    fn prompt_or_unknown_values_do_not_block() {
        assert_eq!(blocking_switch(&all("Prompt")), None);
        assert_eq!(access_from(&all("Prompt")), MicrophoneAccess::Granted);
    }

    #[test]
    fn an_unreadable_store_is_unknown_not_blocked() {
        let nothing = Fixed {
            device: None,
            apps: None,
            desktop: None,
        };
        assert_eq!(blocking_switch(&nothing), None);
        assert_eq!(access_from(&nothing), MicrophoneAccess::NotApplicable);
    }

    #[test]
    fn the_desktop_apps_message_names_the_verified_setting() {
        let problem = MicrophoneProblem::Blocked(MicrophoneSwitch::DesktopApps);
        assert_eq!(
            problem.message(),
            "Windows is blocking microphone access for desktop apps"
        );
        assert!(problem.suggested_action().contains(
            "Settings > Privacy & security > Microphone > Let desktop apps access your microphone"
        ));
        assert_eq!(problem.reason(), "windows_desktop_apps_microphone_off");
    }

    fn open_error(message: &str, cause: &str) -> RecordError {
        RecordError::new(error_codes::CAPTURE, message, cause)
    }

    #[test]
    fn access_denied_from_the_audio_stack_is_a_windows_block() {
        let error = open_error("build audio stream", "Access is denied. (0x80070005)");
        assert_eq!(
            classify_open_failure(&error, None),
            MicrophoneProblem::AccessDenied
        );
        let explained = explain_with(error, None);
        assert_eq!(explained.message, MicrophoneProblem::AccessDenied.message());
        assert!(
            explained.cause.contains("0x80070005"),
            "system text is kept"
        );
        assert!(explained
            .suggested_action
            .as_deref()
            .is_some_and(|action| action.contains("Let desktop apps access your microphone")));
    }

    #[test]
    fn a_readable_switch_names_the_cause_over_any_open_error() {
        let error = open_error("mic config", "The device is not responding");
        let problem = classify_open_failure(&error, Some(MicrophoneSwitch::Apps));
        assert_eq!(problem, MicrophoneProblem::Blocked(MicrophoneSwitch::Apps));
    }

    #[test]
    fn missing_busy_and_other_failures_each_get_their_own_reason() {
        let cases = [
            (
                open_error("no input device", "no default microphone found"),
                MicrophoneProblem::NoDevice,
            ),
            (
                open_error(
                    "start microphone audio",
                    "The device is already in use. (0x8889000A)",
                ),
                MicrophoneProblem::InUse,
            ),
            (
                open_error(
                    "selected microphone is unavailable",
                    "the privately saved native microphone endpoint is absent",
                ),
                MicrophoneProblem::Unavailable,
            ),
            (
                open_error(
                    "mic config",
                    "The requested device is no longer available (0x88890004)",
                ),
                MicrophoneProblem::Unavailable,
            ),
            (
                open_error("mic config", "unexpected driver failure (0x80004005)"),
                MicrophoneProblem::Other,
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(classify_open_failure(&error, None), expected, "{error}");
            let explained = explain_with(error.clone(), None);
            assert_eq!(MicrophoneProblem::from_error(&explained), Some(expected));
            assert!(
                explained.cause.contains(&error.cause),
                "system text is kept"
            );
        }
    }

    #[test]
    fn failures_after_the_open_keep_their_own_text() {
        let error = open_error("write microphone audio", "disk full");
        assert_eq!(explain_with(error.clone(), None), error);
    }
}

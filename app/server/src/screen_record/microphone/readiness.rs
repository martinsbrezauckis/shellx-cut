//! Microphone-only readiness for Timeline voiceover.
//!
//! Voiceover records the microphone alone, so neither its readiness read nor
//! its start admission reads, lists, or probes screen capture. A missing
//! prerequisite is returned as one plain sentence that names the permission or
//! device and where the person provides it. Windows reports a Windows privacy
//! switch that is off, or the absence of any microphone, in the same shape.

use cut_core::{error_codes, CutError};
use record_capture::{MicrophoneAccess, MicrophoneProblem, MicrophoneSwitch};
use serde_json::{json, Value};

const SETTINGS_ACTION: &str = "Turn on ShellX Cut in System Settings > Privacy & Security > Microphone, then press Record voiceover again";

/// What voiceover needs, gathered without opening the microphone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MicrophoneReadiness {
    pub compiled: bool,
    pub access: MicrophoneAccess,
    /// The Windows privacy switch that is off, when one is.
    pub windows_switch: Option<MicrophoneSwitch>,
    /// The native input inventory lists no microphone.
    pub no_device: bool,
}

/// A missing prerequisite: a stable reason code and the error that names it.
pub(crate) struct Blocker {
    pub reason: &'static str,
    pub error: CutError,
}

impl MicrophoneReadiness {
    /// Read the current state without prompting or starting an audio stream.
    #[cfg(any(not(target_os = "linux"), test))]
    pub(crate) fn current() -> Self {
        #[cfg(test)]
        if let Some(readiness) = TEST_READINESS.with(std::cell::Cell::get) {
            return readiness;
        }
        Self {
            compiled: record_capture::MICROPHONE_CAPTURE_COMPILED,
            access: record_capture::microphone_access(),
            windows_switch: record_capture::microphone_privacy_block(),
            no_device: record_capture::no_microphone_device(),
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) async fn current_linux() -> Self {
        #[cfg(test)]
        if let Some(readiness) = TEST_READINESS.with(std::cell::Cell::get) {
            return readiness;
        }
        Self {
            compiled: record_capture::MICROPHONE_CAPTURE_COMPILED,
            access: record_capture::microphone_access(),
            windows_switch: None,
            no_device: linux_no_device(crate::linux_microphone_probe::availability().await),
        }
    }

    /// The missing prerequisite that blocks a take, if any.
    pub(crate) fn blocker(self) -> Option<CutError> {
        self.problem().map(|blocker| blocker.error)
    }

    /// The missing prerequisite with its reason code, if any.
    pub(crate) fn problem(self) -> Option<Blocker> {
        if !self.compiled {
            return Some(Blocker {
                reason: "no_recorder",
                error: CutError::new(
                    error_codes::UNIMPLEMENTED,
                    "a microphone recorder is needed to record voiceover, and this ShellX Cut build has none",
                    "microphone capture is not compiled into this build",
                )
                .with_suggested_action(
                    "Record voiceover in the ShellX Cut desktop app for Windows, macOS or Linux",
                ),
            });
        }
        if let Some(switch) = self.windows_switch {
            return Some(windows_blocker(MicrophoneProblem::Blocked(switch)));
        }
        if self.no_device {
            let mut blocker = windows_blocker(MicrophoneProblem::NoDevice);
            if !cfg!(windows) {
                blocker.error.cause = "native microphone inventory before any take".into();
                blocker.error.suggested_action = Some(if cfg!(target_os = "macos") {
                    "Connect a microphone, or enable an input in System Settings > Sound > Input, then try again".into()
                } else {
                    "Connect a microphone, or enable an input in your system sound settings, then try again".into()
                });
            }
            return Some(blocker);
        }
        let (reason, cause) = match self.access {
            MicrophoneAccess::Denied => (
                "macos_denied",
                "macOS has not given ShellX Cut access to the microphone",
            ),
            MicrophoneAccess::Restricted => (
                "macos_restricted",
                "microphone access for ShellX Cut is restricted on this Mac (for example by a device policy)",
            ),
            _ => return None,
        };
        Some(Blocker {
            reason,
            error: CutError::new(
                error_codes::NOT_FOUND,
                "microphone permission is needed to record voiceover",
                cause,
            )
            .with_suggested_action(SETTINGS_ACTION),
        })
    }
}

#[cfg(target_os = "linux")]
fn linux_no_device(answer: crate::linux_microphone_probe::Availability) -> bool {
    answer == crate::linux_microphone_probe::Availability::Absent
}

/// The Windows problem as a refusal: its plain sentence and where to fix it.
fn windows_blocker(problem: MicrophoneProblem) -> Blocker {
    Blocker {
        reason: problem.reason(),
        error: CutError::new(
            error_codes::NOT_FOUND,
            problem.message(),
            "Windows microphone readiness read before any take",
        )
        .with_suggested_action(problem.suggested_action()),
    }
}

/// Admission for a voiceover start. When the person has not answered the
/// macOS microphone question yet, this asks the OS to show it and returns a
/// refusal that says what to do; the next start records.
pub(crate) fn admit_voiceover(
    readiness: MicrophoneReadiness,
    request_access: impl FnOnce(),
) -> Result<(), CutError> {
    if let Some(error) = readiness.blocker() {
        return Err(error);
    }
    if readiness.access == MicrophoneAccess::NotDetermined {
        request_access();
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            "microphone permission is needed to record voiceover",
            "macOS is now asking whether ShellX Cut may use the microphone",
        )
        .with_suggested_action(
            "Choose Allow in the macOS microphone prompt, then press Record voiceover again",
        ));
    }
    Ok(())
}

/// `{reason, message, suggested_action}` for the missing prerequisite, or null.
pub(crate) fn blocked_projection(readiness: MicrophoneReadiness) -> Value {
    readiness.problem().map_or(Value::Null, |blocker| {
        json!({
            "reason": blocker.reason,
            "message": blocker.error.message,
            "suggested_action": blocker.error.suggested_action,
        })
    })
}

/// The `screen_record.doctor {mode:"microphone"}` result. It lists the
/// microphone choice and permission only; no screen, window, or camera read.
pub(crate) fn projection(readiness: MicrophoneReadiness, mic_warm: Option<Value>) -> Value {
    let microphone = super::doctor_projection();
    json!({
        "mode": "microphone",
        "start_allowed": readiness.problem().is_none(),
        "microphone_access": readiness.access,
        "microphone_blocked": blocked_projection(readiness),
        "microphones": microphone.microphones,
        "microphone_selection": microphone.microphone_selection,
        "mic_warm": mic_warm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn only_a_completed_absent_linux_answer_blocks_microphone_start() {
        use crate::linux_microphone_probe::Availability;
        for (answer, blocked) in [
            (Availability::Present, false),
            (Availability::Absent, true),
            (Availability::Unknown, false),
        ] {
            let readiness = MicrophoneReadiness {
                no_device: linux_no_device(answer),
                ..ready(MicrophoneAccess::NotApplicable)
            };
            assert_eq!(readiness.problem().is_some(), blocked, "{answer:?}");
            let doctor = projection(readiness, None);
            assert_eq!(doctor["start_allowed"], !blocked, "{answer:?}");
            assert_eq!(
                doctor["microphone_blocked"].is_null(),
                !blocked,
                "{answer:?}"
            );
            if blocked {
                let problem = readiness.problem().unwrap();
                assert_eq!(problem.reason, "no_microphone_device");
                assert_eq!(problem.error.message, "No microphone was found");
            }
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn missing_microphone_advice_names_the_current_system() {
        let problem = MicrophoneReadiness {
            no_device: true,
            ..ready(MicrophoneAccess::NotApplicable)
        }
        .problem()
        .unwrap();
        assert_eq!(problem.reason, "no_microphone_device");
        assert_eq!(problem.error.message, "No microphone was found");
        let action = problem.error.suggested_action.unwrap();
        assert!(
            action.contains(if cfg!(target_os = "macos") {
                "System Settings > Sound > Input"
            } else {
                "sound settings"
            }),
            "{action}"
        );
    }

    fn ready(access: MicrophoneAccess) -> MicrophoneReadiness {
        MicrophoneReadiness {
            compiled: true,
            access,
            windows_switch: None,
            no_device: false,
        }
    }

    #[test]
    fn granted_or_platform_without_permission_is_admitted_without_asking() {
        for access in [MicrophoneAccess::Granted, MicrophoneAccess::NotApplicable] {
            let mut asked = false;
            assert!(admit_voiceover(ready(access), || asked = true).is_ok());
            assert!(!asked);
        }
    }

    #[test]
    fn unanswered_permission_shows_the_prompt_and_says_what_to_do() {
        let mut asked = false;
        let error = admit_voiceover(ready(MicrophoneAccess::NotDetermined), || asked = true)
            .expect_err("the first start waits for the answer");
        assert!(
            asked,
            "the start must make macOS show its microphone prompt"
        );
        assert!(error.message.contains("microphone permission is needed"));
        assert!(error
            .suggested_action
            .as_deref()
            .is_some_and(|action| action.contains("Allow")));
    }

    #[test]
    fn denied_permission_names_the_microphone_setting() {
        for access in [MicrophoneAccess::Denied, MicrophoneAccess::Restricted] {
            let error = admit_voiceover(ready(access), || panic!("never asks again"))
                .expect_err("denied microphone blocks voiceover");
            assert!(error.message.contains("microphone"));
            assert!(!error.message.contains("screen"));
            assert!(error
                .suggested_action
                .as_deref()
                .is_some_and(|action| action.contains("Privacy & Security > Microphone")));
        }
    }

    #[test]
    fn a_build_without_a_microphone_recorder_says_so() {
        let readiness = MicrophoneReadiness {
            compiled: false,
            access: MicrophoneAccess::NotApplicable,
            windows_switch: None,
            no_device: false,
        };
        let error = admit_voiceover(readiness, || {}).expect_err("no recorder");
        assert!(error.message.contains("microphone recorder is needed"));
    }

    fn windows(switch: Option<MicrophoneSwitch>, no_device: bool) -> MicrophoneReadiness {
        MicrophoneReadiness {
            compiled: true,
            access: if switch.is_some() {
                MicrophoneAccess::Denied
            } else {
                MicrophoneAccess::Granted
            },
            windows_switch: switch,
            no_device,
        }
    }

    #[test]
    fn a_windows_privacy_switch_names_the_setting_and_blocks_the_take() {
        let readiness = windows(Some(MicrophoneSwitch::DesktopApps), false);
        let error = admit_voiceover(readiness, || panic!("Windows never asks"))
            .expect_err("a blocked microphone refuses");
        assert_eq!(
            error.message,
            "Windows is blocking microphone access for desktop apps"
        );
        assert!(error.suggested_action.as_deref().is_some_and(|action| action
            .contains("Settings > Privacy & security > Microphone > Let desktop apps access your microphone")));
        let projected = projection(readiness, None);
        assert_eq!(projected["start_allowed"], false);
        assert_eq!(projected["microphone_access"], "denied");
        assert_eq!(
            projected["microphone_blocked"]["reason"],
            "windows_desktop_apps_microphone_off"
        );
    }

    #[test]
    fn each_windows_switch_has_its_own_reason() {
        for (switch, reason) in [
            (MicrophoneSwitch::Device, "windows_microphone_access_off"),
            (MicrophoneSwitch::Apps, "windows_apps_microphone_off"),
            (
                MicrophoneSwitch::DesktopApps,
                "windows_desktop_apps_microphone_off",
            ),
        ] {
            let projected = projection(windows(Some(switch), false), None);
            assert_eq!(projected["microphone_blocked"]["reason"], reason);
        }
    }

    #[test]
    fn no_microphone_device_is_told_apart_from_a_block() {
        let projected = projection(windows(None, true), None);
        assert_eq!(projected["start_allowed"], false);
        assert_eq!(
            projected["microphone_blocked"]["reason"],
            "no_microphone_device"
        );
        assert_eq!(
            projected["microphone_blocked"]["message"],
            "No microphone was found"
        );
        let both = projection(windows(Some(MicrophoneSwitch::Apps), true), None);
        assert_eq!(
            both["microphone_blocked"]["reason"],
            "windows_apps_microphone_off"
        );
    }

    #[test]
    fn a_windows_microphone_that_is_on_and_present_is_admitted() {
        let projected = projection(windows(None, false), None);
        assert_eq!(projected["start_allowed"], true);
        assert!(projected["microphone_blocked"].is_null());
        assert!(admit_voiceover(windows(None, false), || {}).is_ok());
    }
}

#[cfg(test)]
thread_local! {
    static TEST_READINESS: std::cell::Cell<Option<MicrophoneReadiness>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) struct TestReadiness(Option<MicrophoneReadiness>);

#[cfg(test)]
impl TestReadiness {
    pub(crate) fn install(readiness: MicrophoneReadiness) -> Self {
        Self(TEST_READINESS.with(|value| value.replace(Some(readiness))))
    }
}

#[cfg(test)]
impl Drop for TestReadiness {
    fn drop(&mut self) {
        TEST_READINESS.with(|value| value.set(self.0));
    }
}

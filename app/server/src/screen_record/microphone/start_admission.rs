//! Passive microphone admission before any recording owner or artifacts.

use super::readiness::MicrophoneReadiness;
use cut_core::CutError;
use record_capture::MicrophoneSource;

#[cfg(not(target_os = "linux"))]
pub(crate) fn source_for_recording_start(audio: bool) -> Result<MicrophoneSource, CutError> {
    admit_requested(audio, MicrophoneReadiness::current, super::source_for_start)
}

#[cfg(target_os = "linux")]
pub(crate) fn source_for_recording_start_linux(
    audio: bool,
    readiness: Option<MicrophoneReadiness>,
) -> Result<MicrophoneSource, CutError> {
    admit_requested(
        audio,
        || readiness.expect("microphone readiness is sampled before mic-enabled Start"),
        super::source_for_start,
    )
}

fn admit_requested(
    audio: bool,
    read: impl FnOnce() -> MicrophoneReadiness,
    resolve: impl FnOnce() -> Result<MicrophoneSource, CutError>,
) -> Result<MicrophoneSource, CutError> {
    if !audio {
        return Ok(MicrophoneSource::SystemDefault);
    }
    if let Some(mut problem) = read().problem() {
        // Keep the Doctor reason in the existing error shape and preserve its
        // exact message and suggested action.
        problem.error.cause = problem.reason.into();
        return Err(problem.error.with_requirement("microphone_permission"));
    }
    resolve()
}

#[cfg(test)]
mod tests {
    use super::*;
    use record_capture::{MicrophoneAccess, MicrophoneEndpointRef, MicrophoneSwitch};

    fn ready(access: MicrophoneAccess) -> MicrophoneReadiness {
        MicrophoneReadiness {
            compiled: true,
            access,
            windows_switch: None,
            no_device: false,
        }
    }

    #[test]
    fn screen_only_never_reads_readiness_or_resolves_a_microphone() {
        let source = admit_requested(
            false,
            || panic!("no microphone readiness read"),
            || panic!("no microphone source resolution"),
        )
        .unwrap();
        assert_eq!(source, MicrophoneSource::SystemDefault);
    }

    #[test]
    fn passive_problem_keeps_its_reason_message_and_action_without_resolving() {
        let readiness = MicrophoneReadiness {
            windows_switch: Some(MicrophoneSwitch::DesktopApps),
            ..ready(MicrophoneAccess::Granted)
        };
        let expected = readiness.problem().unwrap();
        let error = admit_requested(
            true,
            || readiness,
            || panic!("blocked source is not resolved"),
        )
        .unwrap_err();
        assert_eq!(error.code, expected.error.code);
        assert_eq!(error.cause, expected.reason);
        assert_eq!(error.message, expected.error.message);
        assert_eq!(error.suggested_action, expected.error.suggested_action);
    }

    #[test]
    fn no_passive_problem_preserves_old_source_and_errors_including_unanswered_permission() {
        record_capture::forbid_native_capture_access();
        for access in [
            MicrophoneAccess::Granted,
            MicrophoneAccess::NotApplicable,
            MicrophoneAccess::NotDetermined,
        ] {
            let expected_source = MicrophoneSource::Selected(MicrophoneEndpointRef::Windows {
                endpoint_id: "fixed-source-for-admission-test".into(),
            });
            let mut calls = 0;
            let source = admit_requested(
                true,
                || ready(access),
                || {
                    calls += 1;
                    Ok(expected_source.clone())
                },
            )
            .unwrap();
            assert_eq!(source, expected_source);
            assert_eq!(calls, 1);

            let old_error = CutError::new(
                "capture",
                "selected microphone is unavailable",
                "the privately saved native microphone endpoint is absent",
            )
            .with_suggested_action("choose another microphone or switch to System Default");
            let error =
                admit_requested(true, || ready(access), || Err(old_error.clone())).unwrap_err();
            assert_eq!(
                error, old_error,
                "source errors must not be rewritten for {access:?}"
            );
        }
    }
}

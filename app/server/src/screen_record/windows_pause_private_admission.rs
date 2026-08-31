//! Private admission for the narrow Windows pause-pilot capture owner.
//!
//! The pilot owns screen plus optional microphone/system-audio only. Passive
//! input, key content, windows, crops, and cursor facts remain unavailable
//! before any journal, native worker, or filesystem owner is created. This is
//! intentionally not a public request schema or product mode.

use super::monitor_start_admission::Target;
use super::windows_pause_session::{WindowsPauseSessionAdmission, WindowsPauseSessionError};
use record_capture::SelectedCaptureStreams;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WindowsPausePrivateRefusal {
    PassiveInput,
    KeyInput,
    Window,
    Region,
    Cursor,
    ScreenProfile,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn admit(
    target: Target,
    fps: f64,
    microphone: bool,
    system_audio: bool,
    passive_input: bool,
    key_input: bool,
    has_window: bool,
    has_region: bool,
    capture_cursor: bool,
) -> Result<WindowsPauseSessionAdmission, WindowsPausePrivateRefusal> {
    if passive_input {
        return Err(WindowsPausePrivateRefusal::PassiveInput);
    }
    if key_input {
        return Err(WindowsPausePrivateRefusal::KeyInput);
    }
    if has_window {
        return Err(WindowsPausePrivateRefusal::Window);
    }
    if has_region {
        return Err(WindowsPausePrivateRefusal::Region);
    }
    if capture_cursor {
        return Err(WindowsPausePrivateRefusal::Cursor);
    }
    WindowsPauseSessionAdmission::admit(
        target,
        SelectedCaptureStreams::new(microphone, system_audio, false, false),
        fps,
        super::recovery::CHECKPOINT_INTERVAL_MS,
    )
    .map_err(|error| match error {
        WindowsPauseSessionError::Admission => WindowsPausePrivateRefusal::ScreenProfile,
        // No lifecycle can exist during admission. Keep future error additions
        // fail-closed rather than treating an unexpected setup failure as ready.
        _ => WindowsPausePrivateRefusal::ScreenProfile,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact_monitor() -> String {
        format!("shellx-monitor-v1:windows:{}", "a".repeat(64))
    }

    fn target() -> Target {
        Target {
            legacy_index: Some(2),
            exact_id: Some(exact_monitor()),
        }
    }

    fn admit_screen_only() -> Result<WindowsPauseSessionAdmission, WindowsPausePrivateRefusal> {
        admit(
            target(),
            30.0,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
        )
    }

    #[test]
    fn admits_only_exact_screen_with_optional_sealed_audio() {
        assert!(admit_screen_only().is_ok());
        assert!(admit(
            target(),
            30.0,
            true,
            false,
            false,
            false,
            false,
            false,
            false,
        )
        .is_ok());
        assert!(admit(
            target(),
            30.0,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
        )
        .is_ok());
        assert!(admit(
            target(),
            30.0,
            true,
            true,
            false,
            false,
            false,
            false,
            false,
        )
        .is_ok());
        assert!(matches!(
            admit(
                target(),
                29.97,
                false,
                false,
                false,
                false,
                false,
                false,
                false
            ),
            Err(WindowsPausePrivateRefusal::ScreenProfile)
        ));
        assert!(matches!(
            admit(
                Target {
                    legacy_index: Some(2),
                    exact_id: None,
                },
                30.0,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
            ),
            Err(WindowsPausePrivateRefusal::ScreenProfile)
        ));
    }

    #[test]
    fn admitted_audio_selection_is_the_native_owner_selection() {
        let admission = admit(
            target(),
            30.0,
            true,
            true,
            false,
            false,
            false,
            false,
            false,
        )
        .unwrap();
        assert_eq!(
            admission.streams.streams(),
            &[
                record_recovery::RecordingStream::ScreenVideo,
                record_recovery::RecordingStream::MicrophoneAudio,
                record_recovery::RecordingStream::SystemAudio,
            ]
        );
        assert_eq!(
            admission.profile(),
            &record_capture::windows_pause_pilot::WindowsPausePilotProfile::admit(
                record_capture::windows_pause_pilot::WindowsPausePilotRequest::screen_with_audio(
                    exact_monitor(),
                    30.0,
                    true,
                    true,
                ),
            )
            .unwrap(),
        );
    }

    #[test]
    fn refuses_every_remaining_unsealed_normal_stream_before_native_start() {
        assert!(matches!(
            admit(
                target(),
                30.0,
                false,
                false,
                true,
                false,
                false,
                false,
                false
            ),
            Err(WindowsPausePrivateRefusal::PassiveInput)
        ));
        assert!(matches!(
            admit(
                target(),
                30.0,
                false,
                false,
                false,
                true,
                false,
                false,
                false
            ),
            Err(WindowsPausePrivateRefusal::KeyInput)
        ));
    }

    #[test]
    fn refuses_surfaces_the_pilot_cannot_reopen_or_crop() {
        assert!(matches!(
            admit(
                target(),
                30.0,
                false,
                false,
                false,
                false,
                true,
                false,
                false
            ),
            Err(WindowsPausePrivateRefusal::Window)
        ));
        assert!(matches!(
            admit(
                target(),
                30.0,
                false,
                false,
                false,
                false,
                false,
                true,
                false
            ),
            Err(WindowsPausePrivateRefusal::Region)
        ));
        assert!(matches!(
            admit(
                target(),
                30.0,
                false,
                false,
                false,
                false,
                false,
                false,
                true
            ),
            Err(WindowsPausePrivateRefusal::Cursor)
        ));
    }
}

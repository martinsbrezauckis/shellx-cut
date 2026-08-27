//! One bounded microphone-worker outcome projection shared by native backends.

use std::thread::JoinHandle;
use std::time::Duration;

use record_core::Result;

use crate::mic::CapturedMicrophone;
use crate::MicrophoneCaptureOutcome;

pub(super) fn finish(
    requested: bool,
    handle: Option<JoinHandle<Result<CapturedMicrophone>>>,
) -> (Option<String>, MicrophoneCaptureOutcome) {
    let Some(handle) = handle else {
        return if requested {
            (None, MicrophoneCaptureOutcome::MicrophoneLostNoTrack)
        } else {
            (None, MicrophoneCaptureOutcome::NotRequested)
        };
    };

    match crate::mic::join_bounded(handle, Duration::from_secs(2)) {
        Some(Ok(capture)) => match (capture.path, capture.microphone_lost) {
            (Some(path), false) => (Some(path), MicrophoneCaptureOutcome::Saved),
            (Some(path), true) => (
                Some(path),
                MicrophoneCaptureOutcome::MicrophoneLostSavedPrefix,
            ),
            (None, _) => (None, MicrophoneCaptureOutcome::MicrophoneLostNoTrack),
        },
        Some(Err(error)) => {
            eprintln!("warning: microphone lost, no track saved: {error}");
            (None, MicrophoneCaptureOutcome::MicrophoneLostNoTrack)
        }
        None => {
            eprintln!("warning: microphone lost, no track saved before bounded join");
            (None, MicrophoneCaptureOutcome::MicrophoneLostNoTrack)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    fn completed(capture: CapturedMicrophone) -> JoinHandle<Result<CapturedMicrophone>> {
        thread::spawn(move || Ok(capture))
    }

    #[test]
    fn absent_worker_distinguishes_not_requested_from_lost() {
        assert_eq!(
            finish(false, None).1,
            MicrophoneCaptureOutcome::NotRequested
        );
        assert_eq!(
            finish(true, None).1,
            MicrophoneCaptureOutcome::MicrophoneLostNoTrack
        );
    }

    #[test]
    fn saved_prefix_and_empty_capture_are_projected_exactly() {
        let (path, outcome) = finish(
            true,
            Some(completed(CapturedMicrophone {
                path: Some("mic.wav".into()),
                microphone_lost: true,
            })),
        );
        assert_eq!(path.as_deref(), Some("mic.wav"));
        assert_eq!(outcome, MicrophoneCaptureOutcome::MicrophoneLostSavedPrefix);

        assert_eq!(
            finish(
                true,
                Some(completed(CapturedMicrophone {
                    path: None,
                    microphone_lost: false,
                })),
            ),
            (None, MicrophoneCaptureOutcome::MicrophoneLostNoTrack)
        );
    }
}

use super::{ActiveTake, Phase, VoiceoverCaptureControl, VoiceoverMaterialization};
use cut_core::{error_codes, CutError};
use record_capture::VoiceoverCaptureOutcome;

pub(super) fn sealed_materialization<C: VoiceoverCaptureControl>(
    active: &mut ActiveTake<C>,
    current_revision: &str,
) -> Result<VoiceoverMaterialization, CutError> {
    if active.take.revision != current_revision {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "voiceover artifact became stale before placement",
            format!(
                "take accepted '{}' but the current project revision is '{current_revision}'",
                active.take.revision
            ),
        )
        .with_suggested_action("discard this take and record again from the current timeline"));
    }
    let Phase::Finished(outcome) = &active.phase else {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "voiceover take is not sealed yet",
            "wait for native microphone finalization before importing the WAV",
        ));
    };
    match outcome {
        VoiceoverCaptureOutcome::Saved(artifact) => Ok(VoiceoverMaterialization {
            take: active.take.clone(),
            artifact: artifact.clone(),
            device_lost_after_prefix: false,
        }),
        VoiceoverCaptureOutcome::DeviceLostSavedPrefix(artifact) => Ok(VoiceoverMaterialization {
            take: active.take.clone(),
            artifact: artifact.clone(),
            device_lost_after_prefix: true,
        }),
        other => Err(CutError::new(
            error_codes::CONFLICT,
            "voiceover produced no placeable WAV",
            format!("terminal outcome: {other:?}"),
        )),
    }
}

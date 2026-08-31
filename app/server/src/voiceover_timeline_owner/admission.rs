use cut_core::{error_codes, CutError, Project, TrackKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VoiceoverCue {
    Playhead { at_ms: u64 },
    InOut { in_ms: u64, out_ms: u64 },
}

impl VoiceoverCue {
    pub(super) fn start_ms(&self) -> u64 {
        match self {
            Self::Playhead { at_ms } => *at_ms,
            Self::InOut { in_ms, .. } => *in_ms,
        }
    }

    pub(super) fn out_ms(&self) -> Option<u64> {
        match self {
            Self::Playhead { .. } => None,
            Self::InOut { out_ms, .. } => Some(*out_ms),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VoiceoverTimelineRequest {
    pub(crate) request_id: String,
    pub(crate) expected_revision: String,
    pub(crate) audio_track: String,
    pub(crate) cue: VoiceoverCue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AcceptedVoiceoverTimeline {
    pub(crate) request_id: String,
    pub(crate) revision: String,
    pub(crate) audio_track: String,
    pub(crate) start_ms: u64,
    pub(crate) out_ms: Option<u64>,
}

/// Validate the semantic target before any microphone device or WAV staging
/// file can be opened. A future public handler must call this under the same
/// project/revision admission lock as its native-start action.
pub(crate) fn admit(
    project: &Project,
    current_revision: &str,
    request: VoiceoverTimelineRequest,
) -> Result<AcceptedVoiceoverTimeline, CutError> {
    if request.request_id.is_empty() || request.request_id.len() > 128 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "voiceover request_id must contain 1 to 128 characters",
            "the request identity is required for a future durable take commit",
        ));
    }
    if request.expected_revision != current_revision {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "voiceover target is stale",
            format!(
                "the request accepted '{}' but the current project revision is '{current_revision}'",
                request.expected_revision
            ),
        )
        .with_suggested_action("refresh the Timeline and start a new voiceover take"));
    }
    let track = project.track(&request.audio_track).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("audio track '{}' no longer exists", request.audio_track),
            "voiceover must target one current audio track",
        )
    })?;
    if track.kind != TrackKind::Audio {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("track '{}' is not an audio track", request.audio_track),
            "voiceover clips can only be placed on an audio track",
        ));
    }
    if track.locked {
        return Err(CutError::new(
            error_codes::GUARDRAIL,
            format!("track '{}' is locked", request.audio_track),
            "the selected audio track rejects timeline edits",
        )
        .with_suggested_action("unlock the audio track or choose a different track"));
    }
    if let VoiceoverCue::InOut { in_ms, out_ms } = &request.cue {
        if out_ms <= in_ms {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "voiceover Out must be after In",
                format!("received In {in_ms}ms and Out {out_ms}ms"),
            ));
        }
    }
    Ok(AcceptedVoiceoverTimeline {
        request_id: request.request_id,
        revision: request.expected_revision,
        audio_track: request.audio_track,
        start_ms: request.cue.start_ms(),
        out_ms: request.cue.out_ms(),
    })
}

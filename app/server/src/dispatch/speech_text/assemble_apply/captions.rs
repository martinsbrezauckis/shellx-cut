//! Caption-track construction for materialized reviewed short plans.

use super::super::captions::caption_cues_from_timeline_words;
use super::super::*;
use super::binding::ASSEMBLE_CAPTION_TRACK_ID;
use super::planning::locked_track_error;

pub(super) fn append_short_captions(
    project: &cut_core::Project,
    words: Vec<(u64, u64, String)>,
) -> Result<(Vec<cut_core::Track>, Vec<String>), CutError> {
    let mut tracks = project.tracks.clone();
    if !tracks
        .iter()
        .any(|track| track.id == ASSEMBLE_CAPTION_TRACK_ID)
    {
        tracks.push(cut_core::Track {
            id: ASSEMBLE_CAPTION_TRACK_ID.into(),
            kind: cut_core::TrackKind::Caption,
            clips: vec![],
            gain_db: 0.0,
            gain_windows: vec![],
            blend_mode: None,
            visible: true,
            locked: false,
            muted: false,
            solo: false,
            pan: 0.0,
        });
    }
    let track = tracks
        .iter_mut()
        .find(|track| track.id == ASSEMBLE_CAPTION_TRACK_ID)
        .expect("just ensured assemble caption track");
    if track.kind != cut_core::TrackKind::Caption {
        return Err(CutError::new(
            error_codes::CONFLICT,
            format!("track '{ASSEMBLE_CAPTION_TRACK_ID}' is not a caption track"),
            "rename or remove the conflicting track before applying short captions",
        ));
    }
    if track.locked {
        return Err(locked_track_error(ASSEMBLE_CAPTION_TRACK_ID));
    }
    let first_id = track
        .clips
        .iter()
        .filter_map(|clip| clip.id()?.strip_prefix("asmcap_")?.parse::<usize>().ok())
        .max()
        .unwrap_or(0);
    let cues = caption_cues_from_timeline_words(words, None, "asmcap_", first_id);
    if cues.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "the reviewed short ranges contain no captionable words",
            "review a short plan whose transcript has timed words before applying it",
        ));
    }
    let ids = cues.iter().map(|cue| cue.id.clone()).collect();
    track
        .clips
        .extend(cues.into_iter().map(cut_core::Clip::Caption));
    track.clips.sort_by_key(|clip| match clip {
        cut_core::Clip::Caption(cue) => cue.range_ms[0],
        _ => 0,
    });
    Ok((tracks, ids))
}

pub(super) fn added_clip_id(effects: &[OpEffect], track: &str) -> Result<String, CutError> {
    effects
        .iter()
        .find(|effect| {
            effect.track.as_deref() == Some(track) && effect.detail.get("added_clip").is_some()
        })
        .and_then(|effect| effect.detail.get("added_clip"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            CutError::new(
                error_codes::JOB_FAILED,
                "Assemble could not prove an inserted clip id",
                format!("the core insert result on track '{track}' had no added_clip effect"),
            )
        })
}

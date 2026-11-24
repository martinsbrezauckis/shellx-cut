//! Keep saved tracks renderable under the absolute per-clip safety ceiling and command budget.
use cut_core::{
    error_codes,
    keyframe_limit::{MAX_KEYFRAME_POINTS, MAX_KEYFRAME_TOTAL_POINTS},
    CutError, Edl, EdlSegment, KfParam, Project, TrackKind,
};

use super::keyframe_cmd::{
    audio_timestamps_budget_error, base_opacity_body_bytes, command_budget_error, decimal_digits,
    estimate_audio_cmd_usage, estimate_video_cmd_bytes, overlay_opacity_body_bytes,
    overlay_pos_body_bytes, scale_crop_body_bytes, shaped_scale_body_bytes,
    MAX_AUDIO_CMD_TIMESTAMPS, MAX_RENDER_CMD_BYTES,
};
use super::tracked_mask::moving_region_body_bytes;

#[derive(Default)]
struct BudgetAcc {
    total_bytes: u64,
    worst_bytes: u64,
    worst_clip: Option<String>,
    max_audio_timestamps: u64,
    worst_audio_bytes: u64,
    worst_audio_clip: Option<String>,
}

impl BudgetAcc {
    fn add_video(&mut self, bytes: u64, clip_id: Option<&str>) {
        if bytes == 0 {
            return;
        }
        self.total_bytes = self.total_bytes.saturating_add(bytes);
        if bytes >= self.worst_bytes {
            self.worst_bytes = bytes;
            if let Some(id) = clip_id {
                self.worst_clip = Some(id.to_string());
            }
        }
    }

    fn add_audio(&mut self, bytes: u64, timestamps: u64, clip_id: Option<&str>) {
        self.add_video(bytes, clip_id);
        if timestamps > self.max_audio_timestamps {
            self.max_audio_timestamps = timestamps;
            self.worst_audio_bytes = bytes;
            if let Some(id) = clip_id {
                self.worst_audio_clip = Some(id.to_string());
            }
        }
    }

    fn finish(self) -> Result<(), CutError> {
        if self.total_bytes > MAX_RENDER_CMD_BYTES {
            return Err(command_budget_error(
                self.total_bytes,
                self.worst_clip.as_deref(),
            ));
        }
        if self.max_audio_timestamps > MAX_AUDIO_CMD_TIMESTAMPS {
            return Err(audio_timestamps_budget_error(
                self.worst_audio_bytes.max(self.total_bytes),
                self.max_audio_timestamps,
                self.worst_audio_clip.as_deref(),
            ));
        }
        Ok(())
    }
}

pub(super) fn require_keyframe_counts_bounded(
    project: &Project,
    edl: &Edl,
) -> Result<(), CutError> {
    cut_core::redaction_limit::check_project(project)?;
    for track in &project.tracks {
        for clip in &track.clips {
            if let cut_core::Clip::Media(media) = clip {
                check_total(&media.id, media.keyframes.iter().map(|k| k.points.len()))?;
            }
        }
    }
    for segment in &edl.segments {
        let clip = segment.clip_id.as_deref().unwrap_or("unknown");
        check_total(clip, segment.keyframes.iter().map(|k| k.points.len()))?;
        for track in &segment.keyframes {
            if track.lead_in.as_ref().is_some_and(|lead| {
                lead.t_ms >= 0
                    || !lead.value.is_finite()
                    || track.points.len() < 2
                    || track.points.first().is_none_or(|p| p.t_ms != 0)
            }) {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "invalid keyframe curve lead-in",
                    "a clipped easing needs a finite preceding point before local zero",
                ));
            }
        }
    }
    Ok(())
}

/// Check per-clip keyframe limits and total generated `sendcmd`/`asendcmd` byte
/// and audio timestamp budgets before allocating strings or starting jobs.
pub fn check_command_budget(project: &Project, edl: &Edl) -> Result<(), CutError> {
    check_command_budget_for(project, edl, true, true, true)
}

/// Check audio keyframe limits and generated `asendcmd` byte/timestamp budgets
/// for an audio-only render (`export.audio`).
pub fn check_audio_command_budget(project: &Project, edl: &Edl) -> Result<(), CutError> {
    check_command_budget_for(project, edl, false, true, false)
}

pub(super) fn require_bounded_keyframes(project: &Project, edl: &Edl) -> Result<(), CutError> {
    check_command_budget_for(project, edl, true, true, true)
}

pub(super) fn check_command_budget_for(
    project: &Project,
    edl: &Edl,
    with_video: bool,
    with_audio: bool,
    check_nests: bool,
) -> Result<(), CutError> {
    require_keyframe_counts_bounded(project, edl)?;

    let mut acc = BudgetAcc::default();
    if with_video {
        accumulate_video_budget(project, edl, &mut acc);
    }
    if with_audio {
        accumulate_audio_budget(project, edl, &mut acc);
    }
    acc.finish()?;

    if check_nests {
        for nest in &project.nests {
            let mut sub = Project::new(
                &format!("{}::{}", project.name, nest.id),
                project.settings.clone(),
            );
            sub.tracks = nest.tracks.clone();
            let sub_edl = cut_core::edl_from_project(&sub);
            check_command_budget_for(&sub, &sub_edl, with_video, with_audio, false)?;
        }
    }
    Ok(())
}

fn has_curve(seg: &EdlSegment, param: KfParam, min_points: usize) -> bool {
    super::kf_points(&seg.keyframes, param).is_some_and(|(pts, _)| pts.len() >= min_points)
}

fn accumulate_video_budget(project: &Project, edl: &Edl, acc: &mut BudgetAcc) {
    let fps = project.settings.fps;
    let keep_hdr = super::hdr::GraphColor::resolve_with(
        project,
        edl,
        super::RenderOptions::default().color,
        None,
    )
    .keep_hdr();
    let tracks = super::planned_video_tracks(project);

    if let Some(base) = tracks.first().filter(|t| t.visible) {
        for (n, seg) in edl.track_segments(base.id).enumerate() {
            if seg.asset.is_none() || seg.src_in_ms.is_none() || seg.src_out_ms.is_none() {
                continue;
            }
            let dur_ms = seg.timeline_out_ms.saturating_sub(seg.timeline_in_ms);
            let label_len = 1u64.saturating_add(decimal_digits(n as u64));
            let mut seg_bytes = 0u64;
            if super::keyframe_window::long(seg, KfParam::Scale)
                && has_curve(seg, KfParam::Scale, 2)
            {
                seg_bytes = seg_bytes.saturating_add(estimate_video_cmd_bytes(
                    fps,
                    dur_ms,
                    scale_crop_body_bytes(label_len.saturating_add(1)),
                ));
            }
            if has_curve(seg, KfParam::Opacity, 2) {
                seg_bytes = seg_bytes.saturating_add(estimate_video_cmd_bytes(
                    fps,
                    dur_ms,
                    base_opacity_body_bytes(label_len),
                ));
            }
            if let Some(mask) = seg.mask.as_ref().filter(|m| super::mask_uses_geq(m)) {
                let body = moving_region_body_bytes(mask, label_len);
                seg_bytes = seg_bytes.saturating_add(estimate_video_cmd_bytes(fps, dur_ms, body));
            }
            acc.add_video(seg_bytes, seg.clip_id.as_deref());
        }
    }

    for (ti, track) in tracks.iter().enumerate().skip(1) {
        for (n, seg) in edl.track_segments(track.id).enumerate() {
            if seg.asset.is_none() || seg.src_in_ms.is_none() || seg.src_out_ms.is_none() {
                continue;
            }
            let dur_ms = seg.timeline_out_ms.saturating_sub(seg.timeline_in_ms);
            let uniq_len = 1u64
                .saturating_add(decimal_digits(ti as u64))
                .saturating_add(1)
                .saturating_add(decimal_digits(
                    (n.saturating_mul(2).saturating_add(1)) as u64,
                ));
            let mut seg_bytes = 0u64;
            if has_curve(seg, KfParam::Opacity, 2) {
                seg_bytes = seg_bytes.saturating_add(estimate_video_cmd_bytes(
                    fps,
                    dur_ms,
                    overlay_opacity_body_bytes(uniq_len, keep_hdr),
                ));
            }
            if super::keyframe_window::long(seg, KfParam::Scale)
                && has_curve(seg, KfParam::Scale, 2)
            {
                let canvas_uniq = uniq_len.saturating_add(2);
                let body =
                    scale_crop_body_bytes(uniq_len).max(shaped_scale_body_bytes(canvas_uniq));
                seg_bytes = seg_bytes.saturating_add(estimate_video_cmd_bytes(fps, dur_ms, body));
            }
            let x_long = has_curve(seg, KfParam::PosX, MAX_KEYFRAME_POINTS + 1);
            let y_long = has_curve(seg, KfParam::PosY, MAX_KEYFRAME_POINTS + 1);
            if x_long || y_long {
                let canvas_uniq = uniq_len.saturating_add(2);
                let body = overlay_pos_body_bytes(canvas_uniq, x_long, y_long);
                seg_bytes = seg_bytes.saturating_add(estimate_video_cmd_bytes(fps, dur_ms, body));
            }
            acc.add_video(seg_bytes, seg.clip_id.as_deref());
        }
    }
}

fn accumulate_audio_budget(project: &Project, edl: &Edl, acc: &mut BudgetAcc) {
    for (t, track) in project
        .tracks
        .iter()
        .filter(|tr| {
            tr.kind == TrackKind::Audio && !tr.clips.is_empty() && project.audio_track_audible(tr)
        })
        .enumerate()
    {
        for (n, seg) in edl.track_segments(&track.id).enumerate() {
            if seg.asset.is_none() || seg.src_in_ms.is_none() || seg.src_out_ms.is_none() {
                continue;
            }
            if !super::keyframe_window::long(seg, KfParam::Volume) {
                continue;
            }
            let Some((pts, _)) = super::kf_points(&seg.keyframes, KfParam::Volume) else {
                continue;
            };
            let dur_ms = seg.timeline_out_ms.saturating_sub(seg.timeline_in_ms);
            let uniq_len = 1u64
                .saturating_add(decimal_digits(t as u64))
                .saturating_add(1)
                .saturating_add(decimal_digits(n as u64));
            let (bytes, timestamps) = estimate_audio_cmd_usage(pts.len(), dur_ms, uniq_len);
            acc.add_audio(bytes, timestamps, seg.clip_id.as_deref());
        }
    }
}

fn check_total(clip: &str, points: impl Iterator<Item = usize>) -> Result<(), CutError> {
    let count = points.fold(0usize, usize::saturating_add);
    if count > MAX_KEYFRAME_TOTAL_POINTS {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "clip '{clip}' has {count} keyframe points in total; the limit is {MAX_KEYFRAME_TOTAL_POINTS}"
            ),
            "reduce this clip's keyframe points and try again",
        )
        .with_clip(clip)
        .with_suggested_action("reduce this clip's keyframe points and try again"));
    }
    Ok(())
}

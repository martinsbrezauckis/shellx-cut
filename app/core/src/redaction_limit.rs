//! Absolute admission bounds for a clip's redaction expression.
use crate::{
    error::{codes, CutError},
    keyframe_limit::{MAX_REDACTION_MOTION_POINTS, MAX_REDACTION_REGIONS},
    types::{Clip, ClipMask, Project},
};

pub fn check_mask(clip: &str, mask: &ClipMask) -> Result<(), CutError> {
    let regions = mask.regions.len().saturating_add(1);
    if regions > MAX_REDACTION_REGIONS {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!(
                "clip '{clip}' redaction has {regions} regions; the limit is {MAX_REDACTION_REGIONS}"
            ),
            "remove excess redaction regions and try again",
        )
        .with_clip(clip)
        .with_suggested_action("remove excess redaction regions and try again"));
    }
    let points = mask
        .regions
        .iter()
        .fold(mask.track.as_ref().map_or(0, Vec::len), |total, region| {
            total.saturating_add(region.track.as_ref().map_or(0, Vec::len))
        });
    if points > MAX_REDACTION_MOTION_POINTS {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!(
                "clip '{clip}' redaction has {points} motion points in total; the limit is {MAX_REDACTION_MOTION_POINTS}"
            ),
            "reduce this clip's redaction motion points and try again",
        )
        .with_clip(clip)
        .with_suggested_action("reduce this clip's redaction motion points and try again"));
    }
    Ok(())
}

/// The `edit.redact` route alone uses this wrapper; `edit.add_mask` keeps its contract.
pub fn add_mask(
    project: &mut Project,
    clip: &str,
    mask: Option<ClipMask>,
) -> Result<Vec<crate::OpEffect>, CutError> {
    if let Some(mask) = &mask {
        check_mask(clip, mask)?;
    }
    crate::edit::add_mask(project, clip, mask)
}

/// Historical redaction records may exceed today's edit ceilings. Their saved
/// effect contains the resolved mask; replay it unchanged only during journal load.
pub(crate) fn apply_saved_over_cap(
    project: &mut Project,
    op: &crate::OpRecord,
) -> Result<bool, CutError> {
    if op.verb != "edit.redact" {
        return Ok(false);
    }
    let Some(mask_value) = op
        .effects
        .iter()
        .find_map(|effect| effect.detail.get("mask"))
    else {
        return Ok(false);
    };
    if mask_value.is_null() {
        return Ok(false);
    }
    let regions = mask_value["regions"].as_array();
    let region_count = regions.map_or(0, Vec::len).saturating_add(1);
    let point_count = regions
        .map_or(0, |items| {
            items.iter().fold(0usize, |total, item| {
                total.saturating_add(item["track"].as_array().map_or(0, Vec::len))
            })
        })
        .saturating_add(mask_value["track"].as_array().map_or(0, Vec::len));
    if region_count <= MAX_REDACTION_REGIONS && point_count <= MAX_REDACTION_MOTION_POINTS {
        return Ok(false);
    }
    let mask: ClipMask = serde_json::from_value(mask_value.clone())?;
    let clip = op.args["clip"].as_str().ok_or_else(|| {
        CutError::new(
            codes::INVALID_ARGS,
            "saved redaction has no clip",
            "the operation journal needs a clip id",
        )
    })?;
    crate::edit::add_mask(project, clip, Some(mask))?;
    Ok(true)
}

/// Read-only guard: a saved project stays loadable, then render rejects it.
pub fn check_project(project: &Project) -> Result<(), CutError> {
    for track in &project.tracks {
        for clip in &track.clips {
            if let Clip::Media(media) = clip {
                if let Some(mask) = &media.mask {
                    check_mask(&media.id, mask)?;
                }
            }
        }
    }
    Ok(())
}

//! Bounded title/shape metadata projection for split overlay descendants.

use super::*;

const MAX_OVERLAY_METADATA_OPERATION_BYTES: usize = 16 * 1024;
const MAX_OVERLAY_METADATA_ENTRY_BYTES: usize = 64 * 1024;
const MAX_OVERLAY_METADATA_PROJECTION_BYTES: usize = 512 * 1024;
const MAX_OVERLAY_METADATA_PROJECTION_ENTRIES: usize = 512;

#[derive(Clone, Copy)]
struct OverlayMetadataEntry {
    // Conservative upper bound: the originating op plus every folded update.
    // This lets validation stay allocation-free even for malicious JSON values.
    projected_bytes: usize,
}

#[derive(Default)]
struct OverlayMetadataBudget {
    entries: usize,
    projected_bytes: usize,
}

struct CappedJsonByteCounter {
    bytes: usize,
    limit: usize,
    exceeded: bool,
}

impl std::io::Write for CappedJsonByteCounter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let available = self.limit.saturating_sub(self.bytes);
        if buffer.len() > available {
            self.exceeded = true;
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "JSON byte budget exceeded",
            ));
        }
        self.bytes += buffer.len();
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn overlay_metadata_error(message: impl Into<String>, detail: impl Into<String>) -> CutError {
    CutError::new(error_codes::INVALID_ARGS, message, detail).with_suggested_action(
        "reduce the imported title/shape metadata or split count, then reopen the project",
    )
}

fn bounded_overlay_metadata_bytes(
    value: &Value,
    limit: usize,
    operation: &str,
    op_id: &str,
) -> Result<usize, CutError> {
    let mut counter = CappedJsonByteCounter {
        bytes: 0,
        limit,
        exceeded: false,
    };
    match serde_json::to_writer(&mut counter, value) {
        Ok(()) => Ok(counter.bytes),
        Err(_) if counter.exceeded => Err(overlay_metadata_error(
            "overlay metadata exceeds the supported size",
            format!("{operation} operation '{op_id}' exceeds the {limit}-byte metadata limit"),
        )),
        Err(error) => Err(overlay_metadata_error(
            "overlay metadata could not be encoded",
            format!("{operation} operation '{op_id}' is not serializable: {error}"),
        )),
    }
}

pub(in crate::dispatch) fn validate_overlay_metadata_args(
    args: &Value,
    operation: &str,
) -> Result<(), CutError> {
    bounded_overlay_metadata_bytes(
        args,
        MAX_OVERLAY_METADATA_OPERATION_BYTES,
        operation,
        "new request",
    )
    .map(|_| ())
}

fn replace_overlay_metadata(
    entries: &mut std::collections::BTreeMap<String, OverlayMetadataEntry>,
    budget: &mut OverlayMetadataBudget,
    clip: &str,
    next: OverlayMetadataEntry,
    operation: &str,
    op_id: &str,
) -> Result<(), CutError> {
    if next.projected_bytes > MAX_OVERLAY_METADATA_ENTRY_BYTES {
        return Err(overlay_metadata_error(
            "overlay metadata descendant exceeds the supported size",
            format!(
                "{operation} operation '{op_id}' would project {}/{} bytes onto clip '{clip}'",
                next.projected_bytes, MAX_OVERLAY_METADATA_ENTRY_BYTES
            ),
        ));
    }
    let previous = entries.get(clip).copied();
    let projected_entries = budget.entries + usize::from(previous.is_none());
    let projected_bytes = budget
        .projected_bytes
        .saturating_sub(previous.map_or(0, |entry| entry.projected_bytes))
        .saturating_add(next.projected_bytes);
    if projected_entries > MAX_OVERLAY_METADATA_PROJECTION_ENTRIES
        || projected_bytes > MAX_OVERLAY_METADATA_PROJECTION_BYTES
    {
        return Err(overlay_metadata_error(
            "overlay metadata projection exceeds the supported budget",
            format!(
                "{operation} operation '{op_id}' would project {projected_entries} descendants and {projected_bytes}/{} bytes (limits: {} descendants and {} bytes)",
                MAX_OVERLAY_METADATA_PROJECTION_BYTES,
                MAX_OVERLAY_METADATA_PROJECTION_ENTRIES,
                MAX_OVERLAY_METADATA_PROJECTION_BYTES,
            ),
        ));
    }
    entries.insert(clip.to_string(), next);
    budget.entries = projected_entries;
    budget.projected_bytes = projected_bytes;
    Ok(())
}

fn update_overlay_metadata(
    entries: &mut std::collections::BTreeMap<String, OverlayMetadataEntry>,
    budget: &mut OverlayMetadataBudget,
    op: &OpRecord,
    operation: &str,
) -> Result<(), CutError> {
    let Some(clip) = op.args.get("clip").and_then(Value::as_str) else {
        return Ok(());
    };
    let Some(current) = entries.get(clip).copied() else {
        return Ok(());
    };
    let update_bytes = bounded_overlay_metadata_bytes(
        &op.args,
        MAX_OVERLAY_METADATA_OPERATION_BYTES,
        operation,
        &op.op_id,
    )?;
    replace_overlay_metadata(
        entries,
        budget,
        clip,
        OverlayMetadataEntry {
            projected_bytes: current.projected_bytes.saturating_add(update_bytes),
        },
        operation,
        &op.op_id,
    )
}

fn split_effect_ids_from_effects(effects: &[OpEffect]) -> Option<(&str, &str)> {
    effects.iter().find_map(|effect| {
        Some((
            effect.detail.get("left")?.as_str()?,
            effect.detail.get("right")?.as_str()?,
        ))
    })
}

fn added_clip_id(op: &OpRecord) -> Option<&str> {
    op.effects
        .iter()
        .find_map(|effect| effect.detail.get("added_clip")?.as_str())
}

/// The core `edit.split` effect names the retained left clip and the freshly
/// allocated right half. Overlay specs live in the operation log rather than
/// on `MediaClip`, so title/shape reconstruction must follow that identity
/// edge just as the timeline does.
pub(in crate::dispatch) fn split_effect_ids(op: &OpRecord) -> Option<(&str, &str)> {
    (op.verb == "edit.split")
        .then(|| split_effect_ids_from_effects(&op.effects))
        .flatten()
}

/// Validate the small, declarative metadata projection that backs editable
/// title and shape overlays. The materialized timeline is bounded separately;
/// this closes the op-log-only path where one imported spec could otherwise be
/// cloned once per split and then serialized again by `project.state` or a
/// title/shape recovery edit.
///
/// The topology pass deliberately replays `edit.split` without recorded ids,
/// then compares the generated left/right pair with the operation effect. That
/// makes provenance an actual timeline edge rather than arbitrary metadata in
/// an imported log.
pub(in crate::dispatch) fn validate_split_metadata_projection(
    ops: &[OpRecord],
) -> Result<(), CutError> {
    use std::collections::BTreeMap;

    let mut title_entries: BTreeMap<String, OverlayMetadataEntry> = BTreeMap::new();
    let mut shape_entries: BTreeMap<String, OverlayMetadataEntry> = BTreeMap::new();
    let mut budget = OverlayMetadataBudget::default();
    let mut topology = cut_core::Project::new("", cut_core::ProjectSettings::default());

    for (index, op) in ops.iter().enumerate() {
        if op.verb == "edit.split" {
            let computed = cut_core::apply_edit_verb(&mut topology, &op.verb, &op.args)
                .map_err(|error| {
                    overlay_metadata_error(
                        "overlay split metadata cannot be replayed",
                        format!(
                            "edit.split operation '{}' is invalid against the prior timeline: {error}",
                            op.op_id
                        ),
                    )
                })?;
            let recorded = split_effect_ids(op);
            let expected = split_effect_ids_from_effects(&computed);
            if recorded != expected {
                return Err(overlay_metadata_error(
                    "overlay split metadata does not match replayed topology",
                    format!(
                        "edit.split operation '{}' records {:?}, but replay produced {:?}",
                        op.op_id, recorded, expected
                    ),
                ));
            }
            let (left, right) = recorded.ok_or_else(|| {
                overlay_metadata_error(
                    "overlay split metadata is missing replay provenance",
                    format!(
                        "edit.split operation '{}' records no left/right effect",
                        op.op_id
                    ),
                )
            })?;
            for entries in [&mut title_entries, &mut shape_entries] {
                if let Some(current) = entries.get(left).copied() {
                    replace_overlay_metadata(
                        entries,
                        &mut budget,
                        right,
                        current,
                        "edit.split",
                        &op.op_id,
                    )?;
                }
            }
        } else {
            cut_core::apply_record(&mut topology, op, &ops[..index]).map_err(|error| {
                overlay_metadata_error(
                    "overlay metadata cannot be replayed",
                    format!("operation '{}' is invalid during replay: {error}", op.op_id),
                )
            })?;
        }
        topology.sync_active_sequence();

        match op.verb.as_str() {
            "title.add" => {
                if let Some(clip) = added_clip_id(op) {
                    let bytes = bounded_overlay_metadata_bytes(
                        &op.args,
                        MAX_OVERLAY_METADATA_OPERATION_BYTES,
                        "title.add",
                        &op.op_id,
                    )?;
                    replace_overlay_metadata(
                        &mut title_entries,
                        &mut budget,
                        clip,
                        OverlayMetadataEntry {
                            projected_bytes: bytes,
                        },
                        "title.add",
                        &op.op_id,
                    )?;
                }
            }
            "edit.add_shape" => {
                if let Some(clip) = added_clip_id(op) {
                    let bytes = bounded_overlay_metadata_bytes(
                        &op.args,
                        MAX_OVERLAY_METADATA_OPERATION_BYTES,
                        "edit.add_shape",
                        &op.op_id,
                    )?;
                    replace_overlay_metadata(
                        &mut shape_entries,
                        &mut budget,
                        clip,
                        OverlayMetadataEntry {
                            projected_bytes: bytes,
                        },
                        "edit.add_shape",
                        &op.op_id,
                    )?;
                }
            }
            "title.update" => {
                update_overlay_metadata(&mut title_entries, &mut budget, op, "title.update")?;
            }
            "shape.update" => {
                update_overlay_metadata(&mut shape_entries, &mut budget, op, "shape.update")?;
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

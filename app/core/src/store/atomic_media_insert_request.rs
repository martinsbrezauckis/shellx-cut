//! Request validation and optional track topology for one atomic media insert.

use super::*;
use crate::types::TrackKind;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AtomicMediaInsertTrack {
    pub id: String,
    pub kind: TrackKind,
    pub visible: bool,
}

#[derive(Debug, Clone)]
pub struct AtomicMediaInsert {
    pub idempotency_key: String,
    pub asset: Asset,
    pub insert: Value,
    pub binding: Value,
    /// When present, this exact topology is created in the same committed
    /// operation as the asset and clip. It is intentionally create-only: an
    /// existing id is a conflicting owner, never an orphan to adopt.
    pub track: Option<AtomicMediaInsertTrack>,
}

pub(super) fn validate_request(request: &AtomicMediaInsert, actor: &Actor) -> Result<(), CutError> {
    if !is_lower_hex_sha256(&request.idempotency_key) {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "atomic media insert idempotency key is invalid",
            "expected the lowercase SHA-256 of the exact attested transaction",
        ));
    }
    if actor.request.is_none() {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "atomic media insert requires durable request metadata",
            "bind the transaction to request_id, fingerprint, and expected revision",
        ));
    }
    if !request.binding.is_object() {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "atomic media insert binding must be an object",
            "record immutable source and target facts with the transaction",
        ));
    }
    if !request.insert.is_object() || request.insert.get("asset").is_some() {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "atomic media insert has an invalid insert payload",
            "pass canonical edit.insert arguments without a caller-selected asset id",
        ));
    }
    if let Some(track) = &request.track {
        validate_track(track)?;
    }
    Ok(())
}

pub(super) fn apply_track(
    project: &mut Project,
    track: Option<&AtomicMediaInsertTrack>,
    pinned: Option<&mut crate::rebase::PinnedIds>,
) -> Result<Vec<OpEffect>, CutError> {
    let Some(track) = track else {
        return Ok(Vec::new());
    };
    validate_track(track)?;
    let args = json!({"kind": track.kind, "id": track.id});
    let mut effects = match pinned {
        Some(pinned) => apply_edit_verb_pinned(project, "edit.add_track", &args, Some(pinned))?,
        None => apply_edit_verb(project, "edit.add_track", &args)?,
    };
    if !track.visible {
        effects.extend(apply_edit_verb(
            project,
            "edit.track_visible",
            &json!({"track": track.id, "on": false}),
        )?);
    }
    Ok(effects)
}

fn validate_track(track: &AtomicMediaInsertTrack) -> Result<(), CutError> {
    if track.id.trim().is_empty() || track.kind == TrackKind::Caption {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "atomic media insert track topology is invalid",
            "use a non-empty video or audio track id",
        ));
    }
    Ok(())
}

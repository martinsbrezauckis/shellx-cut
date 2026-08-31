//! Atomic asset registration plus canonical `edit.insert` replay.
use super::atomic_media_insert_replay::track_topology;
use super::atomic_media_insert_request::{apply_track, validate_request, AtomicMediaInsert};
use super::*;
const DETAIL_KEY: &str = "atomic_media_insert";

#[derive(Debug, Clone)]
pub struct AtomicMediaInsertResult {
    pub asset_id: String,
    pub clip_id: String,
    pub op: OpRecord,
    pub already_applied: bool,
}

impl ProjectStore {
    /// Atomically stage one asset and canonical `edit.insert` operation.
    pub fn apply_atomic_media_insert(
        &mut self,
        request: AtomicMediaInsert,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<AtomicMediaInsertResult, CutError> {
        validate_request(&request, &actor)?;
        let prior_request_ops = self.log.request_ops(&actor)?;
        let journal = self.log.replay_view()?;
        let records = journal.records();
        if let Some(op_ids) = prior_request_ops {
            let op = one_prior_op(records, &op_ids, &request)?;
            return replay_result(self, op, &request, true);
        }
        if let Some(op) = records.iter().find(|op| {
            detail(op).is_some_and(|detail| {
                detail.get("idempotency_key").and_then(Value::as_str)
                    == Some(request.idempotency_key.as_str())
            })
        }) {
            if op.actor.request != actor.request {
                return Err(CutError::new(
                    codes::CONFLICT,
                    "atomic media insert idempotency key belongs to another request",
                    "reuse the original request identity or allocate a new transaction",
                ));
            }
            return replay_result(self, op, &request, true);
        }
        drop(journal);

        let asset_id = self.next_asset_ids(1)?.pop().ok_or_else(|| {
            CutError::new(
                codes::CONFLICT,
                "atomic media insert could not allocate an asset id",
                "the durable project history did not yield the requested allocation",
            )
        })?;
        let mut insert = request
            .insert
            .as_object()
            .ok_or_else(invalid_insert)?
            .clone();
        insert.insert("asset".into(), Value::String(asset_id.clone()));
        let insert = Value::Object(insert);
        let mut next = self.project.clone();
        if next
            .assets
            .insert(asset_id.clone(), request.asset.clone())
            .is_some()
        {
            return Err(CutError::new(
                codes::CONFLICT,
                format!("asset '{asset_id}' already exists"),
                "atomic media insert ids are allocated from durable project history",
            ));
        }
        let mut effects = apply_track(&mut next, request.track.as_ref(), None)?;
        effects.extend(apply_edit_verb(&mut next, "edit.insert", &insert)?);
        let clip_id = effects
            .iter()
            .find_map(|effect| effect.detail.get("added_clip").and_then(Value::as_str))
            .ok_or_else(|| {
                CutError::new(
                    codes::CONFLICT,
                    "atomic media insert produced no clip identity",
                    "the canonical edit.insert lowering did not bind a timeline clip",
                )
            })?
            .to_string();
        effects.push(edit::fx(
            None,
            json!({
                DETAIL_KEY: true,
                "idempotency_key": request.idempotency_key,
                "asset_id": asset_id,
                "clip_id": clip_id,
                "asset": request.asset,
                "insert": insert,
                "binding": request.binding,
                "track": request.track,
            }),
        ));
        let op = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "edit.insert".into(),
            args: insert,
            rationale,
            effects,
            inverse: None,
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &op)?;
        Ok(AtomicMediaInsertResult {
            asset_id,
            clip_id,
            op,
            already_applied: false,
        })
    }
}

pub(super) fn has_detail(op: &OpRecord) -> bool {
    detail(op).is_some()
}

pub(super) fn asset_id(op: &OpRecord) -> Option<String> {
    detail(op)
        .and_then(|detail| detail.get("asset_id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

pub(super) fn asset_map(op: &OpRecord) -> Result<Option<BTreeMap<String, Asset>>, CutError> {
    let Some(detail) = detail(op) else {
        return Ok(None);
    };
    let asset_id = detail
        .get("asset_id")
        .and_then(Value::as_str)
        .ok_or_else(|| replay_corrupt(op, "atomic media insert asset id is missing"))?;
    let asset = detail
        .get("asset")
        .cloned()
        .ok_or_else(|| replay_corrupt(op, "atomic media insert asset is missing"))?;
    Ok(Some(BTreeMap::from([(
        asset_id.to_string(),
        serde_json::from_value(asset)?,
    )])))
}

pub(super) fn replay(project: &mut Project, op: &OpRecord) -> Result<(), CutError> {
    let assets =
        asset_map(op)?.ok_or_else(|| replay_corrupt(op, "atomic media insert is missing"))?;
    let detail = detail(op).ok_or_else(|| replay_corrupt(op, "atomic media insert is missing"))?;
    let insert = detail
        .get("insert")
        .ok_or_else(|| replay_corrupt(op, "atomic media insert lowering is missing"))?;
    let asset_id = detail
        .get("asset_id")
        .and_then(Value::as_str)
        .ok_or_else(|| replay_corrupt(op, "atomic media insert asset id is missing"))?;
    let clip_id = detail
        .get("clip_id")
        .and_then(Value::as_str)
        .ok_or_else(|| replay_corrupt(op, "atomic media insert clip id is missing"))?;
    if detail
        .get("idempotency_key")
        .and_then(Value::as_str)
        .is_none_or(|key| !is_lower_hex_sha256(key))
        || detail.get("binding").and_then(Value::as_object).is_none()
        || &op.args != insert
        || op.args.get("asset").and_then(Value::as_str) != Some(asset_id)
        || op
            .effects
            .iter()
            .find_map(|effect| effect.detail.get("added_clip").and_then(Value::as_str))
            != Some(clip_id)
    {
        return Err(replay_corrupt(
            op,
            "atomic media insert identity, binding, or lowering is malformed",
        ));
    }
    let mut next = project.clone();
    for (asset_id, asset) in assets {
        if next.assets.insert(asset_id.clone(), asset).is_some() {
            return Err(replay_corrupt(
                op,
                format!("atomic media insert asset '{asset_id}' already exists"),
            ));
        }
    }
    let track = track_topology(detail, op)?;
    let mut pinned = crate::rebase::PinnedIds::from_effects(&op.effects);
    apply_track(&mut next, track.as_ref(), Some(&mut pinned))?;
    apply_edit_verb_pinned(&mut next, "edit.insert", insert, Some(&mut pinned))?;
    *project = next;
    Ok(())
}

fn one_prior_op<'a>(
    records: &'a [OpRecord],
    op_ids: &[String],
    request: &AtomicMediaInsert,
) -> Result<&'a OpRecord, CutError> {
    let [op_id] = op_ids else {
        return Err(CutError::new(
            codes::CONFLICT,
            "atomic media insert request has an ambiguous durable history",
            format!("expected one operation, found {}", op_ids.len()),
        ));
    };
    let op = records
        .iter()
        .find(|op| op.op_id == *op_id)
        .ok_or_else(|| {
            CutError::new(
                codes::CONFLICT,
                "atomic media insert request history is incomplete",
                format!("operation '{op_id}' is absent from the validated journal"),
            )
        })?;
    if detail(op).is_none() {
        return Err(CutError::new(
            codes::CONFLICT,
            "atomic media insert request was reused by another operation",
            format!("operation '{op_id}' is not an atomic media insert"),
        ));
    }
    if detail(op)
        .and_then(|detail| detail.get("idempotency_key"))
        .and_then(Value::as_str)
        != Some(request.idempotency_key.as_str())
    {
        return Err(CutError::new(
            codes::CONFLICT,
            "atomic media insert request does not match its committed transaction",
            "request identity and transaction fingerprint must describe the same placement",
        ));
    }
    Ok(op)
}

fn replay_result(
    store: &ProjectStore,
    op: &OpRecord,
    request: &AtomicMediaInsert,
    already_applied: bool,
) -> Result<AtomicMediaInsertResult, CutError> {
    let detail = detail(op).ok_or_else(|| replay_corrupt(op, "atomic media insert is missing"))?;
    let asset_id = detail
        .get("asset_id")
        .and_then(Value::as_str)
        .ok_or_else(|| replay_corrupt(op, "atomic media insert asset id is missing"))?
        .to_string();
    let clip_id = detail
        .get("clip_id")
        .and_then(Value::as_str)
        .ok_or_else(|| replay_corrupt(op, "atomic media insert clip id is missing"))?
        .to_string();
    if detail.get("asset") != Some(&serde_json::to_value(&request.asset)?)
        || detail.get("insert") != Some(&with_asset(&request.insert, &asset_id)?)
        || detail.get("binding") != Some(&request.binding)
        || detail.get("track").cloned().unwrap_or(Value::Null)
            != serde_json::to_value(&request.track)?
    {
        return Err(CutError::new(
            codes::CONFLICT,
            "atomic media insert idempotency key was reused with different semantics",
            "reuse the exact attested transaction or allocate a new request id",
        ));
    }
    let asset_is_live = store.project.assets.contains_key(&asset_id);
    let clip_is_live = store
        .project
        .all_sequence_tracks()
        .flat_map(|track| track.clips.iter())
        .any(|clip| clip.id() == Some(clip_id.as_str()));
    if !asset_is_live || !clip_is_live {
        return Err(CutError::new(
            codes::CONFLICT,
            "atomic media insert was committed but is not currently active",
            format!("operation '{}' was later undone or removed", op.op_id),
        )
        .with_suggested_action("redo the original operation or allocate a new request id"));
    }
    Ok(AtomicMediaInsertResult {
        asset_id,
        clip_id,
        op: op.clone(),
        already_applied,
    })
}

fn detail(op: &OpRecord) -> Option<&serde_json::Map<String, Value>> {
    if op.verb != "edit.insert" {
        return None;
    }
    op.effects
        .iter()
        .map(|effect| &effect.detail)
        .find(|detail| detail.get(DETAIL_KEY).and_then(Value::as_bool) == Some(true))
}

fn with_asset(insert: &Value, asset_id: &str) -> Result<Value, CutError> {
    let mut insert = insert.as_object().ok_or_else(invalid_insert)?.clone();
    insert.insert("asset".into(), Value::String(asset_id.into()));
    Ok(Value::Object(insert))
}

fn invalid_insert() -> CutError {
    CutError::new(
        codes::INVALID_ARGS,
        "atomic media insert has an invalid insert payload",
        "pass canonical edit.insert arguments as an object",
    )
}

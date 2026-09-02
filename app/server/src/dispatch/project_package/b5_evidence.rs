// B5 grouped-relink evidence retained with a portable package plan.
//
// This stays private to B6 planning: callers receive only the resulting hash,
// never the receipt's source paths or grouped-operation internals.

fn validate_b5_receipt(
    receipt: Option<&Value>,
    source: &PackageSourceSnapshot,
) -> Result<Option<String>, CutError> {
    let Some(receipt) = receipt else {
        if source.current_b5_receipt.is_some() {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "portable package requires the current B5 relink receipt",
                "the current project revision repaired media through grouped bulk relink",
            )
            .with_suggested_action(
                "use the B5 receipt retained by the open project, then preview the package again",
            ));
        }
        return Ok(None);
    };
    if let Some(expected) = source.current_b5_receipt.as_ref() {
        if receipt != expected {
            return Err(b5_error(
                "receipt does not exactly match the current grouped B5 relink operation",
            ));
        }
    }
    let object = receipt
        .as_object()
        .ok_or_else(|| b5_error("receipt must be an object"))?;
    if object.get("schema").and_then(Value::as_str) != Some(B5_RECEIPT_SCHEMA)
        || object.get("immutable").and_then(Value::as_bool) != Some(true)
    {
        return Err(b5_error(
            "receipt is not immutable shellx-cut/media-relink-receipt/1 data",
        ));
    }
    if object.get("project_identity") != Some(&source.project_identity)
        || object.get("post_revision").and_then(Value::as_str)
            != Some(source.project_revision.as_str())
    {
        return Err(b5_error(
            "receipt does not belong to this exact source revision",
        ));
    }
    let plan_hash = object.get("plan_hash").and_then(Value::as_str);
    if !plan_hash.is_some_and(is_exact_sha256) {
        return Err(b5_error("receipt plan_hash is not a full SHA-256 value"));
    }
    let group_id = object.get("grouped_op_id").and_then(Value::as_str);
    if !group_id.is_some_and(|id| {
        source
            .ops
            .iter()
            .any(|op| op.op_id == id && op.verb == "media.relink_apply")
    }) {
        return Err(b5_error(
            "receipt grouped operation is not in the source journal",
        ));
    }
    let assets = object
        .get("assets")
        .and_then(Value::as_array)
        .filter(|assets| !assets.is_empty())
        .ok_or_else(|| b5_error("receipt has no relinked asset rows"))?;
    let mut seen = BTreeSet::new();
    for row in assets {
        let asset_id = row
            .get("asset")
            .and_then(Value::as_str)
            .ok_or_else(|| b5_error("receipt row is missing asset"))?;
        let expected = row
            .get("expected_hash")
            .and_then(Value::as_str)
            .filter(|hash| is_exact_sha256(hash))
            .ok_or_else(|| b5_error("receipt row is missing a full expected_hash"))?;
        let chosen = row
            .get("chosen")
            .and_then(Value::as_object)
            .ok_or_else(|| b5_error("receipt row is missing chosen evidence"))?;
        let chosen_path = chosen
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| b5_error("receipt chosen evidence is missing path"))?;
        let chosen_hash = chosen.get("sha256").and_then(Value::as_str);
        let asset = source
            .project
            .assets
            .get(asset_id)
            .ok_or_else(|| b5_error("receipt names an asset absent from this project"))?;
        if !seen.insert(asset_id)
            || row.get("disposition").and_then(Value::as_str) != Some("relinked")
            || chosen_hash != Some(expected)
            || asset.hash != expected
            || asset.path != chosen_path
        {
            return Err(b5_error("receipt row no longer matches the source project"));
        }
    }
    Ok(Some(hash_json(receipt)?))
}

fn b5_error(cause: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::CONFLICT,
        "portable package refuses stale or invalid B5 relink evidence",
        cause.into(),
    )
    .with_suggested_action(
        "run B5 relink again or omit b5_receipt when no offline media was repaired",
    )
}

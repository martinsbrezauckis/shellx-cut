//! Durable caller idempotency and optimistic project-revision controls.

use crate::state::AppState;
use cut_core::{
    error_codes, Actor, CutError, MutationRequest, ProjectStore, VerbResult, VerbWarning,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

mod receipt;

pub(crate) struct PreparedRequest {
    pub args: Value,
    pub actor: Actor,
    pub controlled: bool,
}

pub(crate) fn prepare(
    name: &str,
    mut args: Value,
    actor: Actor,
) -> Result<PreparedRequest, CutError> {
    let Some(object) = args.as_object_mut() else {
        return Ok(PreparedRequest {
            args,
            actor,
            controlled: false,
        });
    };
    let request_id = take_string(object, "request_id")?;
    let expected_revision = take_string(object, "expected_revision")?;
    let Some(request_id) = request_id else {
        if expected_revision.is_some() {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "expected_revision requires request_id",
                "a revision guard without a durable retry identity cannot be correlated safely",
            )
            .with_suggested_action("supply a unique request_id together with expected_revision"));
        }
        return Ok(PreparedRequest {
            args,
            actor,
            controlled: false,
        });
    };
    let caller = serde_json::to_string(&(actor.kind, actor.name.as_str(), actor.via.as_str()))?;
    let fingerprint = fingerprint(name, &args, expected_revision.as_deref())?;
    Ok(PreparedRequest {
        args,
        actor: actor.with_request(MutationRequest {
            caller,
            request_id,
            fingerprint,
            expected_revision,
        }),
        controlled: true,
    })
}

fn take_string(object: &mut Map<String, Value>, key: &str) -> Result<Option<String>, CutError> {
    match object.remove(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("{key} must be a string"),
            "the shared mutation-control schema requires a JSON string",
        )),
    }
}

fn fingerprint(name: &str, args: &Value, expected: Option<&str>) -> Result<String, CutError> {
    let canonical = canonicalize(json!({
        "verb": name,
        "args": args,
        "expected_revision": expected,
    }));
    let bytes = serde_json::to_vec(&canonical)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn canonicalize(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys: Vec<_> = object.into_iter().collect();
            keys.sort_by(|left, right| left.0.cmp(&right.0));
            Value::Object(
                keys.into_iter()
                    .map(|(key, value)| (key, canonicalize(value)))
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize).collect()),
        other => other,
    }
}

pub(crate) async fn preflight(
    state: &AppState,
    verb: &str,
    actor: &Actor,
) -> Result<Option<VerbResult>, CutError> {
    let guard = state.project.read().await;
    let Some(store) = guard.as_ref() else {
        return Ok(None);
    };
    let Some(op_ids) = store.log.request_ops(actor)? else {
        if let Some(expected) = actor
            .request
            .as_ref()
            .and_then(|request| request.expected_revision.as_deref())
        {
            ensure_revision(store, expected)?;
        }
        return Ok(None);
    };
    receipt::replay(store, verb, actor, &op_ids).map(Some)
}

fn ensure_revision(store: &ProjectStore, expected: &str) -> Result<(), CutError> {
    let actual = store.log.current_revision()?;
    if actual.as_deref() == Some(expected) {
        return Ok(());
    }
    Err(CutError::new(
        error_codes::CONFLICT,
        format!("expected project revision '{expected}' is stale"),
        format!(
            "the current durable project revision is '{}'",
            actual.as_deref().unwrap_or("none")
        ),
    )
    .with_suggested_action("refresh project.state and submit the mutation with a new request_id"))
}

pub(crate) async fn finalize(
    state: &AppState,
    verb: &str,
    actor: &Actor,
    result: &mut VerbResult,
    publish_receipt: bool,
) {
    if !result.ok || result.op_ids.as_ref().is_none_or(Vec::is_empty) {
        return;
    }
    let guard = state.project.read().await;
    let Some(store) = guard.as_ref() else {
        return;
    };
    let revision = match store.log.current_revision() {
        Ok(Some(revision)) => revision,
        Ok(None) => return,
        Err(error) => {
            result
                .warnings
                .get_or_insert_with(Vec::new)
                .push(receipt_warning(error));
            return;
        }
    };
    result.project_revision = Some(revision.clone());
    if !publish_receipt {
        return;
    }
    if let Err(error) = receipt::write(store, verb, actor, &revision, result) {
        result
            .warnings
            .get_or_insert_with(Vec::new)
            .push(receipt_warning(error));
    }
}

fn receipt_warning(error: CutError) -> VerbWarning {
    let mut detail = Map::new();
    detail.insert("committed".into(), Value::Bool(true));
    detail.insert("cause".into(), Value::String(error.to_string()));
    VerbWarning {
        code: "request_receipt_write_failed".into(),
        message: "The operation committed, but exact automatic retry replay is unavailable.".into(),
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cut_core::ActorKind;

    fn actor() -> Actor {
        Actor {
            kind: ActorKind::Agent,
            name: "test".into(),
            via: "rest".into(),
            request: None,
        }
    }

    #[test]
    fn fingerprints_are_stable_across_object_key_order() {
        let left = prepare(
            "edit.add_marker",
            json!({"request_id":"request-0001","label":"x","at_ms":1}),
            actor(),
        )
        .unwrap();
        let right = prepare(
            "edit.add_marker",
            json!({"at_ms":1,"request_id":"request-0001","label":"x"}),
            actor(),
        )
        .unwrap();
        assert_eq!(left.actor.request, right.actor.request);
        assert_eq!(left.args, right.args);
    }
}

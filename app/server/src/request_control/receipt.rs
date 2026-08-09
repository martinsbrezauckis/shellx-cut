//! Atomic project-local response receipts for completed mutation requests.

use cut_core::{error_codes, Actor, CutError, ProjectStore, VerbResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const SCHEMA: &str = "shellx-cut/mutation-request/1";

#[derive(Debug, Serialize, Deserialize)]
struct DurableRequestReceipt {
    schema: String,
    verb: String,
    request_id: String,
    fingerprint: String,
    project_revision: String,
    response: VerbResult,
}

pub(super) fn replay(
    store: &ProjectStore,
    verb: &str,
    actor: &Actor,
    op_ids: &[String],
) -> Result<VerbResult, CutError> {
    let request = actor.request.as_ref().expect("request actor");
    let path = path(store, actor)?;
    if !path.is_file() {
        return Err(CutError::new(
            error_codes::CONFLICT,
            format!(
                "mutation request '{}' already committed but its response receipt is incomplete",
                request.request_id
            ),
            format!("durable operation ids: {}", op_ids.join(", ")),
        )
        .with_suggested_action(
            "do not retry with a new request_id; inspect the listed ops and refresh project.state",
        ));
    }
    let receipt = read(&path)?;
    validate(&receipt, verb, actor, op_ids)?;
    Ok(receipt.response)
}

pub(super) fn write(
    store: &ProjectStore,
    verb: &str,
    actor: &Actor,
    revision: &str,
    result: &VerbResult,
) -> Result<(), CutError> {
    let request = actor.request.as_ref().expect("controlled request actor");
    let op_ids = store.log.request_ops(actor)?.ok_or_else(|| {
        CutError::new(
            error_codes::CONFLICT,
            "mutation response has no matching durable request operation",
            "the response op ids cannot be bound to request metadata in ops.jsonl",
        )
    })?;
    if !response_ops_are_prefix(result, &op_ids) {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "mutation response op ids do not match durable request history",
            format!(
                "journal has {}; response has {:?}",
                op_ids.join(", "),
                result.op_ids
            ),
        ));
    }
    let path = path(store, actor)?;
    if path.is_file() {
        let existing = read(&path)?;
        validate(&existing, verb, actor, &op_ids)?;
        return Ok(());
    }
    let dir = path.parent().expect("receipt path has a parent");
    std::fs::create_dir_all(dir)?;
    let receipt = DurableRequestReceipt {
        schema: SCHEMA.into(),
        verb: verb.into(),
        request_id: request.request_id.clone(),
        fingerprint: request.fingerprint.clone(),
        project_revision: revision.into(),
        response: result.clone(),
    };
    let mut bytes = serde_json::to_vec_pretty(&receipt)?;
    bytes.push(b'\n');
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tmp = dir.join(format!(".request-{}-{nonce}.tmp", std::process::id()));
    let write_result = (|| -> Result<(), CutError> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, &path)?;
        sync_dir(dir);
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    write_result
}

fn path(store: &ProjectStore, actor: &Actor) -> Result<PathBuf, CutError> {
    let request = actor.request.as_ref().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "mutation request metadata is missing",
            "request receipt access requires a controlled actor",
        )
    })?;
    let key = serde_json::to_vec(&(request.caller.as_str(), request.request_id.as_str()))?;
    Ok(store
        .dir
        .join("request-receipts")
        .join(format!("{:x}.json", Sha256::digest(key))))
}

fn read(path: &Path) -> Result<DurableRequestReceipt, CutError> {
    serde_json::from_slice(&std::fs::read(path)?).map_err(|error| {
        CutError::new(
            error_codes::IO,
            "mutation response receipt is corrupt",
            format!("{}: {error}", path.display()),
        )
        .with_suggested_action("inspect the receipt and project journal before retrying")
    })
}

fn validate(
    receipt: &DurableRequestReceipt,
    verb: &str,
    actor: &Actor,
    op_ids: &[String],
) -> Result<(), CutError> {
    let request = actor.request.as_ref().expect("request actor");
    if receipt.schema != SCHEMA
        || receipt.verb != verb
        || receipt.request_id != request.request_id
        || receipt.fingerprint != request.fingerprint
        || !response_ops_are_prefix(&receipt.response, op_ids)
        || receipt.response.project_revision.as_deref() != Some(&receipt.project_revision)
    {
        return Err(CutError::new(
            error_codes::CONFLICT,
            format!(
                "mutation request '{}' has an invalid durable receipt",
                request.request_id
            ),
            "the receipt does not match the caller, payload fingerprint, operation ids, or project revision",
        )
        .with_suggested_action("inspect the receipt and project journal before retrying"));
    }
    Ok(())
}

fn response_ops_are_prefix(response: &VerbResult, durable: &[String]) -> bool {
    let Some(response_ops) = response.op_ids.as_deref() else {
        return false;
    };
    !response_ops.is_empty()
        && response_ops.len() <= durable.len()
        && durable.starts_with(response_ops)
}

fn sync_dir(dir: &Path) {
    if let Ok(file) = File::open(dir) {
        let _ = file.sync_all();
    }
}

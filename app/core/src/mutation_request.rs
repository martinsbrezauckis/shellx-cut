//! Durable mutation-request identity indexed from the operation journal.

use crate::error::{codes, CutError};
use crate::ops::{Actor, OpRecord};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Durable identity attached to every op produced by one caller request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationRequest {
    /// Stable serialized identity of the original transport caller. Nested
    /// recipe/plugin dispatch keeps this scope even when op attribution changes.
    pub caller: String,
    pub request_id: String,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct RequestIndex {
    requests: BTreeMap<String, IndexedRequest>,
}

#[derive(Debug)]
struct IndexedRequest {
    fingerprint: String,
    op_ids: Vec<String>,
}

impl RequestIndex {
    pub fn from_records(records: &[OpRecord]) -> Result<Self, CutError> {
        let mut index = Self::default();
        for record in records {
            let Some(request) = record.actor.request.as_ref() else {
                continue;
            };
            let key = key(&record.actor).expect("a request actor has a request key");
            match index.requests.get_mut(&key) {
                Some(previous) if previous.fingerprint != request.fingerprint => {
                    return Err(CutError::new(
                        codes::CONFLICT,
                        format!(
                            "mutation request '{}' has conflicting payloads in project history",
                            request.request_id
                        ),
                        format!(
                            "operation '{}' does not match the earlier durable request fingerprint",
                            record.op_id
                        ),
                    )
                    .with_suggested_action(
                        "do not append; inspect the request metadata in ops.jsonl with project repair tooling",
                    ));
                }
                Some(previous) => previous.op_ids.push(record.op_id.clone()),
                None => {
                    index.requests.insert(
                        key,
                        IndexedRequest {
                            fingerprint: request.fingerprint.clone(),
                            op_ids: vec![record.op_id.clone()],
                        },
                    );
                }
            }
        }
        Ok(index)
    }

    pub fn validate_append(
        &self,
        op: &OpRecord,
        current_revision: Option<&str>,
    ) -> Result<(), CutError> {
        let Some(request) = op.actor.request.as_ref() else {
            return Ok(());
        };
        let key = key(&op.actor).expect("a request actor has a request key");
        match self.requests.get(&key) {
            Some(previous) if previous.fingerprint != request.fingerprint => Err(conflict(
                &request.request_id,
                "the same caller already used this request_id with a different payload",
            )),
            Some(_) => Ok(()),
            None if request.expected_revision.as_deref() == current_revision => Ok(()),
            None if request.expected_revision.is_none() => Ok(()),
            None => Err(conflict(
                &request.request_id,
                format!(
                    "expected project revision '{}', but the current revision is '{}'",
                    request.expected_revision.as_deref().unwrap_or("none"),
                    current_revision.unwrap_or("none")
                ),
            )),
        }
    }

    /// Record an append that is already durable and pre-validated. This cannot
    /// fail: request keys are an unambiguous length-prefixed encoding, so the
    /// post-fsync journal-index update has no ordinary error path.
    pub fn record(&mut self, op: &OpRecord) {
        let Some(request) = op.actor.request.as_ref() else {
            return;
        };
        let key = key(&op.actor).expect("a request actor has a request key");
        self.requests
            .entry(key)
            .or_insert_with(|| IndexedRequest {
                fingerprint: request.fingerprint.clone(),
                op_ids: Vec::new(),
            })
            .op_ids
            .push(op.op_id.clone());
    }

    pub fn op_ids(&self, actor: &Actor) -> Result<Option<Vec<String>>, CutError> {
        let Some(request) = actor.request.as_ref() else {
            return Ok(None);
        };
        let key = key(actor).expect("a request actor has a request key");
        match self.requests.get(&key) {
            Some(previous) if previous.fingerprint != request.fingerprint => Err(conflict(
                &request.request_id,
                "the same caller already used this request_id with a different payload",
            )),
            Some(previous) => Ok(Some(previous.op_ids.clone())),
            None => Ok(None),
        }
    }
}

fn key(actor: &Actor) -> Option<String> {
    let request = actor.request.as_ref()?;
    Some(format!(
        "{}:{}{}:{}",
        request.caller.len(),
        request.caller,
        request.request_id.len(),
        request.request_id,
    ))
}

fn conflict(request_id: &str, cause: impl Into<String>) -> CutError {
    CutError::new(
        codes::CONFLICT,
        format!("mutation request '{request_id}' conflicts with durable project history"),
        cause,
    )
    .with_suggested_action(
        "use a new request_id for a new action, or refresh project.state and retry from its project_revision",
    )
}

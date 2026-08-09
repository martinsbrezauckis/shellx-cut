//! Durable journal commit state and non-fatal project-cache recovery.

use super::*;

impl ProjectStore {
    fn commit_staged_with_cache_writer<F>(
        &mut self,
        mut next: Project,
        op: &OpRecord,
        write_cache: F,
    ) -> Result<(), CutError>
    where
        F: FnOnce(&Self, &str) -> Result<(), CutError>,
    {
        // Check the generated verb contract before appending. Once the journal
        // is durable, reporting an unknown classification would make the
        // caller's retry ambiguous; rejecting before append preserves that
        // invariant while keeping replay fail-closed for historic corruption.
        let is_history_edit = Self::is_history_edit(op)?;
        next.sync_active_sequence();
        let encoded = serde_json::to_string_pretty(&next)?;
        // Sequence navigation is derived from the operation that is about to
        // append. Build it before durability so no later reconstruction error
        // can make a durable append look like an ordinary failed mutation.
        let sequence_history = matches!(
            op.verb.as_str(),
            "project.sequence_create" | "project.sequence_switch"
        )
        .then(|| Self::history_state_after_append(&self.log, &next.active_sequence, op))
        .transpose()?;
        let append = self.log.append(op)?;
        let cache_result = write_cache(self, &encoded);
        self.project = next;
        if let Some((history, pos)) = sequence_history {
            self.undo_history = history;
            self.undo_pos = pos;
            self.tip_group = None;
        } else {
            self.record_history_commit(op, is_history_edit);
        }
        snapshots::write_if_due(&self.dir, &self.log, &self.project);
        if let Some(cause) = append.identity_degraded() {
            let mut detail = serde_json::Map::new();
            detail.insert("committed".into(), Value::Bool(true));
            detail.insert("op_id".into(), Value::String(op.op_id.clone()));
            detail.insert("cause".into(), Value::String(cause.into()));
            self.commit_warnings
                .entry(op.op_id.clone())
                .or_default()
                .push(VerbWarning {
                    code: "journal_identity_refresh_degraded".into(),
                    message: "The operation committed, but journal identity validation is unavailable until the project is reopened."
                        .into(),
                    detail,
                });
        }
        if let Err(error) = cache_result {
            // The journal is already durable and live memory reflects it. A
            // simple error here would invite a duplicate non-idempotent retry.
            let cache_rebuilt = self.save().is_ok();
            let mut detail = serde_json::Map::new();
            detail.insert("committed".into(), Value::Bool(true));
            detail.insert("op_id".into(), Value::String(op.op_id.clone()));
            detail.insert("cache_rebuilt".into(), Value::Bool(cache_rebuilt));
            detail.insert("cause".into(), Value::String(error.to_string()));
            self.commit_warnings
                .entry(op.op_id.clone())
                .or_default()
                .push(VerbWarning {
                    code: "project_cache_write_failed".into(),
                    message: if cache_rebuilt {
                        "The operation committed and the project cache was rebuilt after a write failure."
                            .into()
                    } else {
                        "The operation committed, but project.json remains degraded; reopen rebuilds it from the journal."
                            .into()
                    },
                    detail,
                });
        }
        Ok(())
    }

    /// Append the truth record, publish live state, and refresh the disposable
    /// cache. Cache failure after append becomes an in-band warning.
    pub fn commit_staged(&mut self, next: Project, op: &OpRecord) -> Result<(), CutError> {
        self.commit_staged_with_cache_writer(next, op, |store, encoded| {
            store.write_project_json(encoded)
        })
    }

    /// Drain cache warnings for exactly the operations returned to one caller.
    pub fn take_commit_warnings(&mut self, op_ids: &[String]) -> Vec<VerbWarning> {
        op_ids
            .iter()
            .flat_map(|op_id| self.commit_warnings.remove(op_id).into_iter().flatten())
            .collect()
    }

    pub fn commit(&mut self, op: &OpRecord) -> Result<(), CutError> {
        self.commit_staged(self.project.clone(), op)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_write_failure_after_append_is_success_with_a_commit_warning() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::create(dir.path(), "demo", None).unwrap();
        let mut next = store.project.clone();
        let args = json!({"at_ms": 250, "label": "durable"});
        let effects = apply_edit_verb(&mut next, "edit.add_marker", &args).unwrap();
        let record = OpRecord {
            op_id: store.log.next_id().unwrap(),
            ts: OpRecord::now_ts(),
            actor: Actor::system(),
            verb: "edit.add_marker".into(),
            args,
            rationale: None,
            effects,
            inverse: None,
            status: OpStatus::Applied,
        };

        store
            .commit_staged_with_cache_writer(next, &record, |_store, _encoded| {
                Err(CutError::new(
                    codes::IO,
                    "injected project cache write failure",
                    "test fault after durable journal append",
                ))
            })
            .expect("a durable mutation must not become an ambiguous error");

        assert_eq!(store.log.read_all().unwrap().len(), 2);
        assert_eq!(store.project.markers.len(), 1);
        let warnings = store.take_commit_warnings(std::slice::from_ref(&record.op_id));
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "project_cache_write_failed");
        assert_eq!(warnings[0].detail["committed"], true);
        assert_eq!(warnings[0].detail["op_id"], record.op_id);
        assert_eq!(warnings[0].detail["cache_rebuilt"], true);
        assert!(store
            .take_commit_warnings(std::slice::from_ref(&record.op_id))
            .is_empty());

        let reopened = ProjectStore::open(&store.dir).unwrap();
        assert_eq!(reopened.project.markers.len(), 1);
        assert_eq!(reopened.log.read_all().unwrap().len(), 2);
    }

    #[test]
    fn post_sync_identity_failure_is_committed_warned_and_retry_safe() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::create(dir.path(), "demo", None).unwrap();
        let mut next = store.project.clone();
        let args = json!({"at_ms": 250, "label": "durable"});
        let effects = apply_edit_verb(&mut next, "edit.add_marker", &args).unwrap();
        let actor = Actor::system().with_request(crate::MutationRequest {
            caller: "test-client".into(),
            request_id: "post-sync-stamp-failure".into(),
            fingerprint: format!("sha256:{}", "a".repeat(64)),
            expected_revision: Some("op_000001".into()),
        });
        let record = OpRecord {
            op_id: store.log.next_id().unwrap(),
            ts: OpRecord::now_ts(),
            actor,
            verb: "edit.add_marker".into(),
            args,
            rationale: None,
            effects,
            inverse: None,
            status: OpStatus::Applied,
        };

        store.log.inject_next_stamp_refresh_failure();
        store
            .commit_staged(next, &record)
            .expect("a post-sync identity fault must acknowledge the durable commit");
        assert_eq!(store.project.markers.len(), 1);
        // The append is known durable from its returned op/warning, but the
        // failed post-sync filesystem stamp means no later journal read may
        // trust the in-memory index until a strict reopen revalidates it.
        for error in [
            store.log.current_revision().unwrap_err(),
            store.log.next_id().unwrap_err(),
            store.log.request_ops(&record.actor).unwrap_err(),
        ] {
            assert_eq!(error.code, codes::CONFLICT);
            assert!(error.message.contains("needs revalidation"));
        }
        let warnings = store.take_commit_warnings(std::slice::from_ref(&record.op_id));
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "journal_identity_refresh_degraded");
        assert_eq!(warnings[0].detail["committed"], true);
        assert_eq!(warnings[0].detail["op_id"], record.op_id);

        let retry = store.log.append(&record).unwrap_err();
        assert_eq!(retry.code, codes::CONFLICT);
        assert!(retry.message.contains("needs revalidation"));

        // Reopen revalidates strict journal truth and restores request
        // idempotency metadata without duplicating the acknowledged record.
        let reopened = ProjectStore::open(&store.dir).unwrap();
        assert_eq!(reopened.log.read_all().unwrap().len(), 2);
        assert_eq!(
            reopened.log.current_revision().unwrap().as_deref(),
            Some(record.op_id.as_str())
        );
        assert_eq!(reopened.log.next_id().unwrap(), "op_000003");
        assert_eq!(
            reopened.log.request_ops(&record.actor).unwrap(),
            Some(vec![record.op_id])
        );
    }
}

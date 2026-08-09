//! Open-time journal records plus prefix identities for replay consumers.
//!
//! This is an in-memory acceleration index only. `ops.jsonl` stays durable
//! truth; callers validate its byte length before borrowing this view, and a
//! successful append updates the index only after `sync_data` completes.

use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::SystemTime;

/// Result of an acknowledged journal append. A degraded identity stamp is a
/// committed success: the operation is durable and indexed, while the next
/// journal access fails closed until the project is reopened and revalidated.
#[derive(Debug, Default)]
pub struct AppendOutcome {
    pub(super) identity_degraded: Option<String>,
}

impl AppendOutcome {
    /// A committed append that needs a reopen before the next journal access.
    pub fn identity_degraded(&self) -> Option<&str> {
        self.identity_degraded.as_deref()
    }
}

/// A bounded cursor-addressed slice of the durable operation history.
#[derive(Debug, Clone)]
pub struct JournalPage {
    pub ops: Vec<OpRecord>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
    /// Sum of canonical JSON bytes for returned records, excluding the outer
    /// response envelope. This makes page evidence deterministic.
    pub encoded_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct JournalStamp {
    byte_len: u64,
    modified: Option<SystemTime>,
}

#[derive(Debug)]
enum JournalIdentity {
    Verified(JournalStamp),
    /// The append itself is already durable, but the filesystem could not
    /// provide a new identity stamp. Refuse further use until open performs a
    /// strict scan again rather than trusting the stale pre-append identity.
    Degraded,
}

impl JournalStamp {
    fn from_path(path: &Path) -> Result<Self, CutError> {
        let metadata = std::fs::metadata(path)?;
        Ok(Self {
            byte_len: metadata.len(),
            // Do not silently drop the file identity signal: if the platform
            // cannot expose it, replay must fail instead of trusting a
            // potentially replaced same-length journal.
            modified: Some(metadata.modified()?),
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct JournalView {
    records: Arc<Vec<OpRecord>>,
    prefix_hashes: Arc<Vec<[u8; 32]>>,
}

impl JournalView {
    pub(crate) fn records(&self) -> &[OpRecord] {
        self.records.as_slice()
    }

    pub(crate) fn prefix_hash(&self, prefix_len: usize) -> Option<String> {
        self.prefix_hashes.get(prefix_len).copied().map(digest_hex)
    }

    #[cfg(test)]
    pub(crate) fn test_from_records(records: Vec<OpRecord>) -> Result<Self, CutError> {
        Ok(Self {
            prefix_hashes: Arc::new(build_prefix_hashes(&records)?),
            records: Arc::new(records),
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct JournalIndexMetrics {
    pub(crate) full_journal_rereads: u64,
    pub(crate) full_prefix_rehashes: u64,
    pub(crate) appended_identity_updates: u64,
    pub(crate) record_vector_cow_clones: u64,
    pub(crate) prefix_vector_cow_clones: u64,
    pub(crate) paged_records_examined: u64,
}

#[derive(Debug)]
pub(super) struct JournalIndex {
    pub(super) next_seq: u64,
    identity: JournalIdentity,
    pub(super) requests: RequestIndex,
    records: Arc<Vec<OpRecord>>,
    prefix_hashes: Arc<Vec<[u8; 32]>>,
    op_positions: BTreeMap<String, usize>,
    metrics: JournalIndexMetrics,
    #[cfg(test)]
    fail_next_stamp_refresh: bool,
}

impl JournalIndex {
    pub(super) fn new(
        next_seq: u64,
        path: &Path,
        requests: RequestIndex,
        records: Vec<OpRecord>,
    ) -> Result<Self, CutError> {
        let prefix_hashes = build_prefix_hashes(&records)?;
        let mut op_positions = BTreeMap::new();
        for (position, record) in records.iter().enumerate() {
            if op_positions
                .insert(record.op_id.clone(), position)
                .is_some()
            {
                return Err(CutError::new(
                    crate::error::codes::CONFLICT,
                    format!("operation journal repeats id '{}'", record.op_id),
                    "strict replay requires every durable operation id to be unique",
                ));
            }
        }
        Ok(Self {
            next_seq,
            identity: JournalIdentity::Verified(JournalStamp::from_path(path)?),
            requests,
            records: Arc::new(records),
            prefix_hashes: Arc::new(prefix_hashes),
            op_positions,
            metrics: JournalIndexMetrics {
                full_prefix_rehashes: 1,
                ..JournalIndexMetrics::default()
            },
            #[cfg(test)]
            fail_next_stamp_refresh: false,
        })
    }

    pub(super) fn view(&self) -> JournalView {
        JournalView {
            records: Arc::clone(&self.records),
            prefix_hashes: Arc::clone(&self.prefix_hashes),
        }
    }

    pub(super) fn clone_records(&self) -> Vec<OpRecord> {
        self.records.as_ref().clone()
    }

    pub(super) fn page_after(
        &mut self,
        cursor: Option<&str>,
        limit: usize,
        max_bytes: usize,
    ) -> Result<JournalPage, CutError> {
        if limit == 0 || max_bytes == 0 {
            return Err(CutError::new(
                crate::error::codes::INVALID_ARGS,
                "operation page bounds must be positive",
                "limit and max_bytes must both be at least one",
            ));
        }
        let start = match cursor {
            None => 0,
            Some(cursor) => self.op_positions.get(cursor).map_or_else(
                || {
                    Err(CutError::new(
                        crate::error::codes::NOT_FOUND,
                        format!("op '{cursor}' not found in log"),
                        "refresh project.state without since_revision before requesting incremental history",
                    ))
                },
                |position| Ok(position + 1),
            )?,
        };
        let end = start.saturating_add(limit).min(self.records.len());
        let mut ops = Vec::with_capacity(end.saturating_sub(start));
        let mut encoded_bytes: usize = 0;
        for record in &self.records[start..end] {
            let bytes = serde_json::to_vec(record)?.len();
            if bytes > max_bytes && ops.is_empty() {
                return Err(CutError::new(
                    crate::error::codes::INVALID_ARGS,
                    format!("op '{}' exceeds the bounded sync page", record.op_id),
                    format!("one operation encodes to {bytes} bytes; pages cap records at {max_bytes} bytes"),
                ));
            }
            if encoded_bytes.saturating_add(bytes) > max_bytes {
                break;
            }
            encoded_bytes += bytes;
            ops.push(record.clone());
        }
        let consumed = start + ops.len();
        self.metrics.paged_records_examined += ops.len() as u64;
        Ok(JournalPage {
            next_cursor: (consumed < self.records.len())
                .then(|| self.records[consumed - 1].op_id.clone()),
            has_more: consumed < self.records.len(),
            ops,
            encoded_bytes,
        })
    }

    pub(super) fn ensure_unmodified(&self, path: &Path) -> Result<(), CutError> {
        let JournalIdentity::Verified(expected) = &self.identity else {
            return Err(CutError::new(
                crate::error::codes::CONFLICT,
                "operation journal identity needs revalidation",
                "the previous durable append could not refresh its filesystem identity stamp",
            )
            .with_suggested_action(
                "close and reopen the project to strictly validate and reindex the journal before editing",
            ));
        };
        let actual = JournalStamp::from_path(path)?;
        if actual == *expected {
            return Ok(());
        }
        Err(CutError::new(
            crate::error::codes::CONFLICT,
            "operation journal changed outside the open project",
            format!(
                "expected {} bytes but ops.jsonl now has {}",
                expected.byte_len, actual.byte_len
            ),
        )
        .with_suggested_action(
            "close and reopen the project to validate and reindex the journal before editing",
        ))
    }

    /// Refresh after `sync_data`. A refresh failure cannot turn an already
    /// durable operation into an ordinary failure; leave the index degraded so
    /// the next access fails closed until a strict reopen repairs it.
    pub(super) fn refresh_stamp_after_durable_append(&mut self, path: &Path) -> Option<String> {
        #[cfg(test)]
        if std::mem::take(&mut self.fail_next_stamp_refresh) {
            self.identity = JournalIdentity::Degraded;
            return Some("injected post-sync journal identity refresh failure".into());
        }
        match JournalStamp::from_path(path) {
            Ok(stamp) => {
                self.identity = JournalIdentity::Verified(stamp);
                None
            }
            Err(error) => {
                self.identity = JournalIdentity::Degraded;
                Some(error.to_string())
            }
        }
    }

    /// Called only after the journal write and fsync have succeeded.
    pub(super) fn record_durable_append(&mut self, op: OpRecord, encoded: &[u8]) {
        let previous = self
            .prefix_hashes
            .last()
            .copied()
            .expect("journal prefix index always contains the empty prefix");
        if Arc::strong_count(&self.records) > 1 {
            self.metrics.record_vector_cow_clones += 1;
        }
        if Arc::strong_count(&self.prefix_hashes) > 1 {
            self.metrics.prefix_vector_cow_clones += 1;
        }
        let position = self.records.len();
        self.op_positions.insert(op.op_id.clone(), position);
        Arc::make_mut(&mut self.records).push(op);
        Arc::make_mut(&mut self.prefix_hashes).push(next_hash(previous, encoded));
        self.metrics.appended_identity_updates += 1;
    }

    #[cfg(test)]
    pub(crate) fn reset_metrics(&mut self) {
        self.metrics = JournalIndexMetrics::default();
    }

    #[cfg(test)]
    pub(crate) fn metrics(&self) -> JournalIndexMetrics {
        self.metrics
    }

    #[cfg(test)]
    pub(crate) fn inject_next_stamp_refresh_failure(&mut self) {
        self.fail_next_stamp_refresh = true;
    }
}

fn build_prefix_hashes(records: &[OpRecord]) -> Result<Vec<[u8; 32]>, CutError> {
    let mut hashes = Vec::with_capacity(records.len() + 1);
    let mut digest: [u8; 32] = Sha256::digest(b"shellx-cut/history-prefix/1").into();
    hashes.push(digest);
    for record in records {
        let encoded = serde_json::to_vec(record)?;
        digest = next_hash(digest, &encoded);
        hashes.push(digest);
    }
    Ok(hashes)
}

fn next_hash(previous: [u8; 32], encoded: &[u8]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(previous);
    hash.update((encoded.len() as u64).to_be_bytes());
    hash.update(encoded);
    hash.finalize().into()
}

fn digest_hex(digest: [u8; 32]) -> String {
    let mut text = String::with_capacity(64);
    for byte in digest {
        write!(&mut text, "{byte:02x}").expect("writing to String cannot fail");
    }
    text
}

#[cfg(test)]
mod tests;

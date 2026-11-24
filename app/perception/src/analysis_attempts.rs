//! Execution evidence for one explicitly requested file/instrument battery.
//! This module never schedules analysis or probes tools. Missing records remain
//! unknown; a successful process without provider events does not invent a run.
//! History is bounded execution evidence, not a check verdict or a full audit
//! log. Consumers must also have the matching analysis facts before claiming a
//! check passed. Embed finish's snapshot with successful analysis results; a
//! persistence failure is supplemental and must not become an export refusal.
use cut_core::CutError;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
mod store;
pub use store::load_attempts;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    Asked,
    Running,
    Succeeded,
    Failed,
    NotApplicable,
    NotAvailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentAttempt {
    pub instrument: String,
    pub provider: String,
    pub state: ExecutionState,
    pub ran: bool,
    pub started_at_unix_ms: Option<u64>,
    pub finished_at_unix_ms: Option<u64>,
    pub tool_version: Option<String>,
    pub version_reason: Option<String>,
    #[serde(default)]
    pub tool_versions: std::collections::BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub tool_version_errors: std::collections::BTreeMap<String, String>,
    pub error: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisAttempt {
    pub schema_version: u64,
    pub attempt_id: String,
    pub asset_id: String,
    pub asset_hash: String,
    pub asked_at_unix_ms: u64,
    pub finished_at_unix_ms: Option<u64>,
    pub instruments: Vec<InstrumentAttempt>,
    pub error: Option<CutError>,
    pub storage_error: Option<CutError>,
    pub protocol_error: Option<String>,
}

#[derive(Deserialize)]
struct Event {
    instrument: String,
    provider: String,
    state: ExecutionState,
    at_unix_ms: u64,
    tool_version: Option<String>,
    version_reason: Option<String>,
    #[serde(default)]
    tool_versions: std::collections::BTreeMap<String, Option<String>>,
    #[serde(default)]
    tool_version_errors: std::collections::BTreeMap<String, String>,
    error: Option<serde_json::Value>,
}

pub struct AttemptRecorder {
    receipts_dir: PathBuf,
    persist: bool,
    record: Mutex<AnalysisAttempt>,
}
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

impl AttemptRecorder {
    /// Construct without writing: cache hits must not acquire a fake new attempt.
    /// `persist:false` is for immutable/ephemeral consumers, which embed finish's
    /// returned record in their own receipt rather than mutate the source cache.
    pub fn new(
        receipts_dir: &Path,
        asset_id: &str,
        asset_hash: &str,
        instruments: &[&str],
        persist: bool,
    ) -> Self {
        let asked_at_unix_ms = now_ms();
        Self {
            receipts_dir: receipts_dir.into(),
            persist,
            record: Mutex::new(AnalysisAttempt {
                schema_version: 1,
                attempt_id: format!(
                    "{}-{asked_at_unix_ms}-{}",
                    std::process::id(),
                    SEQUENCE.fetch_add(1, Ordering::Relaxed)
                ),
                asset_id: asset_id.into(),
                asset_hash: asset_hash.into(),
                asked_at_unix_ms,
                finished_at_unix_ms: None,
                instruments: instruments
                    .iter()
                    .map(|name| InstrumentAttempt {
                        instrument: (*name).into(),
                        provider: "perception".into(),
                        state: ExecutionState::Asked,
                        ran: false,
                        started_at_unix_ms: None,
                        finished_at_unix_ms: None,
                        tool_version: None,
                        version_reason: Some("The provider has not reported a version.".into()),
                        tool_versions: Default::default(),
                        tool_version_errors: Default::default(),
                        error: None,
                    })
                    .collect(),
                error: None,
                storage_error: None,
                protocol_error: None,
            }),
        }
    }

    /// Call only after deciding a fresh request is needed, never on a cache hit.
    pub fn asked(&self) {
        let mut record = self.record.lock().unwrap_or_else(|p| p.into_inner());
        if record.finished_at_unix_ms.is_none() {
            self.save(&mut record);
        }
    }

    /// Observe only the dedicated bounded stderr protocol. Other logs/progress
    /// remain untouched. Malformed evidence is retained, never a content verdict.
    pub fn observe(&self, line: &str) -> bool {
        let Some(payload) = line.strip_prefix("CUT_ATTEMPT ") else {
            return false;
        };
        let mut record = self.record.lock().unwrap_or_else(|p| p.into_inner());
        if record.finished_at_unix_ms.is_some() {
            return true;
        }
        let event = (payload.len() <= 16 * 1024)
            .then(|| serde_json::from_str::<Event>(payload).ok())
            .flatten();
        let Some(mut event) = event else {
            record.protocol_error =
                Some("The analysis provider returned malformed attempt evidence.".into());
            self.save(&mut record);
            return true;
        };
        let valid = record
            .instruments
            .iter()
            .any(|i| i.instrument == event.instrument)
            && !event.provider.is_empty()
            && event.provider.len() <= 128
            && (record.instruments.len() < 128
                || record
                    .instruments
                    .iter()
                    .any(|i| i.instrument == event.instrument && i.provider == event.provider))
            && event.tool_versions.len() <= 64
            && event.tool_version_errors.len() <= 64;
        if !valid {
            record.protocol_error =
                Some("The analysis provider returned unexpected attempt evidence.".into());
            self.save(&mut record);
            return true;
        }
        if event.state == ExecutionState::Failed && event.error.is_none() {
            let reason = "The provider reported failure without an error description.";
            record.protocol_error = Some(reason.into());
            event.error = Some(serde_json::json!({
                "code": "sidecar", "message": reason,
                "cause": "The failure event did not include its error."
            }));
        }
        let previous = record
            .instruments
            .iter()
            .position(|i| i.instrument == event.instrument && i.provider == event.provider);
        let old = previous.map(|index| &record.instruments[index]);
        // A retry needs a distinct provider attempt. Never erase a terminal
        // failure (or success) with late contradictory execution evidence.
        if old.is_some_and(|i| {
            !matches!(i.state, ExecutionState::Asked | ExecutionState::Running)
                && event.state != i.state
        }) {
            record.protocol_error =
                Some("The analysis provider returned out-of-order attempt evidence.".into());
            self.save(&mut record);
            return true;
        }
        let running = event.state == ExecutionState::Running;
        let terminal = !matches!(event.state, ExecutionState::Asked | ExecutionState::Running);
        let next = InstrumentAttempt {
            instrument: event.instrument,
            provider: event.provider,
            state: event.state,
            ran: running || event.state == ExecutionState::Succeeded || old.is_some_and(|i| i.ran),
            started_at_unix_ms: old
                .and_then(|i| i.started_at_unix_ms)
                .or(running.then_some(event.at_unix_ms)),
            finished_at_unix_ms: terminal.then_some(event.at_unix_ms),
            version_reason: if event.tool_version.is_none() {
                Some(
                    event
                        .version_reason
                        .unwrap_or_else(|| "The provider did not supply a version.".into()),
                )
            } else {
                event.version_reason
            },
            tool_version: event.tool_version,
            tool_versions: event.tool_versions,
            tool_version_errors: event.tool_version_errors,
            error: event.error,
        };
        if let Some(index) = previous {
            record.instruments[index] = next;
        } else {
            record.instruments.push(next);
        }
        self.save(&mut record);
        true
    }

    /// Preserve the original battery error. Only unfinished providers are marked
    /// failed; a completed basic measurement survives an enhancement failure.
    pub fn finish(&self, error: Option<&CutError>) -> AnalysisAttempt {
        let mut record = self.record.lock().unwrap_or_else(|p| p.into_inner());
        if record.finished_at_unix_ms.is_some() && error.is_none() {
            return record.clone();
        }
        let finished_at = Some(now_ms());
        record.finished_at_unix_ms = finished_at;
        record.error = error.cloned();
        if let Some(error) = error {
            for instrument in &mut record.instruments {
                if matches!(
                    instrument.state,
                    ExecutionState::Asked | ExecutionState::Running
                ) {
                    instrument.state = ExecutionState::Failed;
                    instrument.finished_at_unix_ms = finished_at;
                    // Keep the full raw error once, on the attempt, rather
                    // than duplicating a large process log per instrument.
                    instrument.error = Some(serde_json::json!({
                        "code": error.code,
                        "message": "Analysis ended before this provider returned a result.",
                        "cause": "See the analysis attempt's original error."
                    }));
                }
            }
        }
        // Asked-without-events stays unknown. Running-without-completion is
        // different: the provider started, then failed to return its evidence.
        let mut incomplete = false;
        for instrument in &mut record.instruments {
            if instrument.state == ExecutionState::Running {
                incomplete = true;
                instrument.state = ExecutionState::Failed;
                instrument.finished_at_unix_ms = finished_at;
                instrument.error = Some(serde_json::json!({
                    "code": "sidecar", "message": "Provider did not return a completion record",
                    "cause": "The process ended without terminal execution evidence."
                }));
            }
        }
        if incomplete {
            record.protocol_error =
                Some("A provider started but did not return a completion record.".into());
        }
        self.save(&mut record);
        record.clone()
    }

    pub fn snapshot(&self) -> AnalysisAttempt {
        self.record
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    fn save(&self, record: &mut AnalysisAttempt) {
        if self.persist {
            store::save(&self.receipts_dir, record);
        } else {
            store::remember_session(&self.receipts_dir, record);
        }
    }
}

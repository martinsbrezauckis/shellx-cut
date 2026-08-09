//! events.rs — the WS event stream types + bus (public verb contract "events").
//!
//! Role: typed events `{type: op_applied|job_progress|render_done|
//! receipt_ready|project_changed|ui_state|doctor_updated, ...}` fanned out to every connected WS client
//! (UI panels and remote agents subscribe to the same stream:
//! the UI sees live state because it is just another client).
//! Dependencies: tokio broadcast, serde, cut-core. Primary callers:
//! dispatch.rs (op_applied), jobs.rs (job_progress/render_done),
//! http.rs (WS handler), ui verbs (ui_state).

use cut_core::{OpRecord, RenderReceipt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Every message on GET /api/events. `type` is the serde tag, snake_case to
/// match the public contract event names exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// An op was appended to the log (the review rail's live feed).
    OpApplied { op: OpRecord },
    /// Background job progress (transcribe/perception/render...), 0..=1.
    JobProgress {
        job_id: String,
        kind: String,
        progress: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// A render job finished (success or failure; receipt follows on success).
    RenderDone {
        job_id: String,
        render_id: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// A RenderReceipt (checks included) is ready (render-receipt contract evidence).
    ReceiptReady { receipt: RenderReceipt },
    /// UI client state changed (panels/playhead/selection) — lets a remote
    /// agent mirror what the human sees.
    UiState { state: serde_json::Value },
    /// The active project changed through REST/CLI/MCP or the local UI. Visible
    /// clients must refresh even though create/open/close are workspace
    /// transitions rather than project-log operations.
    ProjectChanged {
        open: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// The environment doctor report changed: a capability flipped —
    /// e.g. ffmpeg went missing→present after system.fetch_tool, or a refresh
    /// re-detected a judge CLI. The start wizard + status-bar environment chip
    /// re-render off this. Fired on startup scan, on refresh, and after a
    /// successful fetch_tool — but only when capabilities actually changed.
    DoctorUpdated { report: crate::doctor::DoctorReport },
}

/// Broadcast bus: cheap clone, every WS connection subscribes. Buffer of 256
/// events; slow consumers miss old events (they resync via revisioned project.state).
#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Event>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self { tx }
    }

    /// Publish to all subscribers. Errors (no receivers) are fine — events
    /// are best-effort; the op-log is the durable record.
    pub fn publish(&self, event: Event) {
        let _ = self.tx.send(event);
    }

    /// New subscription for a WS connection.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }
}

/// Serialize one WS frame. Operation events carry a revision chain so clients
/// can detect a missed frame without treating best-effort broadcast delivery as
/// durable truth.
pub fn wire_event(event: &Event) -> serde_json::Value {
    match event {
        Event::OpApplied { op } => serde_json::json!({
            "type": "op_applied",
            "op": op,
            "revision": op.op_id,
            "from_revision": prior_revision(&op.op_id),
            "delta": {"kind": "op", "count": 1},
        }),
        _ => serde_json::to_value(event).unwrap_or_default(),
    }
}

fn prior_revision(revision: &str) -> Option<String> {
    revision
        .strip_prefix("op_")?
        .parse::<u64>()
        .ok()?
        .checked_sub(1)
        .filter(|previous| *previous > 0)
        .map(|previous| cut_core::OpRecord::format_id(previous - 1))
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_frames_expose_a_revision_chain_for_gap_repair() {
        let op = OpRecord {
            op_id: "op_000002".into(),
            ts: "2026-08-08T00:00:00.000Z".into(),
            actor: cut_core::Actor::system(),
            verb: "edit.add_marker".into(),
            args: serde_json::json!({"at_ms": 100, "label": "sync"}),
            rationale: None,
            effects: vec![],
            inverse: None,
            status: cut_core::OpStatus::Applied,
        };

        let frame = wire_event(&Event::OpApplied { op });
        assert_eq!(frame["revision"], "op_000002");
        assert_eq!(frame["from_revision"], "op_000001");
        assert_eq!(frame["delta"], serde_json::json!({"kind":"op", "count":1}));
    }
}

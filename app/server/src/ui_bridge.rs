//! ui_bridge.rs — server→UI-client command channel + screenshot relay
//! (public verb contract ui.* / "ui.screenshot is a verification PRIMITIVE").
//!
//! Role: tracks which WS connections belong to UI clients (they announce by
//! pushing `ui_state` or `ui_hello`), lets verbs send them commands
//! (`ui_command`), and correlates request/response pairs for both UI commands
//! (`ui_command` → `ui_command_result`) and screenshots
//! (`screenshot_request` → `screenshot_result`).
//!
//! Design: oneshot per pending request; UnboundedSender per UI socket. The
//! newest UI client wins for relayed commands (multiple tabs are legal —
//! only one is the human's working surface; last-connected is the best
//! guess and is observable via ui.state).
//!
//! Dependencies: tokio sync, serde_json, cut-core (errors). Primary callers:
//! dispatch.rs (ui.* verbs), http.rs (WS handler registers/unregisters).

use cut_core::{error_codes, CutError};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

/// How long a relayed UI request waits for the exact client response.
const REQUEST_TIMEOUT_SECS: u64 = 10;

/// Shared bridge handle (Arc inside, cheap clone).
#[derive(Clone, Default)]
pub struct UiBridge {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    /// client_id → outbound text-frame sender. Insertion order tracked via
    /// `order` so "newest client" is well-defined.
    clients: HashMap<u64, mpsc::UnboundedSender<String>>,
    order: Vec<u64>,
    next_client: u64,
    /// request_id → waiting verb call, bound to the exact socket and response
    /// shape that received the request. A different tab must never be able to
    /// acknowledge another tab's command.
    pending: HashMap<u64, PendingRequest>,
    next_request: u64,
}

struct PendingRequest {
    client_id: u64,
    response_type: &'static str,
    verb: Option<String>,
    sender: oneshot::Sender<Value>,
}

impl UiBridge {
    /// Register a WS connection as a UI client; returns its id (used to
    /// unregister on disconnect).
    pub fn register(&self, tx: mpsc::UnboundedSender<String>) -> u64 {
        let mut g = self.inner.lock().expect("ui bridge lock");
        g.next_client += 1;
        let id = g.next_client;
        g.clients.insert(id, tx);
        g.order.push(id);
        id
    }

    /// Remove a disconnected client.
    pub fn unregister(&self, id: u64) {
        let mut g = self.inner.lock().expect("ui bridge lock");
        g.clients.remove(&id);
        g.order.retain(|x| *x != id);
        // Dropping the senders wakes every waiter immediately with a
        // disconnect error; do not leave commands hanging for the timeout.
        g.pending.retain(|_, pending| pending.client_id != id);
    }

    pub fn client_count(&self) -> usize {
        self.inner.lock().expect("ui bridge lock").clients.len()
    }

    /// Request/response round-trip: stamps a request_id onto `msg`, sends it
    /// to the newest UI client, awaits the correlated `resolve` (or times
    /// out with an actionable error).
    pub async fn request(&self, msg: Value) -> Result<Value, CutError> {
        self.request_with_timeout(msg, std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .await
    }

    async fn request_with_timeout(
        &self,
        mut msg: Value,
        timeout: std::time::Duration,
    ) -> Result<Value, CutError> {
        let (tx, rx) = oneshot::channel();
        let (request_id, response_type, verb) = {
            let mut g = self.inner.lock().expect("ui bridge lock");
            let client_id = *g.order.last().ok_or_else(no_ui_client)?;
            let sender = g.clients.get(&client_id).ok_or_else(no_ui_client)?.clone();
            let request_type = msg.get("type").and_then(Value::as_str).unwrap_or_default();
            let (response_type, verb) = match request_type {
                "screenshot_request" => ("screenshot_result", None),
                "ui_command" => (
                    "ui_command_result",
                    msg.get("verb").and_then(Value::as_str).map(str::to_owned),
                ),
                _ => {
                    return Err(CutError::new(
                        error_codes::INVALID_ARGS,
                        "unsupported UI bridge request",
                        format!(
                            "request type '{request_type}' has no correlated response contract"
                        ),
                    ));
                }
            };
            g.next_request += 1;
            let rid = g.next_request;
            msg["request_id"] = serde_json::json!(rid);
            g.pending.insert(
                rid,
                PendingRequest {
                    client_id,
                    response_type,
                    verb: verb.clone(),
                    sender: tx,
                },
            );
            // Select the newest client, register the pending request, and send
            // while holding one lock. That prevents a newer tab from stealing
            // the correlation between selection and delivery.
            if sender.send(msg.to_string()).is_err() {
                g.pending.remove(&rid);
                g.clients.remove(&client_id);
                g.order.retain(|known| *known != client_id);
                return Err(no_ui_client());
            }
            (rid, response_type, verb)
        };
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(_)) => Err(CutError::new(
                error_codes::NO_UI_CLIENT,
                "UI client disconnected mid-request",
                "the WS connection closed before answering",
            )),
            Err(_) => {
                self.inner
                    .lock()
                    .expect("ui bridge lock")
                    .pending
                    .remove(&request_id);
                Err(CutError::new(
                    error_codes::JOB_FAILED,
                    "UI client did not answer in time",
                    format!(
                        "no {response_type}{} for request {request_id} within {}ms",
                        verb.as_deref()
                            .map(|name| format!(" for {name}"))
                            .unwrap_or_default(),
                        timeout.as_millis()
                    ),
                )
                .with_suggested_action(
                    "check that the Cut window is responsive and connected, then retry",
                ))
            }
        }
    }

    /// Resolve only when client id, request id, frame type and (for commands)
    /// verb all match. Unknown/stale/spoofed replies are ignored and the real
    /// waiter remains live.
    pub fn resolve(&self, client_id: u64, value: Value) -> bool {
        let Some(request_id) = value.get("request_id").and_then(Value::as_u64) else {
            return false;
        };
        let response_type = value.get("type").and_then(Value::as_str);
        let response_verb = value.get("verb").and_then(Value::as_str);
        let mut g = self.inner.lock().expect("ui bridge lock");
        let matches = g.pending.get(&request_id).is_some_and(|pending| {
            pending.client_id == client_id
                && response_type == Some(pending.response_type)
                && pending.verb.as_deref() == response_verb
        });
        if !matches {
            return false;
        }
        let pending = g
            .pending
            .remove(&request_id)
            .expect("matched pending request");
        let _ = pending.sender.send(value);
        true
    }
}

/// Shared actionable "no UI connected" error.
fn no_ui_client() -> CutError {
    CutError::new(
        error_codes::NO_UI_CLIENT,
        "no UI client is connected",
        "ui relay needs a browser tab running the app, connected over /api/events",
    )
    .with_suggested_action(
        "call system.doctor to read the live loopback address, then open it in a browser (or use render.frame for composed pixels)",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// No client → actionable no_ui_client (the public contract-required error path).
    #[tokio::test]
    async fn request_without_client_errors() {
        let b = UiBridge::default();
        let e = b
            .request(json!({"type":"screenshot_request"}))
            .await
            .unwrap_err();
        assert_eq!(e.code, "no_ui_client");
        let action = e
            .suggested_action
            .expect("no-ui recovery must remain actionable");
        assert!(action.contains("system.doctor"));
        assert!(!action.contains("127.0.0.1:6161"));
    }

    /// Round-trip: register → request → resolve via the stamped request_id.
    #[tokio::test]
    async fn request_resolves() {
        let b = UiBridge::default();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let client_id = b.register(tx);
        let b2 = b.clone();
        let answerer = tokio::spawn(async move {
            let raw = rx.recv().await.unwrap();
            let msg: Value = serde_json::from_str(&raw).unwrap();
            let rid = msg["request_id"].as_u64().unwrap();
            assert!(b2.resolve(
                client_id,
                json!({"type":"screenshot_result","request_id":rid,"png_base64":"aGk="})
            ));
        });
        let reply = b
            .request(json!({"type":"screenshot_request"}))
            .await
            .unwrap();
        assert_eq!(reply["png_base64"], "aGk=");
        answerer.await.unwrap();
    }

    #[tokio::test]
    async fn command_requires_exact_client_type_and_verb() {
        let b = UiBridge::default();
        let (old_tx, _old_rx) = mpsc::unbounded_channel();
        let old_client = b.register(old_tx);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let client = b.register(tx);
        let b2 = b.clone();
        let answerer = tokio::spawn(async move {
            let raw = rx.recv().await.unwrap();
            let msg: Value = serde_json::from_str(&raw).unwrap();
            let rid = msg["request_id"].as_u64().unwrap();
            assert!(!b2.resolve(
                old_client,
                json!({"type":"ui_command_result","request_id":rid,"verb":"ui.open","applied":true})
            ));
            assert!(!b2.resolve(
                client,
                json!({"type":"screenshot_result","request_id":rid,"verb":"ui.open","applied":true})
            ));
            assert!(!b2.resolve(
                client,
                json!({"type":"ui_command_result","request_id":rid,"verb":"ui.select","applied":true})
            ));
            assert!(b2.resolve(
                client,
                json!({"type":"ui_command_result","request_id":rid,"verb":"ui.open","applied":true})
            ));
        });
        let reply = b
            .request(json!({"type":"ui_command","verb":"ui.open","args":{"panel":"timeline"}}))
            .await
            .unwrap();
        assert_eq!(reply["applied"], true);
        answerer.await.unwrap();
    }

    #[tokio::test]
    async fn disconnect_wakes_only_that_clients_pending_request() {
        let b = UiBridge::default();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let client = b.register(tx);
        let b2 = b.clone();
        let disconnect = tokio::spawn(async move {
            rx.recv().await.unwrap();
            b2.unregister(client);
        });
        let error = b
            .request(json!({"type":"ui_command","verb":"ui.open","args":{"panel":"timeline"}}))
            .await
            .unwrap_err();
        assert_eq!(error.code, error_codes::NO_UI_CLIENT);
        disconnect.await.unwrap();
    }

    #[tokio::test]
    async fn timeout_is_bounded_and_removes_pending_request() {
        let b = UiBridge::default();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let client = b.register(tx);
        let b2 = b.clone();
        let observer = tokio::spawn(async move {
            let raw = rx.recv().await.unwrap();
            let msg: Value = serde_json::from_str(&raw).unwrap();
            let rid = msg["request_id"].as_u64().unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            assert!(!b2.resolve(
                client,
                json!({"type":"ui_command_result","request_id":rid,"verb":"ui.open","applied":true})
            ));
        });
        let error = b
            .request_with_timeout(
                json!({"type":"ui_command","verb":"ui.open","args":{"panel":"timeline"}}),
                std::time::Duration::from_millis(10),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, error_codes::JOB_FAILED);
        observer.await.unwrap();
    }
}

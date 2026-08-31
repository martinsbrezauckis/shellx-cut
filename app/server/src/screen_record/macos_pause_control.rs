//! Public command bridge for the macOS pause-safe capture owner.
//!
//! A request remains pending until the native owner has sealed the matching
//! journal transition.  The map contains no paths or native identifiers and is
//! removed with the capture backend, so a later capture can never inherit a
//! stale control channel.

use crate::dispatch::parse_args;
use cut_core::{error_codes, CutError, VerbResult};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

const ACK_TIMEOUT: Duration = Duration::from_secs(35);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PauseAction {
    Pause,
    Resume,
}

impl PauseAction {
    fn name(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PauseReceipt {
    action: PauseAction,
    state: &'static str,
    logical_media_time_ms: u64,
}

impl PauseReceipt {
    pub(super) fn paused(logical_media_time_ms: u64) -> Self {
        Self {
            action: PauseAction::Pause,
            state: "paused",
            logical_media_time_ms,
        }
    }

    pub(super) fn resumed(logical_media_time_ms: u64) -> Self {
        Self {
            action: PauseAction::Resume,
            state: "recording",
            logical_media_time_ms,
        }
    }
}

pub(super) struct PauseRequest {
    action: PauseAction,
    response: SyncSender<Result<PauseReceipt, String>>,
    pending: Arc<AtomicBool>,
}

impl PauseRequest {
    pub(super) fn action(&self) -> PauseAction {
        self.action
    }

    pub(super) fn complete(self, result: Result<PauseReceipt, String>) {
        let _ = self.response.send(result);
        self.pending.store(false, Ordering::Release);
    }
}

struct ControlSlot {
    sender: mpsc::Sender<PauseRequest>,
    pending: Arc<AtomicBool>,
}

static ACTIVE_CONTROLS: OnceLock<Mutex<HashMap<String, Arc<ControlSlot>>>> = OnceLock::new();

fn controls() -> &'static Mutex<HashMap<String, Arc<ControlSlot>>> {
    ACTIVE_CONTROLS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The capture backend owns this registration for its complete native lifetime.
/// Dropping it makes later public commands fail closed rather than target a
/// completed journal.
pub(super) struct PauseControlRegistration {
    capture_id: String,
    slot: Arc<ControlSlot>,
    receiver: Mutex<Receiver<PauseRequest>>,
}

pub(super) fn register(capture_id: &str) -> Result<PauseControlRegistration, CutError> {
    let (sender, receiver) = mpsc::channel();
    let slot = Arc::new(ControlSlot {
        sender,
        pending: Arc::new(AtomicBool::new(false)),
    });
    let mut active = controls()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if active.contains_key(capture_id) {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "pause controls already own this capture",
            "a capture id may have only one native pause owner",
        ));
    }
    active.insert(capture_id.to_owned(), slot.clone());
    Ok(PauseControlRegistration {
        capture_id: capture_id.to_owned(),
        slot,
        receiver: Mutex::new(receiver),
    })
}

impl PauseControlRegistration {
    pub(super) fn next_request(&self) -> Option<PauseRequest> {
        self.receiver
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .try_recv()
            .ok()
    }

    pub(super) fn reject_queued(&self, reason: &str) {
        while let Some(request) = self.next_request() {
            request.complete(Err(reason.to_owned()));
        }
    }
}

impl Drop for PauseControlRegistration {
    fn drop(&mut self) {
        let mut active = controls()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if active
            .get(&self.capture_id)
            .is_some_and(|current| Arc::ptr_eq(current, &self.slot))
        {
            active.remove(&self.capture_id);
        }
    }
}

pub(super) async fn pause_handler(args: Value) -> Result<VerbResult, CutError> {
    handle(args, PauseAction::Pause).await
}

pub(super) async fn resume_handler(args: Value) -> Result<VerbResult, CutError> {
    handle(args, PauseAction::Resume).await
}

async fn handle(args: Value, action: PauseAction) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        capture_id: String,
    }

    let args: Args = parse_args(args)?;
    super::recovery::validate_capture_id(&args.capture_id)?;
    if !cfg!(target_os = "macos") {
        return Err(platform_unavailable());
    }
    let capture_id = args.capture_id.clone();
    let receipt = tokio::task::spawn_blocking(move || request(&capture_id, action))
        .await
        .map_err(|error| {
            CutError::new(
                error_codes::IO,
                "Pause control worker failed",
                error.to_string(),
            )
        })??;
    Ok(VerbResult::ok(json!({
        "capture_id": args.capture_id,
        "action": receipt.action.name(),
        "saved": true,
        "state": receipt.state,
        "logical_media_time_ms": receipt.logical_media_time_ms,
    })))
}

fn request(capture_id: &str, action: PauseAction) -> Result<PauseReceipt, CutError> {
    let slot = controls()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(capture_id)
        .cloned()
        .ok_or_else(|| {
            CutError::new(
                error_codes::CONFLICT,
                "Pause controls are not active for this capture",
                "Pause and resume are available only while this exact macOS pause recording remains live",
            )
        })?;
    if slot
        .pending
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "A pause transition is already awaiting durable acknowledgement",
            "wait for the current Pause or Resume action to finish",
        ));
    }
    let (response, receipt) = mpsc::sync_channel(1);
    let request = PauseRequest {
        action,
        response,
        pending: slot.pending.clone(),
    };
    if slot.sender.send(request).is_err() {
        slot.pending.store(false, Ordering::Release);
        return Err(CutError::new(
            error_codes::CONFLICT,
            "Pause controls are no longer active for this capture",
            "the recording ended before the requested transition could begin",
        ));
    }
    match receipt.recv_timeout(ACK_TIMEOUT) {
        Ok(Ok(receipt)) => Ok(receipt),
        Ok(Err(reason)) => Err(CutError::new(
            error_codes::CONFLICT,
            "Pause transition was not durably acknowledged",
            reason,
        )),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(CutError::new(
            error_codes::CONFLICT,
            "Pause transition did not reach a durable acknowledgement in time",
            "the native owner is still resolving the requested boundary; do not infer a new state",
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(CutError::new(
            error_codes::CONFLICT,
            "Pause controls ended before acknowledgement",
            "the recording ended or the native owner failed before it sealed the requested transition",
        )),
    }
}

fn platform_unavailable() -> CutError {
    CutError::new(
        error_codes::NOT_FOUND,
        "Pause and resume are available only on macOS",
        "this build has no public pause-safe capture owner on the current platform",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_removes_the_exact_control_on_drop() {
        let capture_id = "cap_pause_control_test";
        let registration = register(capture_id).unwrap();
        assert!(controls().lock().unwrap().contains_key(capture_id));
        drop(registration);
        assert!(!controls().lock().unwrap().contains_key(capture_id));
    }

    #[test]
    fn duplicate_registration_cannot_replace_the_live_owner() {
        let capture_id = "cap_pause_control_conflict_test";
        let registration = register(capture_id).unwrap();
        let error = match register(capture_id) {
            Ok(_) => panic!("duplicate pause control registration unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error.code, error_codes::CONFLICT);
        let installed = controls().lock().unwrap().get(capture_id).cloned().unwrap();
        assert!(Arc::ptr_eq(&installed, &registration.slot));
    }

    #[test]
    fn one_request_stays_pending_until_a_durable_receipt() {
        let capture_id = "cap_pause_pending_test";
        let registration = register(capture_id).unwrap();
        let sender = std::thread::spawn(move || request(capture_id, PauseAction::Pause));
        let request = loop {
            if let Some(request) = registration.next_request() {
                break request;
            }
            std::thread::yield_now();
        };
        assert_eq!(request.action(), PauseAction::Pause);
        request.complete(Ok(PauseReceipt::paused(42)));
        let receipt = sender.join().unwrap().unwrap();
        assert_eq!(receipt.logical_media_time_ms, 42);
        assert_eq!(receipt.state, "paused");
    }
}

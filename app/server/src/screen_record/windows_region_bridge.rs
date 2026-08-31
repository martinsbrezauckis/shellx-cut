//! Authenticated Windows foreground Region admission for the spawned desktop.
//!
//! The private Win32 overlay remains owned by the exact Tauri main HWND. This
//! child-side route accepts only that shell's fresh secret/epoch/PID/correlation
//! handoff, turns its physical DisplayConfig target into an opaque identity,
//! then burns the one-use ticket while re-enumerating the same topology. It is
//! not a verb or a public capability; the consumed crop can enter only the
//! ordinary recorder reservation and GPU-only WGC Region path.

use super::region_selection::{
    self, NativeMonitorIdentity, NativeRegionCrop, NativeWindowsTopologySnapshot,
    RegionSelectionTicket, RegionSelectionValue,
};
use super::windows_region_parent::current_parent_pid;
use super::windows_region_start::{self, PrivateWindowsRegionStart};
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use axum::{Json, Router};
use cut_core::{error_codes, CutError, VerbResult};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const ENV_BRIDGE_SECRET: &str = "SHELLX_CUT_REGION_BRIDGE_SECRET";
const ENV_DESKTOP_EPOCH: &str = "SHELLX_CUT_REGION_DESKTOP_EPOCH";
const ENV_CUTD_EPOCH: &str = "SHELLX_CUT_REGION_CUTD_EPOCH";
const ENV_DESKTOP_PID: &str = "SHELLX_CUT_REGION_DESKTOP_PID";

const BRIDGE_SECRET_HEADER: &str = "x-shellx-cut-region-bridge";
const DESKTOP_EPOCH_HEADER: &str = "x-shellx-cut-desktop-epoch";
const CUTD_EPOCH_HEADER: &str = "x-shellx-cut-cutd-epoch";
const CUTD_PID_HEADER: &str = "x-shellx-cut-cutd-pid";
const REQUEST_PREFIX: &str = "region_request_";
const CORRELATION_CAPACITY: usize = 32;
const CORRELATION_TTL: Duration = Duration::from_secs(90);

static BRIDGE_CONFIGURATION: OnceLock<Option<BridgeConfiguration>> = OnceLock::new();

struct BridgeConfiguration {
    secret: String,
    desktop_epoch: String,
    cutd_epoch: String,
    desktop_pid: u32,
    correlations: Mutex<CorrelationRegistry>,
}

impl BridgeConfiguration {
    fn from_child_environment() -> Option<Self> {
        let secret = read_session_value(ENV_BRIDGE_SECRET, "region_bridge_")?;
        let desktop_epoch = read_session_value(ENV_DESKTOP_EPOCH, "desktop_epoch_")?;
        let cutd_epoch = read_session_value(ENV_CUTD_EPOCH, "cutd_epoch_")?;
        let desktop_pid = std::env::var(ENV_DESKTOP_PID).ok()?.parse::<u32>().ok()?;
        (desktop_pid != 0 && current_parent_pid() == Some(desktop_pid)).then_some(Self {
            secret,
            desktop_epoch,
            cutd_epoch,
            desktop_pid,
            correlations: Mutex::new(CorrelationRegistry::default()),
        })
    }

    fn parent_is_expected(&self) -> bool {
        current_parent_pid() == Some(self.desktop_pid)
    }

    fn authorize(&self, headers: &HeaderMap, request: &BridgeRequest) -> Result<(), CutError> {
        let header = |name| headers.get(name).and_then(|value| value.to_str().ok());
        let child_pid = header(CUTD_PID_HEADER)
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|pid| *pid == std::process::id());
        let valid = self.parent_is_expected()
            && child_pid.is_some()
            && header(BRIDGE_SECRET_HEADER)
                .is_some_and(|value| constant_time_eq(value, &self.secret))
            && header(DESKTOP_EPOCH_HEADER)
                .is_some_and(|value| constant_time_eq(value, &self.desktop_epoch))
            && header(CUTD_EPOCH_HEADER)
                .is_some_and(|value| constant_time_eq(value, &self.cutd_epoch))
            && constant_time_eq(&request.desktop_epoch, &self.desktop_epoch)
            && constant_time_eq(&request.cutd_epoch, &self.cutd_epoch);
        valid.then_some(()).ok_or_else(bridge_refused)
    }

    fn reserve_correlation(&self, request_id: &str) -> Result<(), CutError> {
        self.correlations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .reserve(request_id)
            .map_err(|_| bridge_refused())
    }
}

/// Load, validate, and scrub the desktop handoff before Tokio starts. Headless
/// and independently started cutd instances never install this route.
pub(crate) fn initialize_from_child_environment(allow_foreground_bridge: bool) {
    let configuration = allow_foreground_bridge
        .then(BridgeConfiguration::from_child_environment)
        .flatten();
    clear_child_environment();
    let _ = BRIDGE_CONFIGURATION.set(configuration);
}

fn clear_child_environment() {
    for name in [
        ENV_BRIDGE_SECRET,
        ENV_DESKTOP_EPOCH,
        ENV_CUTD_EPOCH,
        ENV_DESKTOP_PID,
    ] {
        // SAFETY: startup invokes this before any runtime worker or child can
        // observe inherited bridge material.
        unsafe { std::env::remove_var(name) };
    }
}

#[derive(Default)]
struct CorrelationRegistry {
    entries: HashMap<String, Instant>,
}

impl CorrelationRegistry {
    fn reserve(&mut self, request_id: &str) -> Result<(), ()> {
        if !well_formed_session_value(request_id, REQUEST_PREFIX) {
            return Err(());
        }
        let now = Instant::now();
        self.entries.retain(|_, expires_at| *expires_at > now);
        if self.entries.contains_key(request_id) || self.entries.len() >= CORRELATION_CAPACITY {
            return Err(());
        }
        self.entries
            .insert(request_id.to_owned(), now + CORRELATION_TTL);
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BridgeRequest {
    request_id: String,
    desktop_epoch: String,
    cutd_epoch: String,
    selection: WireRegionSelection,
    start: WireStart,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRegionSelection {
    source_gdi: String,
    target_path: String,
    topology_digest: [u8; 32],
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    parent_width: u32,
    parent_height: u32,
}

/// This intentionally mirrors only ordinary non-target start choices. Raw
/// geometry, monitor identity, ticket, path, and rationale cannot be supplied
/// by browser content.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireStart {
    duration_ms: Option<u64>,
    fps: Option<f64>,
    quality: Option<record_core::CaptureQualityRequest>,
    #[serde(default)]
    audio: bool,
    #[serde(default)]
    system_audio: bool,
    studio: Option<Value>,
    #[serde(default)]
    keys: bool,
}

/// Install only for the exact foreground child configuration admitted during
/// synchronous startup. This route stays absent from headless, external, MCP,
/// and descendant cutd processes.
pub(crate) fn install_route(router: Router<AppState>) -> Router<AppState> {
    if configuration().is_some() {
        router.route(
            "/private/foreground-region-start",
            post(start_from_foreground_desktop),
        )
    } else {
        router
    }
}

async fn start_from_foreground_desktop(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Json<VerbResult> {
    let Some(configuration) = configuration() else {
        return Json(VerbResult::err(bridge_refused()));
    };
    let request = match serde_json::from_slice::<BridgeRequest>(&body) {
        Ok(request) => request,
        Err(_) => return Json(VerbResult::err(bridge_refused())),
    };
    if let Err(error) = configuration.authorize(&headers, &request) {
        return Json(VerbResult::err(error));
    }
    if let Err(error) = configuration.reserve_correlation(&request.request_id) {
        return Json(VerbResult::err(error));
    }
    let ticket = match issue_ticket(request.selection) {
        Ok(ticket) => ticket,
        Err(error) => return Json(VerbResult::err(error)),
    };
    let start = PrivateWindowsRegionStart {
        duration_ms: request.start.duration_ms,
        fps: request.start.fps.unwrap_or(30.0),
        quality: request.start.quality,
        audio: request.start.audio,
        system_audio: request.start.system_audio,
        studio: request.start.studio,
        keys: request.start.keys,
        ticket,
    };
    Json(
        windows_region_start::start_from_foreground_ticket(&state, start)
            .await
            .unwrap_or_else(VerbResult::err),
    )
}

/// Replace the raw physical target path with the same opaque identity used by
/// WGC before storing the one-use ticket. Raw source/target data remains only
/// in that ticket until the immediate native revalidation at consumption.
fn issue_ticket(selection: WireRegionSelection) -> Result<RegionSelectionTicket, CutError> {
    let topology = NativeWindowsTopologySnapshot::new(
        selection.source_gdi,
        selection.target_path,
        selection.topology_digest,
    )
    .ok_or_else(bridge_refused)?;
    let monitor_id =
        record_capture::private_windows_monitor_identity_from_target_path(topology.target_path())
            .ok_or_else(bridge_refused)?;
    let monitor = NativeMonitorIdentity::new(monitor_id).ok_or_else(bridge_refused)?;
    let crop = NativeRegionCrop::new(
        selection.left,
        selection.top,
        selection.width,
        selection.height,
        selection.parent_width,
        selection.parent_height,
    )
    .ok_or_else(bridge_refused)?;
    region_selection::issue(RegionSelectionValue::with_windows_topology(
        monitor, crop, topology,
    ))
    .map_err(|_| bridge_refused())
}

/// Atomically burn the ticket before any ordinary settings, Doctor, snapshot,
/// marker, directory, or capture reservation path. Its callback repeats both
/// exact opaque identity resolution and the full original DisplayConfig digest.
pub(super) fn consume_ticket(
    ticket: &RegionSelectionTicket,
) -> Result<(String, record_capture::CaptureRegion), CutError> {
    let consumed = region_selection::consume(ticket.as_str(), |selection| {
        let Some(topology) = selection.windows_topology() else {
            return false;
        };
        let Some(crop) = capture_parameters(selection.crop()) else {
            return false;
        };
        record_capture::private_windows_region_selection_is_current(
            topology.source_gdi(),
            topology.target_path(),
            topology.fingerprint().bytes(),
            selection.monitor_identity().as_str(),
            crop,
        )
    })
    .map_err(|_| invalidated_selection())?;
    let crop = capture_parameters(consumed.crop()).ok_or_else(invalidated_selection)?;
    Ok((consumed.monitor_identity().as_str().to_owned(), crop))
}

fn capture_parameters(selection: NativeRegionCrop) -> Option<record_capture::CaptureRegion> {
    let (left, top, width, height, parent_width, parent_height) = selection.native_parts();
    record_capture::CaptureRegion::new(left, top, width, height, parent_width, parent_height)
}

fn configuration() -> Option<&'static BridgeConfiguration> {
    BRIDGE_CONFIGURATION.get().and_then(Option::as_ref)
}

fn read_session_value(name: &str, prefix: &str) -> Option<String> {
    let value = std::env::var(name).ok()?;
    well_formed_session_value(&value, prefix).then_some(value)
}

fn well_formed_session_value(value: &str, prefix: &str) -> bool {
    let hex = value.strip_prefix(prefix);
    hex.is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn bridge_refused() -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "Windows Region foreground admission was refused",
        "the request was not bound to this foreground desktop and spawned cutd session",
    )
    .with_suggested_action("choose the region again from the foreground ShellX Cut desktop")
}

fn invalidated_selection() -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "Windows Region selection is no longer valid",
        "the one-use selection expired, was replayed, or no longer matches the current physical display and topology",
    )
    .with_suggested_action("choose the region again from the foreground ShellX Cut desktop")
}

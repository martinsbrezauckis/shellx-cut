//! Authenticated macOS foreground Region admission for the spawned desktop.
//!
//! The visual AppKit picker lives in the Tauri foreground process; `cutd` owns
//! the project and the only capture reservation. This private route exists
//! solely while the desktop has spawned this exact child with fresh secret and
//! epoch material. It is never installed for headless or adopted cutd, has no
//! schema/verb/UI representation, accepts no reusable ticket, and burns the
//! server-private selection before ordinary start reservation can begin.

use super::macos_region_start::{self, PrivateMacosRegionStart};
use super::region_selection::{
    self, NativeMonitorIdentity, NativeRegionCrop, RegionSelectionTicket, RegionSelectionValue,
};
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
        let secret = read_session_value(ENV_BRIDGE_SECRET, "region_bridge_");
        let desktop_epoch = read_session_value(ENV_DESKTOP_EPOCH, "desktop_epoch_");
        let cutd_epoch = read_session_value(ENV_CUTD_EPOCH, "cutd_epoch_");
        let desktop_pid = std::env::var(ENV_DESKTOP_PID)
            .ok()
            .and_then(|value| value.parse::<u32>().ok());
        let secret = secret?;
        let desktop_epoch = desktop_epoch?;
        let cutd_epoch = cutd_epoch?;
        let desktop_pid = desktop_pid?;
        (desktop_pid != 0 && current_parent_pid() == desktop_pid).then_some(Self {
            secret,
            desktop_epoch,
            cutd_epoch,
            desktop_pid,
            correlations: Mutex::new(CorrelationRegistry::default()),
        })
    }

    fn parent_is_expected(&self) -> bool {
        current_parent_pid() == self.desktop_pid
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

/// Load the desktop handoff only for a non-headless `cutd serve` invocation.
/// This must run synchronously from `main` before Tokio exists: the values are
/// then retained only in this process's private configuration and scrubbed
/// before any runtime worker or sidecar could inherit them.
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
        // SAFETY: `initialize_from_child_environment` is called synchronously
        // by `main` before it constructs Tokio or starts any worker. We only
        // remove this process's private child-spawn environment values.
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
    display_id: u32,
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    parent_width: u32,
    parent_height: u32,
}

/// This is intentionally narrower than `screen_record.start`: Region owns the
/// target itself, so monitor/window/ticket/path/rationale cannot appear here.
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

/// Install the route only when synchronous startup admitted a foreground
/// desktop with well-formed fresh session material. Headless `cutd serve`, an
/// ordinary daemon, MCP process, inherited child, or external/adopted daemon
/// has no route.
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
    let start = PrivateMacosRegionStart {
        duration_ms: request.start.duration_ms,
        fps: request.start.fps.unwrap_or(30.0),
        quality: request.start.quality,
        audio: request.start.audio,
        system_audio: request.start.system_audio,
        keys: request.start.keys,
        studio: request.start.studio,
        ticket,
    };
    Json(
        macos_region_start::start_from_foreground_ticket(&state, start)
            .await
            .unwrap_or_else(VerbResult::err),
    )
}

/// Create the server-private ticket only after the authenticated foreground
/// request supplies a complete native-pixel crop. The opaque display identity
/// replaces the transient CoreGraphics number before the ticket is stored.
fn issue_ticket(selection: WireRegionSelection) -> Result<RegionSelectionTicket, CutError> {
    let monitor_id =
        record_capture::private_macos_monitor_identity_from_display_id(selection.display_id)
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
    region_selection::issue(RegionSelectionValue::new(monitor, crop)).map_err(|_| bridge_refused())
}

/// Burn the ticket before normal validation, snapshot, Doctor, or reservation
/// work. `RegionSelectionRegistry::consume` removes it atomically before its
/// callback, and this callback performs the exact fresh display/scale/crop
/// check; neither a replay nor a topology change reaches capture reservation.
pub(super) fn consume_ticket(
    ticket: &RegionSelectionTicket,
) -> Result<(String, record_capture::CaptureRegion), CutError> {
    let consumed = region_selection::consume(ticket.as_str(), |selection| {
        let Some((monitor_id, crop)) =
            capture_parameters(selection.monitor_identity(), selection.crop())
        else {
            return false;
        };
        record_capture::private_macos_region_selection_is_current(&monitor_id, crop)
    })
    .map_err(|_| invalidated_selection())?;
    capture_parameters(consumed.monitor_identity(), consumed.crop())
        .ok_or_else(invalidated_selection)
}

fn capture_parameters(
    monitor: &NativeMonitorIdentity,
    selection: NativeRegionCrop,
) -> Option<(String, record_capture::CaptureRegion)> {
    let (left, top, width, height, parent_width, parent_height) = selection.native_parts();
    let crop =
        record_capture::CaptureRegion::new(left, top, width, height, parent_width, parent_height)?;
    Some((monitor.as_str().to_owned(), crop))
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

fn current_parent_pid() -> u32 {
    // SAFETY: getppid has no preconditions and returns the current process's
    // parent only; it neither allocates nor exposes credentials.
    unsafe { libc::getppid() as u32 }
}

fn bridge_refused() -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "macOS Region foreground admission was refused",
        "the request was not bound to this foreground desktop and spawned cutd session",
    )
    .with_suggested_action("choose the region again from the foreground ShellX Cut desktop")
}

fn invalidated_selection() -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "macOS Region selection is no longer valid",
        "the one-use selection expired, was replayed, or no longer matches the current display scale and crop",
    )
    .with_suggested_action("choose the region again from the foreground ShellX Cut desktop")
}

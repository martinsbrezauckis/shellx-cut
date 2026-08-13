//! Desktop-shell update-state service: quiet launch + periodic release checks,
//! surfaced to the engine-served UI instead of a startup modal.
//!
//! ROLE
//!   Replaces the original launch-only one-shot (which popped a native dialog
//!   the moment an update was found — a modal ambush). This service:
//!     * performs the automatic checks (at launch AND every 6 hours while the
//!       app stays open) when the persisted Settings preference allows them;
//!     * holds one honest snapshot `{status, version?, current, checked_at,
//!       error?, checking, installing, supported}`;
//!     * broadcasts every snapshot change as the `cut:update-state` Tauri
//!       event (the remote engine-served origin already holds
//!       `core:event:allow-listen`);
//!     * exposes three narrow commands to the validated engine origin via the
//!       `allow-update-state` permission: `get_update_state` (read),
//!       `update_check_now` (manual check — deliberately ignores the automatic
//!       preference so the Settings button still works when auto is off), and
//!       `update_install_now` (native confirm → download+install → restart —
//!       install authority never leaves the shell; the webview can only ask).
//!
//! WHY SHELL-SIDE
//!   The Cut UI is served by cutd on a remote loopback origin where Tauri 2
//!   denies plugin IPC, so the webview cannot drive tauri-plugin-updater
//!   itself. The whole updater flow therefore lives here; the UI only reads
//!   state and requests actions over the app-command bridge (lib/tauri.ts).
//!
//! PLATFORM HONESTY
//!   Linux (deb/rpm) never uses the in-app updater — the release feed carries
//!   only windows-x86_64 + darwin-aarch64. On Linux the snapshot reports
//!   `status: "unsupported"` and no
//!   network request is ever made; the UI shows the honest explanation instead
//!   of dead buttons.
//!
//! SECURITY
//!   The updater plugin verifies the minisign signature against the configured
//!   updater public key before installing, the version comparator in lib.rs
//!   requires release URLs to be bound to the manifest version, and this
//!   service verifies a second signed version/platform/byte identity after
//!   download but before either native install path — all apply unchanged to
//!   every check this service performs.
//!   `SHELLX_CUT_UPDATE_FEED_URL` (point a test install at a
//!   staged latest.json) can move the FEED but cannot bypass either check, and
//!   downgrades are rejected by the `release.version > installed` comparator.
//!
//! Callers: lib.rs (setup wiring + invoke_handler). Deps: update_settings
//! (the persisted automatic-check preference), tauri-plugin-updater,
//! tauri-plugin-dialog (install confirmation).

// Linux builds compile the pure state machine + the honest unsupported paths
// only (no updater flow — deb/rpm packaging), so several items are exercised
// exclusively by the non-Linux cfg and the unit tests. Silence the resulting
// Linux-only dead-code lints in one visible place instead of per item.
#![cfg_attr(target_os = "linux", allow(dead_code))]

use std::sync::Mutex;
use std::time::Duration;

use base64::Engine as _;
use minisign_verify::PublicKey;
use serde_json::json;
use sha2::{Digest, Sha256};
use tauri::{Emitter, Manager};

/// How often the periodic re-check runs while the app stays open. Chosen so a
/// long editing session learns about a new release the same day without
/// polling GitHub aggressively (≤4 extra requests per 24h session).
pub(crate) const AUTO_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Tauri event carrying the snapshot JSON to the webview on every change.
pub(crate) const EVENT_NAME: &str = "cut:update-state";

/// QA/staging override: replace the release feed URL for THIS process only.
/// Package and artifact-identity signature verification, the version-bound URL
/// policy, and the no-downgrade comparator all still apply — this can point at
/// a staged feed, not around the trust chain. Release builds additionally
/// require an https URL (tauri-plugin-updater enforces the scheme).
pub(crate) const ENV_UPDATE_FEED_URL: &str = "SHELLX_CUT_UPDATE_FEED_URL";

/// JSON schema tag on every snapshot the bridge returns/broadcasts.
const SNAPSHOT_SCHEMA: &str = "shellx-cut/update-state/1";

// ─────────────────────────────────────────────────────────────────────────────
// Pure snapshot state machine (no Tauri types — unit-tested below).
// ─────────────────────────────────────────────────────────────────────────────

/// Result classification of one completed release-feed check.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CheckOutcome {
    /// A newer signed release exists; the payload is its version string.
    Available(String),
    /// The feed answered and the installed build is current.
    UpToDate,
    /// The check could not complete (network, feed shape, …) — honest text.
    Failed(String),
}

/// Lifecycle status of the update surface. Serialized lowercase into the
/// snapshot JSON (`idle | none | available | error | unsupported`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Status {
    /// No check has completed this session (auto off, or still starting).
    Idle,
    /// Last completed check: already on the latest release.
    UpToDate,
    /// Last completed check found a newer release (see `version`).
    Available,
    /// Last completed check failed and no earlier check had found an update.
    Error,
    /// Platform never checks (Linux deb/rpm packaging).
    Unsupported,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Idle => "idle",
            Status::UpToDate => "none",
            Status::Available => "available",
            Status::Error => "error",
            Status::Unsupported => "unsupported",
        }
    }
}

/// The one honest record of the updater surface, shared verbatim with the UI.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Snapshot {
    pub(crate) status: Status,
    /// Newer release version — `Some` exactly while `status == Available`.
    pub(crate) version: Option<String>,
    /// Installed app version (constant per process).
    pub(crate) current: String,
    /// Unix ms of the last COMPLETED check (success or failure).
    pub(crate) checked_at: Option<u64>,
    /// Honest failure text of the most recent FAILED check (check or install).
    /// May coexist with `Available` — see `apply_outcome`.
    pub(crate) error: Option<String>,
    /// A check is in flight right now (UI shows progress, second checks are
    /// coalesced instead of stacking network requests).
    pub(crate) checking: bool,
    /// An install is in flight right now (topbar button shows "Installing…").
    pub(crate) installing: bool,
    /// False on platforms whose packages update outside the app (Linux).
    pub(crate) supported: bool,
}

impl Snapshot {
    pub(crate) fn new(current: String, supported: bool) -> Self {
        Snapshot {
            status: if supported {
                Status::Idle
            } else {
                Status::Unsupported
            },
            version: None,
            current,
            checked_at: None,
            error: None,
            checking: false,
            installing: false,
            supported,
        }
    }

    /// Fold one completed check into the snapshot.
    ///
    /// State-transition rules:
    ///   * `Available(v)` → status `available`, version recorded, error cleared;
    ///   * `UpToDate`     → status `none`, any previously offered version cleared
    ///     (a pulled release must not leave a stale button);
    ///   * `Failed(e)`    → error recorded; BUT if an earlier check already
    ///     found an update, status stays `available` (the release still
    ///     exists — a transient network failure must not hide the button).
    ///
    /// Every completed check stamps `checked_at`.
    pub(crate) fn apply_outcome(&mut self, outcome: CheckOutcome, now_ms: u64) {
        self.checked_at = Some(now_ms);
        self.checking = false;
        match outcome {
            CheckOutcome::Available(version) => {
                self.status = Status::Available;
                self.version = Some(version);
                self.error = None;
            }
            CheckOutcome::UpToDate => {
                self.status = Status::UpToDate;
                self.version = None;
                self.error = None;
            }
            CheckOutcome::Failed(message) => {
                self.error = Some(message);
                if self.status != Status::Available {
                    self.status = Status::Error;
                    self.version = None;
                }
            }
        }
    }

    /// The bridge/event payload. One serializer so command replies and event
    /// broadcasts can never drift apart.
    pub(crate) fn to_json(&self) -> serde_json::Value {
        json!({
            "schema": SNAPSHOT_SCHEMA,
            "status": self.status.as_str(),
            "version": self.version,
            "current": self.current,
            "checked_at": self.checked_at,
            "error": self.error,
            "checking": self.checking,
            "installing": self.installing,
            "supported": self.supported,
        })
    }
}

/// Gate for the AUTOMATIC checks (launch + every 6 h). The manual
/// `update_check_now` command deliberately does NOT consult this: turning the
/// Settings toggle off stops all automatic network activity while the
/// About-panel button keeps working on explicit user request.
pub(crate) fn should_auto_check(preference_enabled: bool) -> bool {
    preference_enabled
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tauri service around the pure core.
// ─────────────────────────────────────────────────────────────────────────────

/// Managed state: the live snapshot plus (on updater platforms) the pending
/// `Update` handle from the most recent successful check, so "Install now"
/// installs exactly the release the user was shown.
pub(crate) struct UpdateService {
    snapshot: Mutex<Snapshot>,
    #[cfg(all(desktop, not(target_os = "linux")))]
    pending: Mutex<Option<tauri_plugin_updater::Update>>,
}

/// True on platforms whose installed app uses the in-app updater feed.
const PLATFORM_SUPPORTED: bool = cfg!(all(desktop, not(target_os = "linux")));

const ARTIFACT_IDENTITY_SCHEMA: &str = "shellx-cut/updater-artifact-identity@1";

/// An updater package's version cannot be inferred safely from its filename or
/// URL: release metadata is not part of Tauri's package-byte signature. The
/// release builder therefore signs this canonical record with the same updater
/// key. This is intentionally platform-neutral: it binds the identity before
/// either NSIS or the macOS archive replacement code sees the bytes.
fn updater_artifact_identity_claim(
    raw_json: &serde_json::Value,
    version: &str,
    platform: &str,
    bytes: &[u8],
) -> Result<(String, String), String> {
    let record = raw_json
        .get("shellx_cut_artifact_identities")
        .and_then(|identities| {
            (identities.get("schema").and_then(serde_json::Value::as_str)
                == Some(ARTIFACT_IDENTITY_SCHEMA))
            .then_some(identities)
        })
        .and_then(|identities| identities.get("platforms"))
        .and_then(|platforms| platforms.get(platform))
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            format!("update to {version} has no signed artifact identity for {platform}")
        })?;
    let identity_version = record
        .get("version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("update to {version} has an invalid artifact identity version"))?;
    if identity_version != version {
        return Err(format!(
            "update metadata advertises {version}, but its signed artifact identity is {identity_version}"
        ));
    }
    let identity_sha256 = record
        .get("sha256")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("update to {version} has no artifact identity SHA-256"))?;
    let actual_sha256 = format!("{:x}", Sha256::digest(bytes));
    if identity_sha256 != actual_sha256 {
        return Err(format!(
            "update to {version} bytes do not match their signed artifact identity"
        ));
    }
    let signature = record
        .get("signature")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("update to {version} has no artifact identity signature"))?;
    Ok((
        format!(
            "{ARTIFACT_IDENTITY_SCHEMA}\nversion={version}\nplatform={platform}\nsha256={actual_sha256}\n"
        ),
        signature.to_string(),
    ))
}

fn updater_public_key() -> Result<PublicKey, String> {
    // This is the same runtime trust key passed to the updater plugin. Do not
    // read the bundle configuration here: a deliberate future key-transition
    // release can keep a build-time key distinct from the key an installed
    // bridge app accepts.
    let encoded = crate::updater_key_transition::UPDATER_PUBLIC_KEY;
    let text = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|error| format!("decode configured updater public key failed: {error}"))?;
    let text = String::from_utf8(text)
        .map_err(|error| format!("configured updater public key is not UTF-8: {error}"))?;
    PublicKey::decode(&text)
        .map_err(|error| format!("parse configured updater public key failed: {error}"))
}

/// Requires a second signature which binds the release's advertised version,
/// selected platform, and raw package bytes. `Update::download` has already
/// verified the ordinary package signature when this runs; neither signature
/// alone is sufficient for a release-writer-without-signing-key replay.
fn verify_update_artifact_identity(
    update: &tauri_plugin_updater::Update,
    bytes: &[u8],
) -> Result<(), String> {
    // `Update::target` is the updater's OS selector (e.g. `windows`), while
    // static latest.json keys include the architecture (e.g.
    // `windows-x86_64`). Use the plugin's public target helper, which is the
    // same platform key it resolves from the static manifest.
    let platform = tauri_plugin_updater::target()
        .ok_or_else(|| "this updater platform has no supported identity key".to_string())?;
    let (identity, encoded_signature) =
        updater_artifact_identity_claim(&update.raw_json, &update.version, &platform, bytes)?;
    let signature = crate::updater_signature::parse_tauri_updater_signature(
        &encoded_signature,
        "update artifact identity signature",
    )?;
    updater_public_key()?
        .verify(identity.as_bytes(), &signature, true)
        .map_err(|error| format!("update artifact identity signature verification failed: {error}"))
}

impl UpdateService {
    pub(crate) fn new(current_version: String) -> Self {
        UpdateService {
            snapshot: Mutex::new(Snapshot::new(current_version, PLATFORM_SUPPORTED)),
            #[cfg(all(desktop, not(target_os = "linux")))]
            pending: Mutex::new(None),
        }
    }

    fn snapshot_json(&self) -> serde_json::Value {
        self.snapshot.lock().unwrap().to_json()
    }

    /// Mutate the snapshot under the lock, then broadcast the new state to the
    /// webview. Every state change goes through here so the UI can never miss
    /// a transition.
    fn update_and_broadcast(&self, app: &tauri::AppHandle, f: impl FnOnce(&mut Snapshot)) {
        let payload = {
            let mut snap = self.snapshot.lock().unwrap();
            f(&mut snap);
            snap.to_json()
        };
        // A failed emit (no window yet / shutting down) is benign — the UI
        // re-reads via get_update_state on mount.
        let _ = app.emit(EVENT_NAME, payload);
    }
}

/// Read the live update snapshot. Safe from the validated engine origin
/// (read-only; part of the `allow-update-state` permission).
#[tauri::command]
pub(crate) fn get_update_state(state: tauri::State<'_, UpdateService>) -> serde_json::Value {
    state.snapshot_json()
}

/// Manual "Check for updates" (Settings > About). Runs regardless of the
/// automatic-check preference — an explicit user request is its own consent.
/// Returns the post-check snapshot so the caller gets honest feedback even if
/// the broadcast event races the reply.
#[tauri::command]
pub(crate) async fn update_check_now(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    #[cfg(all(desktop, not(target_os = "linux")))]
    {
        Ok(perform_check(&app).await.to_json())
    }
    #[cfg(not(all(desktop, not(target_os = "linux"))))]
    {
        // Linux/deb+rpm (and any non-updater target): no network, honest state.
        let state = app.state::<UpdateService>();
        Ok(state.snapshot_json())
    }
}

/// Install the pending update: native confirm dialog → signature-verified
/// download+install → restart. Returns `{ok:false, cancelled:true}` when the
/// user declines, `{ok:false, error}` on failure (also recorded in the
/// snapshot), and never returns on success (the app restarts).
#[tauri::command]
pub(crate) async fn update_install_now(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    #[cfg(all(desktop, not(target_os = "linux")))]
    {
        install_pending(&app).await
    }
    #[cfg(not(all(desktop, not(target_os = "linux"))))]
    {
        let _ = &app;
        Err(
            "Linux builds update through deb/rpm packages — the in-app installer is not used"
                .into(),
        )
    }
}

/// The automatic check driver, spawned once from setup. Linux: one honest log
/// line, snapshot already reports `unsupported`, zero network activity.
#[cfg(all(desktop, target_os = "linux"))]
pub(crate) async fn run_automatic_checks(_app: tauri::AppHandle) {
    eprintln!(
        "[shellx-cut] updater: Linux builds update through deb/rpm packages — launch update check skipped"
    );
}

/// The automatic check driver, spawned once from setup (Windows/macOS).
/// Preference-gated launch check, then a 6-hour loop that RE-READS the
/// preference each tick — so turning the Settings toggle off stops all
/// automatic checks immediately (not just from the next launch), and turning
/// it back on resumes them at the next tick. The manual command never routes
/// through here.
#[cfg(all(desktop, not(target_os = "linux")))]
pub(crate) async fn run_automatic_checks(app: tauri::AppHandle) {
    if should_auto_check(crate::update_settings::check_on_launch(&app)) {
        perform_check(&app).await;
    } else {
        eprintln!("[shellx-cut] launch update check disabled in Settings");
    }
    loop {
        tokio::time::sleep(AUTO_CHECK_INTERVAL).await;
        if should_auto_check(crate::update_settings::check_on_launch(&app)) {
            perform_check(&app).await;
        } else {
            eprintln!("[shellx-cut] periodic update check disabled in Settings — skipped");
        }
    }
}

/// Build the updater (honoring the QA feed override) — kept separate so both
/// the checker and a future flow construct it identically.
#[cfg(all(desktop, not(target_os = "linux")))]
fn build_updater(app: &tauri::AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    use tauri_plugin_updater::UpdaterExt;
    let mut builder = app.updater_builder();
    if let Ok(feed) = std::env::var(ENV_UPDATE_FEED_URL) {
        if !feed.is_empty() {
            let url: tauri::Url = feed
                .parse()
                .map_err(|e| format!("{ENV_UPDATE_FEED_URL} is not a valid URL: {e}"))?;
            eprintln!(
                "[shellx-cut] updater: using staged release feed {url} ({ENV_UPDATE_FEED_URL})"
            );
            builder = builder
                .endpoints(vec![url])
                .map_err(|e| format!("{ENV_UPDATE_FEED_URL} rejected: {e}"))?;
        }
    }
    builder
        .build()
        .map_err(|e| format!("updater unavailable: {e}"))
}

/// One release-feed check: coalesces concurrent callers, stores the pending
/// `Update` handle on success, folds the outcome into the snapshot, and
/// broadcasts every transition. Returns the post-check snapshot.
#[cfg(all(desktop, not(target_os = "linux")))]
async fn perform_check(app: &tauri::AppHandle) -> Snapshot {
    let state = app.state::<UpdateService>();
    // Coalesce: if a check is already in flight, report current state instead
    // of stacking a second network request.
    {
        let snap = state.snapshot.lock().unwrap();
        if snap.checking {
            return snap.clone();
        }
    }
    state.update_and_broadcast(app, |snap| snap.checking = true);

    let outcome = match build_updater(app) {
        Ok(updater) => match updater.check().await {
            Ok(Some(update)) => {
                let version = update.version.clone();
                *state.pending.lock().unwrap() = Some(update);
                CheckOutcome::Available(version)
            }
            Ok(None) => {
                *state.pending.lock().unwrap() = None;
                CheckOutcome::UpToDate
            }
            Err(e) => CheckOutcome::Failed(format!("update check failed: {e}")),
        },
        Err(reason) => CheckOutcome::Failed(reason),
    };
    if let CheckOutcome::Failed(reason) = &outcome {
        eprintln!("[shellx-cut] updater: {reason}");
    }

    let now = now_ms();
    let mut result = None;
    state.update_and_broadcast(app, |snap| {
        snap.apply_outcome(outcome, now);
        result = Some(snap.clone());
    });
    result.expect("update_and_broadcast runs the closure")
}

/// The install flow behind `update_install_now`. Native confirm first (the
/// only blocking dialog in the whole update path — it answers an explicit
/// button click, never a startup ambush), then the signature-verified
/// download+install, then restart.
#[cfg(all(desktop, not(target_os = "linux")))]
async fn install_pending(app: &tauri::AppHandle) -> Result<serde_json::Value, String> {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

    // Scoped reads: no Mutex guard may live across an await (Send + deadlock).
    let (already_installing, pending) = {
        let state = app.state::<UpdateService>();
        let installing = state.snapshot.lock().unwrap().installing;
        let pending = state.pending.lock().unwrap().clone();
        (installing, pending)
    };
    if already_installing {
        return Ok(json!({ "ok": false, "error": "an install is already running" }));
    }
    // Use the pending handle from the check that surfaced the button; if it is
    // somehow gone (e.g. state cleared), re-check rather than failing blind.
    let update = match pending {
        Some(update) => update,
        None => {
            let snap = perform_check(app).await;
            let retry = {
                let state = app.state::<UpdateService>();
                let retry = state.pending.lock().unwrap().clone();
                retry
            };
            match retry {
                Some(update) => update,
                None => {
                    return Ok(json!({
                        "ok": false,
                        "error": snap.error.unwrap_or_else(|| "no update is available to install".into()),
                    }))
                }
            }
        }
    };
    let state = app.state::<UpdateService>();

    let version = update.version.clone();
    let current = update.current_version.clone();
    // Native confirm on user request — "Install & restart" ⇒ proceed.
    let install = app
        .dialog()
        .message(format!(
            "ShellX Cut {version} is available (you're on {current}).\n\nInstall it now? The app will restart."
        ))
        .title("Update available")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Install & restart".into(),
            "Later".into(),
        ))
        .blocking_show();
    if !install {
        return Ok(json!({ "ok": false, "cancelled": true }));
    }

    state.update_and_broadcast(app, |snap| {
        snap.installing = true;
        snap.error = None;
    });
    // Download and verify the signature while the editor remains usable. Only
    // after those bytes are trusted do we quiesce the owned engine and hand
    // them to the platform installer. On Windows the updater plugin exits via
    // `std::process::exit`, so normal Tauri Exit events never get a chance to
    // reap `cutd.exe`, which would leave a mixed-version runtime behind.
    let bytes = match update.download(|_chunk, _total| {}, || {}).await {
        Ok(bytes) => bytes,
        Err(e) => {
            let message = format!("Update to {version} could not be downloaded: {e}");
            eprintln!("[shellx-cut] updater: {message}");
            state.update_and_broadcast(app, |snap| {
                snap.installing = false;
                snap.error = Some(message.clone());
            });
            let _ = app
                .dialog()
                .message(format!("Update to {version} could not be downloaded:\n{e}"))
                .title("Update failed")
                .blocking_show();
            return Ok(json!({ "ok": false, "error": message }));
        }
    };

    // `download` above authenticates the raw package bytes. Bind those exact
    // bytes to the separately advertised release version before stopping the
    // Windows sidecar or giving either platform installer a chance to replace
    // the app. A copied old package + its valid old signature therefore fails
    // closed even when a release writer lies in unsigned latest.json.
    if let Err(error) = verify_update_artifact_identity(&update, &bytes) {
        let message = format!("Update to {version} was rejected: {error}");
        eprintln!("[shellx-cut] updater: {message}");
        state.update_and_broadcast(app, |snap| {
            snap.installing = false;
            snap.error = Some(message.clone());
        });
        let _ = app
            .dialog()
            .message(&message)
            .title("Update rejected")
            .blocking_show();
        return Ok(json!({ "ok": false, "error": message }));
    }

    #[cfg(windows)]
    let engine_was_stopped = match crate::update_handoff::stop_owned_engine_for_update(app) {
        Ok(stopped) => stopped,
        Err(e) => {
            let message = format!("Update to {version} was not started: {e}");
            eprintln!("[shellx-cut] updater: {message}");
            state.update_and_broadcast(app, |snap| {
                snap.installing = false;
                snap.error = Some(message.clone());
            });
            let _ = app
                .dialog()
                .message(&message)
                .title("Close ShellX Cut before updating")
                .blocking_show();
            return Ok(json!({ "ok": false, "error": message }));
        }
    };
    #[cfg(not(windows))]
    let engine_was_stopped = false;

    if let Err(e) = update.install(bytes) {
        let message = format!("Update to {version} could not be installed: {e}");
        eprintln!("[shellx-cut] updater: {message}");
        state.update_and_broadcast(app, |snap| {
            snap.installing = false;
            // Status stays `available` — the release is still real; only this
            // install attempt failed, and the UI shows the honest error.
            snap.error = Some(message.clone());
        });
        let _ = app
            .dialog()
            .message(format!("Update to {version} could not be installed:\n{e}"))
            .title("Update failed")
            .blocking_show();
        // A verified Windows payload can still fail before NSIS takes over
        // (for example while unpacking). If we already quiesced our engine,
        // relaunch the still-installed version so the editor does not remain
        // open in a misleading half-alive state.
        if engine_was_stopped {
            app.restart();
        }
        return Ok(json!({ "ok": false, "error": message }));
    }
    // Installed — relaunch into the new version. On Windows successful
    // `install` starts NSIS and exits the current process before reaching this
    // line; elsewhere this triggers the restart. Never returns on success.
    app.restart();
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests for the pure state machine.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact_identity_release(
        advertised_version: &str,
        identity_version: &str,
        platform: &str,
        bytes: &[u8],
    ) -> serde_json::Value {
        json!({
            "version": advertised_version,
            "shellx_cut_artifact_identities": {
                "schema": ARTIFACT_IDENTITY_SCHEMA,
                "platforms": {
                    platform: {
                        "version": identity_version,
                        "sha256": format!("{:x}", Sha256::digest(bytes)),
                        "signature": "fixture-signature",
                    }
                }
            }
        })
    }

    fn fresh() -> Snapshot {
        Snapshot::new("0.6.105".into(), true)
    }

    #[test]
    fn a_found_update_becomes_available_with_version_and_timestamp() {
        let mut snap = fresh();
        assert_eq!(snap.status, Status::Idle);
        snap.apply_outcome(CheckOutcome::Available("0.7.0".into()), 1_000);
        assert_eq!(snap.status, Status::Available);
        assert_eq!(snap.version.as_deref(), Some("0.7.0"));
        assert_eq!(snap.checked_at, Some(1_000));
        assert_eq!(snap.error, None);
    }

    #[test]
    fn up_to_date_reports_none_and_clears_a_previously_offered_version() {
        let mut snap = fresh();
        snap.apply_outcome(CheckOutcome::Available("0.7.0".into()), 1_000);
        // Release pulled between checks → the button must disappear.
        snap.apply_outcome(CheckOutcome::UpToDate, 2_000);
        assert_eq!(snap.status, Status::UpToDate);
        assert_eq!(snap.version, None);
        assert_eq!(snap.checked_at, Some(2_000));
    }

    #[test]
    fn a_failed_first_check_is_an_honest_error_state() {
        let mut snap = fresh();
        snap.apply_outcome(
            CheckOutcome::Failed("update check failed: dns".into()),
            1_000,
        );
        assert_eq!(snap.status, Status::Error);
        assert_eq!(snap.version, None);
        assert_eq!(snap.error.as_deref(), Some("update check failed: dns"));
        assert_eq!(snap.checked_at, Some(1_000));
    }

    #[test]
    fn a_transient_failure_never_hides_an_already_found_update() {
        let mut snap = fresh();
        snap.apply_outcome(CheckOutcome::Available("0.7.0".into()), 1_000);
        snap.apply_outcome(
            CheckOutcome::Failed("update check failed: offline".into()),
            2_000,
        );
        // The release still exists — keep the button, surface the error too.
        assert_eq!(snap.status, Status::Available);
        assert_eq!(snap.version.as_deref(), Some("0.7.0"));
        assert_eq!(snap.error.as_deref(), Some("update check failed: offline"));
        assert_eq!(snap.checked_at, Some(2_000));
    }

    #[test]
    fn a_successful_check_clears_a_previous_error() {
        let mut snap = fresh();
        snap.apply_outcome(CheckOutcome::Failed("boom".into()), 1_000);
        snap.apply_outcome(CheckOutcome::UpToDate, 2_000);
        assert_eq!(snap.status, Status::UpToDate);
        assert_eq!(snap.error, None);
    }

    #[test]
    fn every_completed_check_finishes_the_in_flight_flag() {
        let mut snap = fresh();
        snap.checking = true;
        snap.apply_outcome(CheckOutcome::UpToDate, 1_000);
        assert!(!snap.checking);
    }

    #[test]
    fn the_preference_gates_automatic_checks_only() {
        // Automatic path: OFF means no launch and no periodic check.
        assert!(!should_auto_check(false));
        assert!(should_auto_check(true));
        // The manual command does not call should_auto_check at all — asserted
        // structurally by the disclosure contract test (update_check_now has
        // no check_on_launch read).
    }

    #[test]
    fn the_periodic_cadence_is_six_hours() {
        assert_eq!(AUTO_CHECK_INTERVAL, Duration::from_secs(21_600));
    }

    #[test]
    fn artifact_identity_rejects_an_old_signed_package_advertised_as_newer() {
        // RED regression for CUT-UPDATER-VERSION-BYTES-001: a release writer
        // can reuse old package bytes and their valid package signature under
        // a higher `latest.json` version. The second, signed identity must
        // reject the version mismatch before either native installer runs.
        let bytes = b"officially signed 0.6.105 package bytes";
        let release = artifact_identity_release("0.6.109", "0.6.105", "windows-x86_64", bytes);
        assert!(matches!(
            updater_artifact_identity_claim(&release, "0.6.109", "windows-x86_64", bytes),
            Err(error) if error.contains("metadata advertises 0.6.109, but its signed artifact identity is 0.6.105")
        ));
    }

    #[test]
    fn artifact_identity_accepts_matching_version_platform_and_bytes() {
        let bytes = b"officially signed 0.6.109 package bytes";
        let release = artifact_identity_release("0.6.109", "0.6.109", "darwin-aarch64", bytes);
        let (identity, signature) =
            updater_artifact_identity_claim(&release, "0.6.109", "darwin-aarch64", bytes)
                .expect("matching identity claim");
        assert_eq!(signature, "fixture-signature");
        assert_eq!(
            identity,
            format!(
                "{ARTIFACT_IDENTITY_SCHEMA}\nversion=0.6.109\nplatform=darwin-aarch64\nsha256={:x}\n",
                Sha256::digest(bytes),
            )
        );
    }

    #[test]
    fn artifact_identity_rejects_substituted_bytes_even_with_matching_version() {
        let release =
            artifact_identity_release("0.6.109", "0.6.109", "windows-x86_64", b"signed bytes");
        assert!(matches!(
            updater_artifact_identity_claim(
                &release,
                "0.6.109",
                "windows-x86_64",
                b"substituted bytes",
            ),
            Err(error) if error.contains("bytes do not match their signed artifact identity")
        ));
    }

    #[test]
    fn artifact_identity_uses_the_runtime_updater_public_key() {
        updater_public_key().expect("runtime updater public key must parse as Minisign");
    }

    #[test]
    fn an_unsupported_platform_snapshot_is_honest_and_inert() {
        let snap = Snapshot::new("0.6.105".into(), false);
        assert_eq!(snap.status, Status::Unsupported);
        let v = snap.to_json();
        assert_eq!(v["status"], "unsupported");
        assert_eq!(v["supported"], false);
        assert_eq!(v["current"], "0.6.105");
    }

    #[test]
    fn the_snapshot_json_carries_the_full_bridge_contract() {
        let mut snap = fresh();
        snap.apply_outcome(CheckOutcome::Available("0.7.0".into()), 42);
        let v = snap.to_json();
        assert_eq!(v["schema"], "shellx-cut/update-state/1");
        assert_eq!(v["status"], "available");
        assert_eq!(v["version"], "0.7.0");
        assert_eq!(v["current"], "0.6.105");
        assert_eq!(v["checked_at"], 42);
        assert_eq!(v["error"], serde_json::Value::Null);
        assert_eq!(v["checking"], false);
        assert_eq!(v["installing"], false);
        assert_eq!(v["supported"], true);
    }
}

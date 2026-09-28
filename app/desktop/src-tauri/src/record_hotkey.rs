//! Truthful desktop Record F9 capability state.
//!
//! Focused-window F9 is always implemented by the React Record panel. This
//! module reports the separate native route only after it has actually been
//! configured or observed. Linux GNOME configuration lives in the sibling
//! module so the portable state and narrow Tauri commands stay small.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

#[cfg(target_os = "linux")]
#[path = "record_hotkey_gnome.rs"]
mod gnome;

const CAPABILITY_SCHEMA: &str = "shellx-cut/record-hotkey-capability@1";
const EVENT_NAME: &str = "cut:record-hotkey-capability";
const PREFERENCE_SCHEMA: &str = "shellx-cut/record-hotkey-preference@1";
const PREFERENCE_FILE: &str = "record-hotkey-preference.json";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct Capability {
    pub schema: &'static str,
    pub state: &'static str,
    pub scope: &'static str,
    pub backend: &'static str,
    pub enabled: bool,
    pub can_enable: bool,
    pub reason: Option<String>,
}

impl Capability {
    fn focused(reason: Option<String>) -> Self {
        Self {
            schema: CAPABILITY_SCHEMA,
            state: "disabled",
            scope: "focused_only",
            backend: "none",
            enabled: false,
            can_enable: false,
            reason,
        }
    }

    #[cfg(target_os = "linux")]
    fn gnome(state: &'static str, enabled: bool, reason: Option<String>) -> Self {
        Self {
            schema: CAPABILITY_SCHEMA,
            state,
            scope: if state == "observed" {
                "global"
            } else {
                "focused_only"
            },
            backend: "gnome_custom_keybinding",
            enabled,
            can_enable: true,
            reason,
        }
    }

    fn native_registered() -> Self {
        Self {
            schema: CAPABILITY_SCHEMA,
            state: "registered",
            scope: "global",
            backend: "native_global_shortcut",
            enabled: true,
            can_enable: false,
            reason: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct StoredPreference {
    schema: String,
    enabled: bool,
}

#[cfg(target_os = "linux")]
struct Runtime {
    service: Option<gnome::Service>,
}

pub(crate) struct RecordHotkeyState {
    capability: Mutex<Capability>,
    #[cfg(target_os = "linux")]
    runtime: Mutex<Runtime>,
}

impl Default for RecordHotkeyState {
    fn default() -> Self {
        Self {
            capability: Mutex::new(Capability::focused(Some(
                "Global F9 is not configured; F9 works while ShellX Cut is focused.".to_string(),
            ))),
            #[cfg(target_os = "linux")]
            runtime: Mutex::new(Runtime { service: None }),
        }
    }
}

fn preference_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|path| path.join(PREFERENCE_FILE))
        .map_err(|error| format!("could not resolve global F9 preference storage: {error}"))
}

/// Only a valid, explicitly saved `false` opts out of startup registration.
/// Missing or damaged preference data retains the default-on attempt.
fn read_preference(path: &Path) -> Option<bool> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| parse_preference(&bytes))
}

fn parse_preference(bytes: &[u8]) -> Option<bool> {
    serde_json::from_slice::<StoredPreference>(bytes)
        .ok()
        .filter(|preference| preference.schema == PREFERENCE_SCHEMA)
        .map(|preference| preference.enabled)
}

#[cfg(target_os = "linux")]
fn should_configure_on_startup(preference: Option<bool>) -> bool {
    preference != Some(false)
}

fn write_preference(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let path = preference_path(app)?;
    let parent = path
        .parent()
        .ok_or_else(|| "could not resolve global F9 preference folder".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("could not create global F9 preference folder: {error}"))?;
    let bytes = serde_json::to_vec_pretty(&StoredPreference {
        schema: PREFERENCE_SCHEMA.to_string(),
        enabled,
    })
    .map_err(|error| format!("could not encode global F9 preference: {error}"))?;
    std::fs::write(&path, bytes)
        .map_err(|error| format!("could not save global F9 preference: {error}"))
}

fn publish(app: &AppHandle, capability: Capability) {
    *app.state::<RecordHotkeyState>().capability.lock().unwrap() = capability.clone();
    let _ = app.emit(EVENT_NAME, capability);
}

pub(crate) fn capability(state: &RecordHotkeyState) -> Capability {
    state.capability.lock().unwrap().clone()
}

#[tauri::command]
pub(crate) fn get_record_hotkey_capability(
    state: tauri::State<'_, RecordHotkeyState>,
) -> Capability {
    capability(&state)
}

#[cfg(target_os = "linux")]
fn enable_gnome(
    app: &AppHandle,
    state: &RecordHotkeyState,
    startup: bool,
) -> Result<Capability, String> {
    if state.runtime.lock().unwrap().service.is_some() {
        return Ok(capability(state));
    }
    let preference_enabled = if startup {
        None
    } else {
        read_preference(&preference_path(app)?)
    };
    let service = gnome::Service::configure(app)?;
    if !startup && preference_enabled != Some(true) {
        if let Err(error) = write_preference(app, true) {
            let _ = gnome::remove_exact_owned_binding();
            drop(service);
            return Err(error);
        }
    }
    let capability = Capability::gnome(
        "configured",
        true,
        Some(
            "Global F9 is configured in GNOME. It becomes global after ShellX Cut observes the first forwarded F9."
                .to_string(),
        ),
    );
    state.runtime.lock().unwrap().service = Some(service);
    publish(app, capability.clone());
    Ok(capability)
}

#[cfg(target_os = "linux")]
#[tauri::command]
pub(crate) fn enable_gnome_record_hotkey(
    app: AppHandle,
    state: tauri::State<'_, RecordHotkeyState>,
) -> Result<Capability, String> {
    enable_gnome(&app, &state, false)
}

#[cfg(not(target_os = "linux"))]
#[tauri::command]
pub(crate) fn enable_gnome_record_hotkey(
    _app: AppHandle,
    _state: tauri::State<'_, RecordHotkeyState>,
) -> Result<Capability, String> {
    Err("GNOME global F9 is available only on Linux GNOME sessions.".to_string())
}

#[cfg(target_os = "linux")]
fn disabled_gnome_capability(cleanup_error: Option<String>) -> Capability {
    Capability::gnome(
        "disabled",
        false,
        Some(match cleanup_error {
            Some(error) => format!(
                "Global F9 is disabled locally, but its GNOME shortcut could not be removed: {error}"
            ),
            None => "Global F9 is disabled. F9 still works while ShellX Cut is focused.".to_string(),
        }),
    )
}

#[cfg(target_os = "linux")]
fn startup_failure_capability(error: String) -> Capability {
    let error = error.trim_end_matches('.');
    Capability::gnome(
        "disabled",
        false,
        Some(format!(
            "Global F9 could not be configured at startup: {error}. F9 still works while ShellX Cut is focused."
        )),
    )
}

#[cfg(target_os = "linux")]
#[tauri::command]
pub(crate) fn disable_gnome_record_hotkey(
    app: AppHandle,
    state: tauri::State<'_, RecordHotkeyState>,
) -> Result<Capability, String> {
    write_preference(&app, false)?;
    let service = state.runtime.lock().unwrap().service.take();
    drop(service);
    let cleanup_error = gnome::remove_exact_owned_binding().err();
    let capability = disabled_gnome_capability(cleanup_error);
    publish(&app, capability.clone());
    Ok(capability)
}

#[cfg(not(target_os = "linux"))]
#[tauri::command]
pub(crate) fn disable_gnome_record_hotkey(
    _app: AppHandle,
    _state: tauri::State<'_, RecordHotkeyState>,
) -> Result<Capability, String> {
    Err("GNOME global F9 is available only on Linux GNOME sessions.".to_string())
}

#[cfg(target_os = "linux")]
pub(crate) fn configure_on_startup(app: &AppHandle) {
    let path = preference_path(app).ok();
    if !should_configure_on_startup(path.as_deref().and_then(read_preference)) {
        publish(
            app,
            Capability::gnome(
                "disabled",
                false,
                Some(
                    "Global F9 was disabled in Cut. F9 still works while ShellX Cut is focused."
                        .to_string(),
                ),
            ),
        );
        return;
    }
    let state = app.state::<RecordHotkeyState>();
    if let Err(error) = enable_gnome(app, &state, true) {
        publish(app, startup_failure_capability(error));
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn configure_on_startup(_app: &AppHandle) {}

pub(crate) fn native_registered(app: &AppHandle) {
    publish(app, Capability::native_registered());
}

pub(crate) fn use_gnome_wayland_fallback() -> bool {
    #[cfg(target_os = "linux")]
    {
        gnome::should_use_fallback()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

#[cfg(target_os = "linux")]
pub(super) fn mark_gnome_observed(app: &AppHandle) {
    publish(
        app,
        Capability::gnome(
            "observed",
            true,
            Some(
                "Global F9 is active: ShellX Cut observed the GNOME shortcut callback.".to_string(),
            ),
        ),
    );
}

/// Shut down only the listener this process owns. A normal next launch attempts
/// configuration again unless the user explicitly saved the disabled preference.
pub(crate) fn release_runtime(app: &AppHandle) {
    #[cfg(target_os = "linux")]
    {
        let service = app
            .state::<RecordHotkeyState>()
            .runtime
            .lock()
            .unwrap()
            .service
            .take();
        drop(service);
        if let Err(error) = gnome::remove_exact_owned_binding() {
            eprintln!("[shellx-cut] did not remove owned GNOME F9 binding at exit: {error}");
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
    }
}

/// The exact forwarder argument is a fixed one-byte local callback. It never
/// constructs a Tauri app or starts cutd, so GNOME can run it while Cut is
/// backgrounded without creating a second engine process.
fn is_exact_forwarder<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let values = args.into_iter().collect::<Vec<_>>();
    values.len() == 1 && values[0].as_ref() == "--record-hotkey-forwarder"
}

pub(crate) fn consume_forwarder_invocation<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    if !is_exact_forwarder(args) {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        let _ = gnome::forward_fixed_event();
        return true;
    }
    #[cfg(not(target_os = "linux"))]
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn startup_defaults_on_unless_the_user_explicitly_disabled_global_f9() {
        assert!(should_configure_on_startup(None));
        assert!(should_configure_on_startup(Some(true)));
        assert!(!should_configure_on_startup(Some(false)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn valid_saved_false_is_the_only_preference_that_suppresses_startup() {
        let encode = |schema: &str, enabled| {
            serde_json::to_vec(&StoredPreference {
                schema: schema.to_string(),
                enabled,
            })
            .unwrap()
        };
        assert_eq!(
            parse_preference(&encode(PREFERENCE_SCHEMA, true)),
            Some(true)
        );
        assert_eq!(
            parse_preference(&encode(PREFERENCE_SCHEMA, false)),
            Some(false)
        );
        assert_eq!(parse_preference(&encode("foreign", false)), None);
        assert_eq!(parse_preference(b"not-json"), None);
    }

    #[test]
    fn only_the_exact_forwarder_argument_can_skip_desktop_startup() {
        assert!(is_exact_forwarder(["--record-hotkey-forwarder"]));
        assert!(!is_exact_forwarder(["--record-hotkey-forwarder", "extra"]));
        assert!(!is_exact_forwarder(["--version"]));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn disabled_gnome_capability_keeps_manual_enable_available() {
        let capability = Capability::gnome("disabled", false, Some("focused only".to_string()));
        assert_eq!(capability.scope, "focused_only");
        assert!(!capability.enabled);
        assert!(capability.can_enable);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn failed_binding_cleanup_still_publishes_the_disabled_local_state() {
        let capability = disabled_gnome_capability(Some("entry is foreign".to_string()));
        assert_eq!(capability.state, "disabled");
        assert!(!capability.enabled);
        assert!(capability
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("could not be removed")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn startup_refusal_keeps_focused_f9_and_reports_the_actual_blocker() {
        let capability = startup_failure_capability("F9 belongs to another shortcut".to_string());
        assert_eq!(capability.state, "disabled");
        assert_eq!(capability.scope, "focused_only");
        assert!(!capability.enabled);
        assert!(capability.can_enable);
        assert!(capability
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("F9 belongs to another shortcut")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn startup_refusal_does_not_duplicate_error_punctuation() {
        let capability = startup_failure_capability("GNOME did not preserve F9.".to_string());
        assert_eq!(
            capability.reason.as_deref(),
            Some("Global F9 could not be configured at startup: GNOME did not preserve F9. F9 still works while ShellX Cut is focused.")
        );
    }
}

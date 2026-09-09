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

fn read_preference(path: &Path) -> bool {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<StoredPreference>(&bytes).ok())
        .filter(|preference| preference.schema == PREFERENCE_SCHEMA)
        .is_some_and(|preference| preference.enabled)
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
    restoring: bool,
) -> Result<Capability, String> {
    if state.runtime.lock().unwrap().service.is_some() {
        return Ok(capability(state));
    }
    let preference_enabled = read_preference(&preference_path(app)?);
    let service = gnome::Service::configure(app)?;
    if !restoring && !preference_enabled {
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
pub(crate) fn restore_opt_in(app: &AppHandle) {
    let enabled = preference_path(app).is_ok_and(|path| read_preference(&path));
    if !enabled {
        publish(
            app,
            Capability::gnome(
                "disabled",
                false,
                Some(
                    "Global F9 is disabled. F9 still works while ShellX Cut is focused."
                        .to_string(),
                ),
            ),
        );
        return;
    }
    let state = app.state::<RecordHotkeyState>();
    if let Err(error) = enable_gnome(app, &state, true) {
        publish(
            app,
            Capability::gnome(
                "disabled",
                true,
                Some(format!(
                    "Global F9 remains enabled but could not be restored: {error}"
                )),
            ),
        );
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn restore_opt_in(_app: &AppHandle) {}

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

/// Shut down only the listener this process owns. The persisted preference stays
/// true, so a normal next launch configures the exact same GNOME entry again.
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

    #[test]
    fn damaged_or_foreign_preferences_do_not_opt_in() {
        assert!(!serde_json::from_slice::<StoredPreference>(b"not-json")
            .is_ok_and(|stored| { stored.schema == PREFERENCE_SCHEMA && stored.enabled }));
    }

    #[test]
    fn only_the_exact_forwarder_argument_can_skip_desktop_startup() {
        assert!(is_exact_forwarder(["--record-hotkey-forwarder"]));
        assert!(!is_exact_forwarder(["--record-hotkey-forwarder", "extra"]));
        assert!(!is_exact_forwarder(["--version"]));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn disabled_gnome_capability_keeps_the_visible_opt_in_available() {
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
}

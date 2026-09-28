//! GNOME's visible custom-keybinding fallback for the Record F9 shortcut.
//!
//! GNOME 46 does not expose the GlobalShortcuts portal backend on the supported
//! PC2 Wayland host, and Shell's GrabAccelerator is allowlisted. A user-owned
//! GNOME custom keybinding is therefore the owned startup route. The binding
//! can invoke only this executable's exact one-byte forwarder.

use gio::prelude::*;
use tauri::{AppHandle, Emitter};

#[path = "record_hotkey_gnome_readback.rs"]
mod readback;
#[path = "record_hotkey_gnome_socket.rs"]
mod socket;
use readback::readback_diagnostic;
pub(crate) use socket::Service;

const ROOT_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
const ENTRY_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const ENTRY_PATH: &str =
    "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/shellxcut_record_f9/";
const ENTRY_NAME: &str = "ShellX Cut: Toggle recording";
const ENTRY_BINDING: &str = "F9";
const FORWARDER_ARGUMENT: &str = "--record-hotkey-forwarder";
impl Service {
    pub(crate) fn configure(app: &AppHandle) -> Result<Self, String> {
        if !should_use_fallback() {
            return Err("Global F9 setup requires a GNOME desktop session.".to_string());
        }
        ensure_schema(ROOT_SCHEMA)?;
        ensure_schema(ENTRY_SCHEMA)?;
        let callback_app = app.clone();
        let service = socket::Service::start(move || {
            super::mark_gnome_observed(&callback_app);
            let _ = callback_app.emit("cut:record-hotkey", ());
        })?;
        if let Err(error) = configure_exact_owned_binding() {
            drop(service);
            return Err(error);
        }
        Ok(service)
    }
}

pub(crate) fn forward_fixed_event() -> Result<(), String> {
    socket::forward_fixed_event()
}

pub(crate) fn should_use_fallback() -> bool {
    let gnome = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .any(|value| value.eq_ignore_ascii_case("gnome"));
    let wayland = std::env::var("XDG_SESSION_TYPE")
        .is_ok_and(|value| value.eq_ignore_ascii_case("wayland"))
        || std::env::var_os("WAYLAND_DISPLAY").is_some();
    gnome && wayland
}

fn ensure_schema(id: &str) -> Result<(), String> {
    gio::SettingsSchemaSource::default()
        .and_then(|source| source.lookup(id, true))
        .map(|_| ())
        .ok_or_else(|| format!("GNOME Settings schema '{id}' is unavailable."))
}

fn root_settings() -> gio::Settings {
    gio::Settings::new(ROOT_SCHEMA)
}

fn entry_settings() -> gio::Settings {
    gio::Settings::with_path(ENTRY_SCHEMA, ENTRY_PATH)
}

type EntryValues = (String, String, String);

fn entry_values(entry: &gio::Settings) -> EntryValues {
    (
        entry.string("name").to_string(),
        entry.string("binding").to_string(),
        entry.string("command").to_string(),
    )
}

fn exact_entry(entry: &gio::Settings, command: &str) -> bool {
    entry_values(entry)
        == (
            ENTRY_NAME.to_string(),
            ENTRY_BINDING.to_string(),
            command.to_string(),
        )
}

fn executable_command() -> Result<String, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("could not resolve ShellX Cut executable: {error}"))?
        .canonicalize()
        .map_err(|error| format!("could not resolve physical ShellX Cut executable: {error}"))?;
    if !executable.is_absolute() || !executable.is_file() {
        return Err("ShellX Cut executable is not a physical regular file.".to_string());
    }
    let text = executable
        .to_str()
        .ok_or_else(|| "ShellX Cut executable path is not valid UTF-8.".to_string())?;
    if text.contains(['\0', '\n', '\r']) {
        return Err(
            "ShellX Cut executable path contains an unsupported control character.".to_string(),
        );
    }
    Ok(format!("{} {FORWARDER_ARGUMENT}", posix_single_quote(text)))
}

fn posix_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn configured_paths(root: &gio::Settings) -> Vec<String> {
    root.strv("custom-keybindings")
        .iter()
        .map(ToString::to_string)
        .collect()
}

fn apply_entry(entry: &gio::Settings, values: &EntryValues) -> Result<(), String> {
    entry.delay();
    let staged = (|| {
        entry
            .set_string("name", &values.0)
            .map_err(|error| format!("could not set GNOME shortcut name: {error}"))?;
        entry
            .set_string("binding", &values.1)
            .map_err(|error| format!("could not set GNOME F9 binding: {error}"))?;
        entry
            .set_string("command", &values.2)
            .map_err(|error| format!("could not set GNOME shortcut command: {error}"))?;
        Ok(())
    })();
    if let Err(error) = staged {
        entry.revert();
        return Err(error);
    }
    entry.apply();
    gio::Settings::sync();
    let observed = entry_values(entry);
    if observed != *values {
        eprintln!(
            "[shellx-cut] GNOME F9 binding readback mismatch: {}",
            readback_diagnostic(values, &observed)
        );
        return Err("GNOME did not preserve ShellX Cut's exact F9 binding.".to_string());
    }
    Ok(())
}

fn set_exact_entry(entry: &gio::Settings, command: &str) -> Result<(), String> {
    apply_entry(
        entry,
        &(
            ENTRY_NAME.to_string(),
            ENTRY_BINDING.to_string(),
            command.to_string(),
        ),
    )
}

fn rollback_configuration(
    root: &gio::Settings,
    original_paths: &[String],
    entry: &gio::Settings,
    original_entry: &EntryValues,
    error: String,
) -> Result<(), String> {
    let root_result = root
        .set_strv("custom-keybindings", original_paths)
        .map_err(|rollback| format!("could not restore GNOME shortcut list: {rollback}"));
    gio::Settings::sync();
    let entry_result = apply_entry(entry, original_entry)
        .map_err(|rollback| format!("could not restore GNOME shortcut entry: {rollback}"));
    match (root_result, entry_result) {
        (Ok(()), Ok(())) => Err(error),
        (root_result, entry_result) => Err(format!(
            "{error}; recovery retained a visible GNOME shortcut configuration ({}, {})",
            root_result
                .err()
                .unwrap_or_else(|| "list restored".to_string()),
            entry_result
                .err()
                .unwrap_or_else(|| "entry restored".to_string())
        )),
    }
}

fn reject_foreign_f9(paths: &[String]) -> Result<(), String> {
    for path in paths {
        if path == ENTRY_PATH {
            continue;
        }
        let entry = gio::Settings::with_path(ENTRY_SCHEMA, path);
        if entry.string("binding") == ENTRY_BINDING {
            return Err(
                "F9 is already assigned in GNOME Shortcut settings. Choose or remove that shortcut before enabling ShellX Cut global F9."
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn configure_exact_owned_binding() -> Result<(), String> {
    let command = executable_command()?;
    let root = root_settings();
    let entry = entry_settings();
    let paths = configured_paths(&root);
    let original_entry = entry_values(&entry);
    reject_foreign_f9(&paths)?;
    if paths.iter().any(|path| path == ENTRY_PATH) && !exact_entry(&entry, &command) {
        return Err(
            "The ShellX Cut GNOME shortcut path is occupied by a different binding; it was not changed."
                .to_string(),
        );
    }
    if !paths.iter().any(|path| path == ENTRY_PATH)
        && entry_values(&entry) != ("".to_string(), "".to_string(), "".to_string())
        && !exact_entry(&entry, &command)
    {
        return Err(
            "The ShellX Cut GNOME shortcut path has stale non-ShellX values; it was not changed."
                .to_string(),
        );
    }
    if !exact_entry(&entry, &command) {
        if let Err(error) = set_exact_entry(&entry, &command) {
            return rollback_configuration(&root, &paths, &entry, &original_entry, error);
        }
    }
    if !paths.iter().any(|path| path == ENTRY_PATH) {
        let mut updated = paths.clone();
        updated.push(ENTRY_PATH.to_string());
        if let Err(error) = root.set_strv("custom-keybindings", updated) {
            return rollback_configuration(
                &root,
                &paths,
                &entry,
                &original_entry,
                format!("could not add ShellX Cut to GNOME shortcuts: {error}"),
            );
        }
        gio::Settings::sync();
        if !configured_paths(&root)
            .iter()
            .any(|path| path == ENTRY_PATH)
        {
            return rollback_configuration(
                &root,
                &paths,
                &entry,
                &original_entry,
                "GNOME did not retain ShellX Cut's custom shortcut entry.".to_string(),
            );
        }
    }
    Ok(())
}

pub(crate) fn remove_exact_owned_binding() -> Result<(), String> {
    if !schema_available() {
        return Ok(());
    }
    let command = executable_command()?;
    let root = root_settings();
    let entry = entry_settings();
    let paths = configured_paths(&root);
    if !paths.iter().any(|path| path == ENTRY_PATH) {
        return Ok(());
    }
    if !exact_entry(&entry, &command) {
        return Err(
            "The GNOME shortcut at ShellX Cut's path changed ownership; it was not removed."
                .to_string(),
        );
    }
    let updated = paths
        .into_iter()
        .filter(|path| path != ENTRY_PATH)
        .collect::<Vec<_>>();
    root.set_strv("custom-keybindings", updated)
        .map_err(|error| format!("could not remove ShellX Cut's GNOME F9 shortcut: {error}"))?;
    gio::Settings::sync();
    if configured_paths(&root)
        .iter()
        .any(|path| path == ENTRY_PATH)
    {
        return Err("GNOME did not remove ShellX Cut's exact shortcut entry.".to_string());
    }
    Ok(())
}

fn schema_available() -> bool {
    gio::SettingsSchemaSource::default()
        .and_then(|source| source.lookup(ROOT_SCHEMA, true))
        .is_some()
        && gio::SettingsSchemaSource::default()
            .and_then(|source| source.lookup(ENTRY_SCHEMA, true))
            .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_keeps_the_forwarder_argument_separate() {
        assert_eq!(
            posix_single_quote("/opt/ShellX Cut/shellx-cut"),
            "'/opt/ShellX Cut/shellx-cut'"
        );
        assert_eq!(posix_single_quote("a'b"), "'a'\"'\"'b'");
    }
}

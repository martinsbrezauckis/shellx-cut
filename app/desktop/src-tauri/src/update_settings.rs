//! Persisted desktop-shell preferences for the launch-time updater.
//!
//! The updater runs before the engine-served UI can participate, so this one
//! boolean belongs to the native shell rather than project state or the public
//! Cut verb registry. The remote UI receives only two narrow app commands:
//! read the preference and replace it with an explicit boolean.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::Manager;

const SETTINGS_SCHEMA: &str = "shellx-cut/update-preferences/1";
const SETTINGS_FILE: &str = "update-preferences.json";

#[derive(Debug, Deserialize, Serialize)]
struct StoredUpdatePreferences {
    schema: String,
    check_on_launch: bool,
}

fn parse_check_on_launch(bytes: &[u8]) -> bool {
    serde_json::from_slice::<StoredUpdatePreferences>(bytes)
        .ok()
        .filter(|stored| stored.schema == SETTINGS_SCHEMA)
        .map(|stored| stored.check_on_launch)
        // Preserve the long-standing behavior for a first launch, an older
        // install, or a damaged preference file. The UI discloses this default.
        .unwrap_or(true)
}

fn read_check_on_launch(path: &Path) -> bool {
    std::fs::read(path)
        .map(|bytes| parse_check_on_launch(&bytes))
        .unwrap_or(true)
}

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|dir| dir.join(SETTINGS_FILE))
        .map_err(|error| format!("could not resolve app settings: {error}"))
}

pub(crate) fn check_on_launch(app: &tauri::AppHandle) -> bool {
    settings_path(app)
        .map(|path| read_check_on_launch(&path))
        .unwrap_or(true)
}

#[tauri::command]
pub(crate) fn get_update_preferences(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "schema": SETTINGS_SCHEMA,
        "check_on_launch": check_on_launch(&app),
    }))
}

#[tauri::command]
pub(crate) fn set_update_preferences(
    app: tauri::AppHandle,
    check_on_launch: bool,
) -> Result<serde_json::Value, String> {
    let path = settings_path(&app)?;
    let parent = path
        .parent()
        .ok_or_else(|| "could not resolve app settings folder".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("could not create app settings folder: {error}"))?;
    let bytes = serde_json::to_vec_pretty(&StoredUpdatePreferences {
        schema: SETTINGS_SCHEMA.to_string(),
        check_on_launch,
    })
    .map_err(|error| format!("could not encode update preference: {error}"))?;
    std::fs::write(&path, bytes)
        .map_err(|error| format!("could not save update preference: {error}"))?;
    Ok(serde_json::json!({
        "schema": SETTINGS_SCHEMA,
        "check_on_launch": read_check_on_launch(&path),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_launch_check_modes_round_trip_through_the_parser() {
        for enabled in [true, false] {
            let bytes = serde_json::to_vec(&StoredUpdatePreferences {
                schema: SETTINGS_SCHEMA.to_string(),
                check_on_launch: enabled,
            })
            .unwrap();
            assert_eq!(parse_check_on_launch(&bytes), enabled);
        }
    }

    #[test]
    fn absent_malformed_or_wrong_schema_preferences_keep_the_disclosed_default() {
        assert!(read_check_on_launch(Path::new(
            "definitely-not-a-real-update-pref.json"
        )));
        assert!(parse_check_on_launch(b"not json"));
        assert!(parse_check_on_launch(
            br#"{"schema":"shellx-cut/update-preferences/0","check_on_launch":false}"#
        ));
    }
}

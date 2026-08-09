use super::permissions::{read_state_at, set_enabled_at, PermissionState, PermissionStateProblem};
use super::{PluginAccess, BUILTIN_PLUGINS};
use std::fs;

#[test]
fn missing_state_defaults_to_all_builtins_enabled() {
    let temp = tempfile::tempdir().unwrap();
    let state = read_state_at(&temp.path().join("plugins.json"));
    assert_eq!(state.access("openverse-assets"), PluginAccess::Enabled);
    assert_eq!(state.status_json()["status"], "ready");
}

#[test]
fn corrupt_state_blocks_all_plugins_and_advertises_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("plugins.json");
    fs::write(&path, r#"{"disabled":["openverse-assets""#).unwrap();

    let state = read_state_at(&path);
    assert_eq!(
        state,
        PermissionState::Blocked(PermissionStateProblem::Corrupt)
    );
    for plugin in BUILTIN_PLUGINS {
        assert_eq!(
            state.access(plugin.name),
            PluginAccess::Blocked(PermissionStateProblem::Corrupt)
        );
    }
    assert_eq!(state.status_json()["status"], "corrupt");
    assert!(state.status_json()["recovery"]
        .as_str()
        .unwrap()
        .contains("plugins.enable"));
}

#[test]
fn malformed_but_parseable_state_is_also_corrupt() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("plugins.json");
    fs::write(&path, r#"{"disabled":["unknown-plugin"]}"#).unwrap();

    let state = read_state_at(&path);
    assert_eq!(
        state,
        PermissionState::Blocked(PermissionStateProblem::Corrupt)
    );
}

#[test]
fn explicit_enable_repairs_corruption_without_granting_other_plugins() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("plugins.json");
    fs::write(&path, "not json").unwrap();

    let update = set_enabled_at(&path, "openverse-assets", true).unwrap();
    assert!(update.recovered);
    let state = read_state_at(&path);
    assert_eq!(state.access("openverse-assets"), PluginAccess::Enabled);
    assert_eq!(state.access("matte-runtime"), PluginAccess::Disabled);
    assert_eq!(state.status_json()["status"], "ready");
}

#[test]
fn atomic_write_replaces_an_existing_complete_state_without_temp_residue() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("plugins.json");
    fs::write(&path, r#"{"disabled":["openverse-assets"]}"#).unwrap();

    let update = set_enabled_at(&path, "matte-runtime", false).unwrap();
    assert!(!update.recovered);
    let state = read_state_at(&path);
    assert_eq!(state.access("openverse-assets"), PluginAccess::Disabled);
    assert_eq!(state.access("matte-runtime"), PluginAccess::Disabled);
    let leftovers = fs::read_dir(temp.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .count();
    assert_eq!(leftovers, 0, "atomic replacement must clean its temp file");
}

#[test]
fn failed_replacement_keeps_the_existing_target_and_cleans_its_temp_file() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("plugins.json");
    fs::create_dir(&path).unwrap();

    assert!(set_enabled_at(&path, "openverse-assets", true).is_err());
    assert!(
        path.is_dir(),
        "failed rename must not replace the existing target"
    );
    let leftovers = fs::read_dir(temp.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .count();
    assert_eq!(leftovers, 0, "failed replacement must clean its temp file");
}

#[test]
fn concurrent_updates_keep_both_explicit_disables() {
    let temp = tempfile::tempdir().unwrap();
    let path_a = temp.path().join("plugins.json");
    let path_b = path_a.clone();
    let first = std::thread::spawn(move || set_enabled_at(&path_a, "openverse-assets", false));
    let second = std::thread::spawn(move || set_enabled_at(&path_b, "matte-runtime", false));
    first.join().unwrap().unwrap();
    second.join().unwrap().unwrap();

    let state = read_state_at(&temp.path().join("plugins.json"));
    assert_eq!(state.access("openverse-assets"), PluginAccess::Disabled);
    assert_eq!(state.access("matte-runtime"), PluginAccess::Disabled);
}

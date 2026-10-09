//! Capture-owned startup observation. It does not qualify event delivery.
use serde_json::{json, Value};

/// Missing/old/malformed records carry no observed startup-failure claim.
pub(crate) fn project_observation(project: &Value) -> Value {
    let unknown = || json!({"state":"unobserved", "backend":"unknown", "capture_keys":false});
    let Some(value) = project.get("input_hook") else {
        return unknown();
    };
    let Some(object) = value.as_object() else {
        return unknown();
    };
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "state" | "backend" | "reason" | "capture_keys"
        )
    }) {
        return unknown();
    }
    let state = value.get("state").and_then(Value::as_str);
    let backend = value.get("backend").and_then(Value::as_str);
    let keys = value.get("capture_keys").and_then(Value::as_bool);
    let reason = value.get("reason");
    let rdevin = matches!(
        backend,
        Some("rdevin_windows" | "rdevin_macos" | "rdevin_x11")
    );
    let valid = keys.is_some()
        && match state {
            Some("registered") => rdevin && reason.is_none_or(Value::is_null),
            Some("unavailable") => {
                rdevin && reason.and_then(Value::as_str) == Some("startup_failed")
            }
            Some("unobserved") => {
                matches!(backend, Some("unknown" | "wayland_evdev"))
                    && reason.is_none_or(Value::is_null)
            }
            _ => false,
        };
    if valid {
        value.clone()
    } else {
        unknown()
    }
}

/// Enrich only ordinary captures; private prepublished receipts retain exact bytes.
pub(super) fn project_bytes<T: serde::Serialize>(
    project: &T,
    observation: &record_capture::InputHookStartup,
    prepublished: bool,
) -> Result<Vec<u8>, serde_json::Error> {
    if prepublished {
        return serde_json::to_vec_pretty(project);
    }
    let mut value = serde_json::to_value(project)?;
    if let Some(object) = value.as_object_mut() {
        object.insert("input_hook".into(), serde_json::to_value(observation)?);
    }
    serde_json::to_vec_pretty(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observation_is_strict_and_legacy_safe() {
        for input in [
            json!({}),
            json!({"input_hook":null}),
            json!({"input_hook":{"state":"unavailable"}}),
            json!({"input_hook":{"state":"unavailable","backend":"wayland_evdev","reason":"startup_failed","capture_keys":false}}),
        ] {
            assert_eq!(project_observation(&input)["state"], "unobserved");
        }
        let observation = json!({"state":"unavailable","backend":"rdevin_windows","reason":"startup_failed","capture_keys":true});
        assert_eq!(
            project_observation(&json!({"input_hook":observation.clone()})),
            observation
        );
        let registered = json!({"state":"registered","backend":"rdevin_x11","capture_keys":false});
        assert_eq!(
            project_observation(&json!({"input_hook":registered.clone(),"events":{"clicks":[]}})),
            registered
        );
    }
    #[test]
    fn private_projection_bytes_remain_exact_and_ordinary_keeps_media() {
        let project = json!({"source_video":"owned/source.mp4","events":{"clicks":[]}});
        let observation = record_capture::InputHookStartup {
            state: record_capture::InputHookStartupState::Unavailable,
            backend: record_capture::InputHookBackend::RdevinWindows,
            reason: Some(record_capture::InputHookStartupReason::StartupFailed),
            capture_keys: true,
        };
        assert_eq!(
            project_bytes(&project, &observation, true).unwrap(),
            serde_json::to_vec_pretty(&project).unwrap()
        );
        let enriched: Value =
            serde_json::from_slice(&project_bytes(&project, &observation, false).unwrap()).unwrap();
        assert_eq!(enriched["source_video"], project["source_video"]);
        assert_eq!(enriched["events"], project["events"]);
        assert_eq!(project_observation(&enriched)["state"], "unavailable");
    }
}

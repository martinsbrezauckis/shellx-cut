use super::{
    fixed_source_from_stored, generic_label, outcome, preference, selected_preference_from_token,
    source_from_stored, tokens, PublicMicrophone,
};
use serde_json::json;

#[test]
fn migrates_legacy_system_default_without_inventing_identity() {
    let loaded =
        preference::migrate(json!({"mode":"system_default"})).expect("legacy default migrates");
    assert_eq!(loaded.preference.mode, preference::Mode::SystemDefault);
    assert!(loaded.preference.endpoint.is_none());
}

#[test]
fn corrupt_preference_falls_back_to_safe_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("microphone-preference.json");
    std::fs::write(&path, b"not-json").unwrap();
    let loaded = preference::load_at(&path);
    assert!(loaded.recovered_corruption);
    assert_eq!(loaded.preference.mode, preference::Mode::SystemDefault);
}

#[test]
fn stored_preference_never_uses_a_capability_token_as_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("microphone-preference.json");
    preference::save_at(&path, &preference::Stored::default()).unwrap();
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains(preference::SCHEMA));
    assert!(!text.contains("mic_cap_"));
}

#[test]
fn all_native_friendly_names_become_generic_category_ordinals() {
    assert_eq!(generic_label("Desk Mic", 1), "Microphone 1");
    assert_eq!(generic_label("USB Vendor Model", 2), "Microphone 2");
}

#[test]
fn public_microphone_projection_cannot_leak_a_private_endpoint_identity() {
    let public = PublicMicrophone {
        token: "mic_cap_current".into(),
        label: generic_label("Vendor Model Serial 123", 1),
    };
    let serialized = serde_json::to_string(&public).unwrap();
    assert_eq!(
        serialized,
        r#"{"token":"mic_cap_current","label":"Microphone 1"}"#
    );
    assert!(!serialized.contains("Vendor") && !serialized.contains("Serial"));
}

#[test]
fn opaque_token_consumes_to_its_private_reference_not_its_string_value() {
    let reference = record_capture::MicrophoneEndpointRef::Windows {
        endpoint_id: "private-endpoint-id".into(),
    };
    let token = tokens::issue(&reference).expect("OS CSPRNG issues capability");
    assert_ne!(token, "private-endpoint-id");
    assert_eq!(tokens::consume(&token), Some(reference));
}

#[test]
fn selected_token_is_resolved_to_private_storage_before_the_ui_mutation_returns() {
    let reference = record_capture::MicrophoneEndpointRef::Windows {
        endpoint_id: "private-endpoint-id".into(),
    };
    let token = tokens::issue(&reference).expect("OS CSPRNG issues capability");
    let endpoints = [record_capture::MicrophoneEndpoint {
        reference: reference.clone(),
        label: "native friendly name".into(),
    }];
    let stored =
        selected_preference_from_token(&token, &endpoints).expect("current token resolves");
    assert_eq!(stored.endpoint, Some(reference));
    assert!(!serde_json::to_string(&stored).unwrap().contains(&token));
    assert_eq!(
        fixed_source_from_stored(stored),
        Ok(record_capture::MicrophoneSource::Selected(
            endpoints[0].reference.clone()
        )),
        "the UI token fixes the selected source that Start will admit and capture"
    );
}

#[test]
fn unavailable_selected_preference_refuses_start_without_default_fallback() {
    let stored = preference::Stored {
        schema: preference::SCHEMA.into(),
        mode: preference::Mode::Selected,
        endpoint: Some(record_capture::MicrophoneEndpointRef::Windows {
            endpoint_id: "unavailable-private-endpoint".into(),
        }),
    };
    let error = source_from_stored(stored).expect_err("absent selected input must block start");
    assert!(error.message.contains("selected microphone is unavailable"));
}

#[test]
fn microphone_loss_has_an_explicit_safe_outcome() {
    let dir = tempfile::tempdir().unwrap();
    outcome::persist(
        dir.path(),
        record_capture::MicrophoneCaptureOutcome::MicrophoneLostNoTrack,
    )
    .unwrap();
    assert_eq!(
        outcome::projection(dir.path()),
        json!({"status": "microphone_lost", "mic_track_saved": false})
    );
}

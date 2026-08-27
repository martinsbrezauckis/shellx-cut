//! Private microphone preference owner and public-safe recorder projections.

mod outcome;
mod preference;
#[cfg(test)]
mod tests;
mod tokens;

use crate::dispatch::parse_args;
use cut_core::{error_codes, CutError, VerbResult};
use serde::Deserialize;
use serde_json::{json, Value};

pub(crate) use outcome::{
    persist as persist_capture_outcome, projection as capture_outcome_projection,
};

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct PublicMicrophone {
    pub token: String,
    pub label: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct MicrophoneSelection {
    pub mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub status: &'static str,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct DoctorProjection {
    pub microphones: Vec<PublicMicrophone>,
    pub microphone_selection: MicrophoneSelection,
}

fn generic_label(_private_native_label: &str, ordinal: usize) -> String {
    format!("Microphone {ordinal}")
}

fn public_endpoints() -> Vec<(record_capture::MicrophoneEndpointRef, PublicMicrophone)> {
    record_capture::list_microphone_endpoints()
        .into_iter()
        .enumerate()
        .filter_map(|(index, endpoint)| {
            let reference = endpoint.reference;
            Some((
                reference.clone(),
                PublicMicrophone {
                    token: tokens::issue(&reference)?,
                    label: generic_label(&endpoint.label, index + 1),
                },
            ))
        })
        .collect()
}

fn selection_projection(
    loaded: &preference::Loaded,
    endpoints: &[(record_capture::MicrophoneEndpointRef, PublicMicrophone)],
) -> MicrophoneSelection {
    if loaded.recovered_corruption {
        return MicrophoneSelection {
            mode: "system_default",
            label: None,
            status: "recovered_corruption",
        };
    }
    match (&loaded.preference.mode, loaded.preference.endpoint.as_ref()) {
        (preference::Mode::SystemDefault, None) => MicrophoneSelection {
            mode: "system_default",
            label: None,
            status: if endpoints.is_empty() {
                "system_default_only"
            } else {
                "ready"
            },
        },
        (preference::Mode::Selected, Some(reference)) => match endpoints
            .iter()
            .find(|(candidate, _)| candidate == reference)
        {
            Some((_, endpoint)) => MicrophoneSelection {
                mode: "selected",
                label: Some(endpoint.label.clone()),
                status: "ready",
            },
            None => MicrophoneSelection {
                mode: "selected",
                label: None,
                status: "unavailable",
            },
        },
        _ => MicrophoneSelection {
            mode: "system_default",
            label: None,
            status: "recovered_corruption",
        },
    }
}

pub(crate) fn doctor_projection() -> DoctorProjection {
    let loaded = preference::path()
        .map(|path| preference::load_at(&path))
        .unwrap_or(preference::Loaded {
            preference: preference::Stored::default(),
            recovered_corruption: true,
        });
    let endpoints = public_endpoints();
    DoctorProjection {
        microphone_selection: selection_projection(&loaded, &endpoints),
        microphones: endpoints.into_iter().map(|(_, public)| public).collect(),
    }
}

pub(crate) fn source_for_start() -> Result<record_capture::MicrophoneSource, CutError> {
    let loaded = preference::load_at(&preference::path()?);
    source_from_stored(loaded.preference)
}

fn source_from_stored(
    stored: preference::Stored,
) -> Result<record_capture::MicrophoneSource, CutError> {
    let source = fixed_source_from_stored(stored)?;
    record_capture::resolve_microphone_source(&source).map_err(super::record_err)?;
    Ok(source)
}

/// Translate private persistent state into the one fixed source captured by a
/// recording. Admission is intentionally separate so it can reject an absent
/// selected endpoint before any capture worker is created.
fn fixed_source_from_stored(
    stored: preference::Stored,
) -> Result<record_capture::MicrophoneSource, CutError> {
    match stored.mode {
        preference::Mode::SystemDefault => Ok(record_capture::MicrophoneSource::SystemDefault),
        preference::Mode::Selected => Ok(record_capture::MicrophoneSource::Selected(
            stored.endpoint.expect("validated selected preference"),
        )),
    }
}

pub(crate) fn warm_projection() -> Value {
    if recording_owns_source() {
        return json!({"live": false, "supported": true, "skipped": "recording", "message": "Microphone test is unavailable while recording."});
    }
    let source = match source_for_start() {
        Ok(source) => source,
        Err(_) => {
            return json!({"live": false, "supported": true, "selection_unavailable": true, "message": "Selected microphone is unavailable. Choose another microphone or System Default."})
        }
    };
    let warm = record_capture::warm_microphone(&source, 1_500);
    json!({"live": warm.live, "peak_dbfs": warm.peak_dbfs, "supported": warm.supported})
}

fn recording_owns_source() -> bool {
    super::capture_sessions()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .values()
        .next()
        .is_some()
}

pub(crate) async fn selection_handler(args: Value) -> Result<VerbResult, CutError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        mode: String,
        microphone_token: Option<String>,
    }

    let args: Args = parse_args(args)?;
    if recording_owns_source() {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "microphone selection is locked while recording",
            "a recording already owns its resolved microphone source",
        )
        .with_suggested_action("stop the recording before changing microphone input"));
    }
    let preference = match args.mode.as_str() {
        "system_default" if args.microphone_token.is_none() => preference::Stored::default(),
        "system_default" => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "System Default does not accept a microphone token",
                "remove microphone_token when selecting System Default",
            ))
        }
        "selected" => {
            let token = args.microphone_token.as_deref().ok_or_else(|| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    "selecting a microphone requires a current token",
                    "refresh the Record workspace and choose one listed microphone",
                )
            })?;
            selected_preference_from_token(token, &record_capture::list_microphone_endpoints())?
        }
        _ => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "microphone mode must be system_default or selected",
                "choose System Default or select a current listed microphone",
            ))
        }
    };
    preference::save_at(&preference::path()?, &preference)?;
    Ok(VerbResult::ok(
        json!({"microphone_selection": doctor_projection().microphone_selection}),
    ))
}

fn selected_preference_from_token(
    token: &str,
    endpoints: &[record_capture::MicrophoneEndpoint],
) -> Result<preference::Stored, CutError> {
    let reference = tokens::consume(token).ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "microphone token expired or is invalid",
            "refresh the microphone list and choose it again",
        )
    })?;
    if !endpoints
        .iter()
        .any(|endpoint| endpoint.reference == reference)
    {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            "selected microphone is no longer available",
            "the selected native endpoint disappeared before Cut could save it",
        )
        .with_suggested_action("reconnect it, choose another microphone, or use System Default"));
    }
    Ok(preference::Stored {
        schema: preference::SCHEMA.into(),
        mode: preference::Mode::Selected,
        endpoint: Some(reference),
    })
}

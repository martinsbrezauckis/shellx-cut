use crate::userdata;
use cut_core::{error_codes, CutError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

pub(super) const SCHEMA: &str = "shellx-cut/microphone-preference/2";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Mode {
    SystemDefault,
    Selected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Stored {
    pub(super) schema: String,
    pub(super) mode: Mode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) endpoint: Option<record_capture::MicrophoneEndpointRef>,
}

impl Default for Stored {
    fn default() -> Self {
        Self {
            schema: SCHEMA.into(),
            mode: Mode::SystemDefault,
            endpoint: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Loaded {
    pub(super) preference: Stored,
    pub(super) recovered_corruption: bool,
}

pub(super) fn path() -> Result<std::path::PathBuf, CutError> {
    userdata::microphone_preference_path().ok_or_else(|| {
        CutError::new(
            error_codes::IO,
            "no app-local microphone preference directory",
            "HOME/USERPROFILE is unset, so Cut cannot persist microphone selection",
        )
    })
}

pub(super) fn load_at(path: &Path) -> Loaded {
    let Ok(bytes) = std::fs::read(path) else {
        return Loaded {
            preference: Stored::default(),
            recovered_corruption: false,
        };
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return Loaded {
            preference: Stored::default(),
            recovered_corruption: true,
        };
    };
    migrate(value).unwrap_or_else(|| Loaded {
        preference: Stored::default(),
        recovered_corruption: true,
    })
}

pub(super) fn migrate(value: Value) -> Option<Loaded> {
    if value.get("schema").is_none() {
        return (value.get("mode").and_then(Value::as_str)? == "system_default").then(|| Loaded {
            preference: Stored::default(),
            recovered_corruption: false,
        });
    }
    let preference = serde_json::from_value::<Stored>(value).ok()?;
    match preference.schema.as_str() {
        SCHEMA
            if matches!(
                (&preference.mode, &preference.endpoint),
                (Mode::SystemDefault, None) | (Mode::Selected, Some(_))
            ) =>
        {
            Some(Loaded {
                preference,
                recovered_corruption: false,
            })
        }
        _ => None,
    }
}

pub(super) fn save_at(path: &Path, preference: &Stored) -> Result<(), CutError> {
    let parent = path.parent().ok_or_else(|| {
        CutError::new(
            error_codes::IO,
            "save microphone preference",
            "app-local microphone preference path has no parent directory",
        )
    })?;
    std::fs::create_dir_all(parent).map_err(|error| {
        CutError::new(
            error_codes::IO,
            "create microphone preference directory",
            error.to_string(),
        )
    })?;
    let bytes = serde_json::to_vec_pretty(preference).map_err(|error| {
        CutError::new(
            error_codes::IO,
            "serialize microphone preference",
            error.to_string(),
        )
    })?;
    record_recovery::replace_synced(path, &bytes).map_err(|error| {
        CutError::new(
            error_codes::IO,
            "publish microphone preference",
            error.to_string(),
        )
    })
}

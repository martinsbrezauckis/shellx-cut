use std::num::NonZeroU64;

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use thiserror::Error;

pub const MAX_SCENES: usize = 32;

pub type SceneResult<T> = std::result::Result<T, SceneError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SceneError {
    #[error("scene id is invalid")]
    InvalidSceneId,
    #[error("scene name is invalid")]
    InvalidSceneName,
    #[error("revision must be non-zero")]
    ZeroRevision,
    #[error("scene catalog must contain 1..={MAX_SCENES} unique presets")]
    InvalidCatalog,
    #[error("presenter PiP size is outside its safe range")]
    InvalidPipSize,
    #[error("scene is not in the accepted snapshot")]
    SceneMissing,
    #[error("scene preset revision is not in the accepted snapshot")]
    SceneRevisionMismatch,
    #[error("event belongs to another accepted snapshot")]
    StaleSnapshot,
    #[error("event sequence is duplicate or out of order")]
    InvalidSequence,
    #[error("event logical media time is out of order")]
    InvalidLogicalTime,
    #[error("scene activation selects the already-active scene")]
    SceneAlreadyActive,
    #[error("timer is disabled")]
    TimerDisabled,
    #[error("timer transition is invalid for its current state")]
    InvalidTimerTransition,
    #[error("logical timer duration overflowed")]
    TimerOverflow,
    #[error("scene persistence schema is unsupported")]
    UnsupportedSceneSchema,
    #[error("scene persistence value is invalid")]
    InvalidScenePersistence,
    #[error("scene presentation requires a sealed camera source")]
    CameraSourceRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct SceneId(String);

impl SceneId {
    pub fn parse(value: impl Into<String>) -> SceneResult<Self> {
        let value = value.into();
        is_valid_scene_id(&value)
            .then_some(Self(value))
            .ok_or(SceneError::InvalidSceneId)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn is_consistent(&self) -> bool {
        is_valid_scene_id(&self.0)
    }
}

impl<'de> Deserialize<'de> for SceneId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(D::Error::custom)
    }
}

fn is_valid_scene_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

macro_rules! revision {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        pub struct $name(NonZeroU64);

        impl $name {
            pub fn new(value: u64) -> SceneResult<Self> {
                NonZeroU64::new(value)
                    .map(Self)
                    .ok_or(SceneError::ZeroRevision)
            }

            pub fn get(self) -> u64 {
                self.0.get()
            }
        }
    };
}

revision!(CatalogRevision);
revision!(PresetRevision);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneSnapshotRevision {
    pub catalog: CatalogRevision,
    pub initial_preset: PresetRevision,
}

//! Immutable recorder-scene snapshots and their logical-media-time reducer.
//!
//! This module intentionally has no journal, capture, renderer, or wall-clock
//! dependency. Future layers may persist and project these values, but cannot
//! change an accepted snapshot while replaying its events.

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};

pub use crate::scene_model::{
    PipCorner, PipShape, PipSizePercent, PresenterPip, SceneComposition, ScenePreset, TimerConfig,
    MAX_PIP_SIZE_PERCENT, MIN_PIP_SIZE_PERCENT,
};
pub use crate::scene_types::{
    CatalogRevision, PresetRevision, SceneError, SceneId, SceneResult, SceneSnapshotRevision,
    MAX_SCENES,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SceneCatalog {
    revision: CatalogRevision,
    presets: Vec<ScenePreset>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneCatalogWire {
    revision: CatalogRevision,
    presets: Vec<ScenePreset>,
}

impl<'de> Deserialize<'de> for SceneCatalog {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SceneCatalogWire::deserialize(deserializer)?;
        Self::new(wire.revision, wire.presets).map_err(D::Error::custom)
    }
}

impl SceneCatalog {
    pub fn new(revision: CatalogRevision, presets: Vec<ScenePreset>) -> SceneResult<Self> {
        let catalog = Self { revision, presets };
        catalog
            .is_consistent()
            .then_some(catalog)
            .ok_or(SceneError::InvalidCatalog)
    }

    pub fn revision(&self) -> CatalogRevision {
        self.revision
    }

    pub fn presets(&self) -> &[ScenePreset] {
        &self.presets
    }

    pub fn preset(&self, id: &SceneId) -> Option<&ScenePreset> {
        self.presets.iter().find(|preset| preset.id() == id)
    }

    fn is_consistent(&self) -> bool {
        !self.presets.is_empty()
            && self.presets.len() <= MAX_SCENES
            && self.presets.iter().all(ScenePreset::is_consistent)
            && self.presets.iter().enumerate().all(|(index, preset)| {
                self.presets[..index]
                    .iter()
                    .all(|other| other.id() != preset.id())
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AcceptedStartSnapshot {
    revision: SceneSnapshotRevision,
    catalog: SceneCatalog,
    initial_scene_id: SceneId,
}

impl AcceptedStartSnapshot {
    pub fn accept(catalog: &SceneCatalog, scene_id: &SceneId) -> SceneResult<Self> {
        if !catalog.is_consistent() {
            return Err(SceneError::InvalidCatalog);
        }
        let scene = catalog.preset(scene_id).ok_or(SceneError::SceneMissing)?;
        Ok(Self {
            revision: SceneSnapshotRevision {
                catalog: catalog.revision,
                initial_preset: scene.revision(),
            },
            catalog: catalog.clone(),
            initial_scene_id: scene.id().clone(),
        })
    }

    pub fn revision(&self) -> SceneSnapshotRevision {
        self.revision
    }

    pub fn initial_scene_id(&self) -> &SceneId {
        &self.initial_scene_id
    }

    pub fn initial_scene(&self) -> SceneResult<&ScenePreset> {
        self.catalog
            .preset(&self.initial_scene_id)
            .ok_or(SceneError::SceneMissing)
    }

    /// The timer configuration is selected at acceptance and is global to the take.
    pub fn timer_config(&self) -> SceneResult<TimerConfig> {
        Ok(self.initial_scene()?.timer())
    }

    pub fn presets(&self) -> &[ScenePreset] {
        self.catalog.presets()
    }

    pub fn preset(&self, id: &SceneId, revision: PresetRevision) -> Option<&ScenePreset> {
        self.catalog
            .preset(id)
            .filter(|preset| preset.revision() == revision)
    }

    pub(crate) fn is_consistent(&self) -> bool {
        self.revision.catalog == self.catalog.revision
            && self.catalog.is_consistent()
            && self
                .catalog
                .preset(&self.initial_scene_id)
                .is_some_and(|preset| preset.revision() == self.revision.initial_preset)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptedStartSnapshotWire {
    revision: SceneSnapshotRevision,
    catalog: SceneCatalog,
    initial_scene_id: SceneId,
}

impl<'de> Deserialize<'de> for AcceptedStartSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = AcceptedStartSnapshotWire::deserialize(deserializer)?;
        let snapshot = Self {
            revision: wire.revision,
            catalog: wire.catalog,
            initial_scene_id: wire.initial_scene_id,
        };
        snapshot
            .is_consistent()
            .then_some(snapshot)
            .ok_or_else(|| D::Error::custom(SceneError::InvalidScenePersistence))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneEvent {
    pub sequence: u64,
    pub logical_media_time_ms: u64,
    pub snapshot_revision: SceneSnapshotRevision,
    pub kind: SceneEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SceneEventKind {
    ActivateScene {
        scene_id: SceneId,
        preset_revision: PresetRevision,
    },
    TimerStart,
    TimerPause,
    TimerResume,
    TimerReset,
    TimerEnd,
}

//! Exact bounded wire DTO for public Recording Scenes admission.

use record_core::scene::ScenePreset;
use record_core::{
    AcceptedStartSnapshot, CatalogRevision, PipCorner, PipShape, PipSizePercent, PresenterPip,
    PresetRevision, SceneCatalog, SceneComposition, SceneId, SceneResult, TimerConfig,
};
use serde::{Deserialize, Serialize};

/// One immutable public scene catalog. It is frozen into the journal header
/// before native capture starts and cannot be reinterpreted later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingSceneConfig {
    pub catalog_revision: u64,
    pub initial_scene_id: String,
    pub presets: Vec<RecordingScenePresetConfig>,
    /// Exactly one timer applies to all presets in the take.
    pub timer: RecordingSceneTimerConfig,
}

impl RecordingSceneConfig {
    pub fn default_screen() -> Self {
        Self {
            catalog_revision: 1,
            initial_scene_id: "screen".into(),
            presets: vec![RecordingScenePresetConfig {
                id: "screen".into(),
                name: "Screen".into(),
                preset_revision: 1,
                layout: RecordingSceneLayout::Screen,
            }],
            timer: RecordingSceneTimerConfig::Elapsed,
        }
    }

    /// Convert exact public vocabulary to the generic immutable snapshot.
    pub fn accepted_snapshot(&self) -> SceneResult<AcceptedStartSnapshot> {
        let catalog_revision = CatalogRevision::new(self.catalog_revision)?;
        let initial_scene_id = SceneId::parse(self.initial_scene_id.clone())?;
        let timer = self.timer.timer_config()?;
        let presets = self
            .presets
            .iter()
            .map(|preset| preset.scene_preset(timer))
            .collect::<SceneResult<Vec<_>>>()?;
        let catalog = SceneCatalog::new(catalog_revision, presets)?;
        AcceptedStartSnapshot::accept(&catalog, &initial_scene_id)
    }

    /// A catalog may contain Presenter PiP while a Screen capture starts with
    /// no camera. Only the initial selected preset needs start admission.
    pub fn initial_requires_camera(&self) -> SceneResult<bool> {
        let initial = self
            .presets
            .iter()
            .find(|preset| preset.id == self.initial_scene_id)
            .ok_or(record_core::SceneError::SceneMissing)?;
        Ok(matches!(
            initial.layout,
            RecordingSceneLayout::PresenterPip { .. }
        ))
    }
}

/// Exact wire form: `{kind:"off"|"elapsed"|"countdown",duration_ms?}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordingSceneTimerConfig {
    Off,
    Elapsed,
    Countdown { duration_ms: u64 },
}

impl RecordingSceneTimerConfig {
    pub(crate) fn timer_config(self) -> SceneResult<TimerConfig> {
        match self {
            Self::Off => Ok(TimerConfig::Off),
            Self::Elapsed => Ok(TimerConfig::Elapsed),
            Self::Countdown { duration_ms } => TimerConfig::countdown(duration_ms),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingScenePresetConfig {
    pub id: String,
    pub name: String,
    pub preset_revision: u64,
    pub layout: RecordingSceneLayout,
}

impl RecordingScenePresetConfig {
    pub(crate) fn scene_preset(&self, timer: TimerConfig) -> SceneResult<ScenePreset> {
        let composition = match self.layout {
            RecordingSceneLayout::Screen => SceneComposition::ScreenOnly,
            RecordingSceneLayout::PresenterPip {
                corner,
                size_percent,
                shape,
            } => SceneComposition::PresenterPip(PresenterPip::new(
                corner.into(),
                PipSizePercent::new(size_percent)?,
                shape.into(),
            )),
        };
        ScenePreset::new(
            SceneId::parse(self.id.clone())?,
            self.name.clone(),
            PresetRevision::new(self.preset_revision)?,
            composition,
            timer,
        )
    }
}

/// Only layouts faithfully represented by the existing composition model are
/// public. Unsupported names fail deserialize before a journal is created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordingSceneLayout {
    Screen,
    PresenterPip {
        corner: RecordingScenePipCorner,
        size_percent: u8,
        shape: RecordingScenePipShape,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingScenePipCorner {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

impl From<RecordingScenePipCorner> for PipCorner {
    fn from(value: RecordingScenePipCorner) -> Self {
        match value {
            RecordingScenePipCorner::TopLeft => Self::TopLeft,
            RecordingScenePipCorner::TopRight => Self::TopRight,
            RecordingScenePipCorner::BottomRight => Self::BottomRight,
            RecordingScenePipCorner::BottomLeft => Self::BottomLeft,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingScenePipShape {
    Circle,
    RoundedRect,
}

impl From<RecordingScenePipShape> for PipShape {
    fn from(value: RecordingScenePipShape) -> Self {
        match value {
            RecordingScenePipShape::Circle => Self::Circle,
            RecordingScenePipShape::RoundedRect => Self::RoundedRect,
        }
    }
}

use std::num::NonZeroU64;

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};

use crate::scene_types::{PresetRevision, SceneError, SceneId, SceneResult};

pub const MIN_PIP_SIZE_PERCENT: u8 = 15;
pub const MAX_PIP_SIZE_PERCENT: u8 = 35;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PipCorner {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PipShape {
    Circle,
    RoundedRect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PipSizePercent(u8);

impl PipSizePercent {
    pub fn new(value: u8) -> SceneResult<Self> {
        (MIN_PIP_SIZE_PERCENT..=MAX_PIP_SIZE_PERCENT)
            .contains(&value)
            .then_some(Self(value))
            .ok_or(SceneError::InvalidPipSize)
    }

    pub fn get(self) -> u8 {
        self.0
    }

    pub(crate) fn is_consistent(self) -> bool {
        (MIN_PIP_SIZE_PERCENT..=MAX_PIP_SIZE_PERCENT).contains(&self.0)
    }
}

impl<'de> Deserialize<'de> for PipSizePercent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u8::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresenterPip {
    corner: PipCorner,
    size_percent: PipSizePercent,
    shape: PipShape,
}

impl PresenterPip {
    pub fn new(corner: PipCorner, size_percent: PipSizePercent, shape: PipShape) -> Self {
        Self {
            corner,
            size_percent,
            shape,
        }
    }

    pub fn corner(self) -> PipCorner {
        self.corner
    }

    pub fn size_percent(self) -> PipSizePercent {
        self.size_percent
    }

    pub fn shape(self) -> PipShape {
        self.shape
    }

    fn is_consistent(self) -> bool {
        self.size_percent.is_consistent()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SceneComposition {
    ScreenOnly,
    PresenterPip(PresenterPip),
}

impl SceneComposition {
    fn is_consistent(self) -> bool {
        match self {
            Self::ScreenOnly => true,
            Self::PresenterPip(pip) => pip.is_consistent(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimerConfig {
    Off,
    Elapsed,
    Countdown { duration_ms: NonZeroU64 },
}

impl TimerConfig {
    pub fn countdown(duration_ms: u64) -> SceneResult<Self> {
        NonZeroU64::new(duration_ms)
            .map(|duration_ms| Self::Countdown { duration_ms })
            .ok_or(SceneError::InvalidTimerTransition)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScenePreset {
    id: SceneId,
    name: String,
    revision: PresetRevision,
    composition: SceneComposition,
    timer: TimerConfig,
}

impl ScenePreset {
    pub fn new(
        id: SceneId,
        name: impl Into<String>,
        revision: PresetRevision,
        composition: SceneComposition,
        timer: TimerConfig,
    ) -> SceneResult<Self> {
        let name = name.into();
        let valid_name = !name.is_empty()
            && name.len() <= 80
            && name.trim() == name
            && !name.chars().any(char::is_control);
        valid_name
            .then_some(Self {
                id,
                name,
                revision,
                composition,
                timer,
            })
            .ok_or(SceneError::InvalidSceneName)
    }

    pub fn id(&self) -> &SceneId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn revision(&self) -> PresetRevision {
        self.revision
    }

    pub fn composition(&self) -> SceneComposition {
        self.composition
    }

    pub fn timer(&self) -> TimerConfig {
        self.timer
    }

    pub(crate) fn is_consistent(&self) -> bool {
        self.id.is_consistent()
            && !self.name.is_empty()
            && self.name.len() <= 80
            && self.name.trim() == self.name
            && !self.name.chars().any(char::is_control)
            && self.composition.is_consistent()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenePresetWire {
    id: SceneId,
    name: String,
    revision: PresetRevision,
    composition: SceneComposition,
    timer: TimerConfig,
}

impl<'de> Deserialize<'de> for ScenePreset {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ScenePresetWire::deserialize(deserializer)?;
        Self::new(
            wire.id,
            wire.name,
            wire.revision,
            wire.composition,
            wire.timer,
        )
        .map_err(D::Error::custom)
    }
}

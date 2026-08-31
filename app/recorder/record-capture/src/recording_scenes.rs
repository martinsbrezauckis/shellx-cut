//! Public facade for the bounded Recording Scenes engine.
//!
//! Configuration admission, durable event ownership, and focused contract
//! tests intentionally live in separate small modules. The existing generic
//! reducer and journal remain their authority.

pub use crate::recording_scenes_config::{
    RecordingSceneConfig, RecordingSceneLayout, RecordingScenePipCorner, RecordingScenePipShape,
    RecordingScenePresetConfig, RecordingSceneTimerConfig,
};
pub use crate::recording_scenes_engine::{
    RecordingSceneEngine, RecordingSceneEngineError, RecordingSceneTimerAction,
    RECORDING_SCENE_PROJECTION_SCHEMA,
};
pub use crate::recording_scenes_projection::RecordingSceneProjection;

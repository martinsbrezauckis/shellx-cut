//! record-core — pure data model for ShellX Record.
//!
//! Role: the serializable types shared by every other crate (engine, render,
//! capture, cli). Holds NO platform code and NO heavy deps, so it builds on any
//! target (incl. headless WSL) and is trivially unit-testable.
//!
//! Modules:
//! - `event` — `EventTrack`: the captured input stream (cursor/click/scroll/key).
//! - `plan` — `EditPlan`: the auto-generated, non-destructive polish description
//!   (eased zoom keyframes, cursor style, frame, background, webcam …).
//! - `project` — `RecordingProject`: ties source media + event track + edit plan.
//! - `ease` — easing curves for keyframe interpolation.
//! - `color` — `Rgba` styling color.
//! - `error` — `RecordError`, mirroring ShellX Cut's `CutError` for clean integration.
//! - `fixtures` — synthetic `EventTrack` generators (headless test inputs).
//!
//! Primary callers: record-engine (reads EventTrack → writes EditPlan),
//! record-render (reads RecordingProject+EditPlan → MP4/GIF), record-cli.

pub mod cadence;
pub mod camera;
pub mod capture_quality;
pub mod capture_source;
pub mod color;
pub mod ease;
pub mod error;
pub mod event;
pub mod event_run_merge;
pub mod fixtures;
pub mod plan;
pub mod project;
pub mod scene;
mod scene_editable_timeline;
mod scene_editable_timeline_plan;
mod scene_editable_timeline_timer;
mod scene_model;
pub mod scene_projection;
pub mod scene_reducer;
mod scene_types;

#[cfg(test)]
mod event_run_merge_tests;
#[cfg(test)]
mod scene_projection_tests;
#[cfg(test)]
mod scene_switch_tests;
#[cfg(test)]
mod scene_tests;

/// Project file schema tag (written into `RecordingProject.schema`).
pub const SCHEMA: &str = "shellx-record/1";

pub use cadence::{
    backend_fps_v1, backend_requested_v1, CadenceError, CaptureCadence, FrameRate,
    ProbedMediaCadence, CAPTURE_CADENCE_SCHEMA,
};
pub use camera::{
    CameraArtifact, CameraClockRange, CameraMediaFacts, CameraTerminalState, CAMERA_ARTIFACT_SCHEMA,
};
pub use capture_quality::{
    CaptureOutputSize, CaptureQualityProfile, CaptureQualityRequest, CaptureQualityResolution,
    CAPTURE_QUALITY_SCHEMA,
};
pub use capture_source::{CaptureSourceFacts, CAPTURE_SOURCE_FACTS_SCHEMA};
pub use color::Rgba;
pub use ease::Ease;
pub use error::{error_codes, RecordError, Result};
pub use event::{
    ClickPositionQuality, ClickSample, CursorCoordinateSource, CursorCoordinateState,
    CursorCorrelation, CursorSample, EventTrack, KeySample, Monitor, MouseButton, ScrollSample,
};
pub use event_run_merge::{merge_sealed_event_tracks, MergedEventTrack, SealedEventTrackRun};
pub use plan::{
    Anchor, Background, CaptionStyle, ClickFx, CursorStyle, EditPlan, FrameStyle, KeyCastEvent,
    Reframe, Shadow, WebcamKeyframe, WebcamOverlay, WebcamPlacement, WebcamShape, ZoomKey,
    ZoomTrack,
};
pub use project::{RecordingProject, Settings};
pub use scene::{
    AcceptedStartSnapshot, CatalogRevision, PipCorner, PipShape, PipSizePercent, PresenterPip,
    PresetRevision, SceneCatalog, SceneComposition, SceneError, SceneEvent, SceneEventKind,
    SceneId, SceneResult, SceneSnapshotRevision, TimerConfig, MAX_SCENES,
};
pub use scene_reducer::{
    reduce_scene_event, replay_scene_events, SceneState, TimerPhase, TimerReading,
};

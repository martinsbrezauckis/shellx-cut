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

pub mod color;
pub mod ease;
pub mod error;
pub mod event;
pub mod fixtures;
pub mod plan;
pub mod project;

/// Project file schema tag (written into `RecordingProject.schema`).
pub const SCHEMA: &str = "shellx-record/1";

pub use color::Rgba;
pub use ease::Ease;
pub use error::{error_codes, RecordError, Result};
pub use event::{
    ClickPositionQuality, ClickSample, CursorCoordinateSource, CursorCoordinateState,
    CursorCorrelation, CursorSample, EventTrack, KeySample, Monitor, MouseButton, ScrollSample,
};
pub use plan::{
    Anchor, Background, CaptionStyle, ClickFx, CursorStyle, EditPlan, FrameStyle, KeyCastEvent,
    Reframe, Shadow, WebcamKeyframe, WebcamOverlay, WebcamPlacement, WebcamShape, ZoomKey,
    ZoomTrack,
};
pub use project::{RecordingProject, Settings};

//! Persisted variable-speed ramp model and bounded metadata admission.

use super::SpeedRampPoint;
use serde::{Deserialize, Serialize};

/// Variable-speed time remap (the `edit.speed_ramp` verb's storage). A piecewise-
/// LINEAR speed curve over the clip's source window, realized at EDL-derivation
/// time as `segments` contiguous CONSTANT-speed sub-segments (each a midpoint
/// sample of the curve over an equal slice of the source). More segments = a
/// smoother ramp at the cost of more filtergraph nodes. The points carry the
/// curve; `segments` the current grid-safe sampling granularity. New frame-aware
/// ramps also retain their bounded requested granularity so a temporary lower-FPS
/// format can reduce the effective filtergraph without permanently coarsening the
/// curve. Historic ramps have no retained preference and keep millisecond replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeedRamp {
    /// Control points, sorted strictly ascending by `at_ms` (≥ 2, enforced at verb
    /// time). The speed between points is linearly interpolated; before the first /
    /// after the last point the nearest factor is held.
    pub points: Vec<SpeedRampPoint>,
    /// Number of constant-speed sub-segments the curve is sampled into at render
    /// (2–120, after the active output grid's frame-safety cap).
    #[serde(deserialize_with = "crate::speed_ramp_timing::deserialize_segments")]
    pub segments: usize,
    /// Original bounded `segments` request for a frame-aware ramp. The effective
    /// [`Self::segments`] is recomputed from this preference on each project-format
    /// regrid, so lowering and then restoring the FPS restores the requested curve
    /// detail where the safety cap allows it. Absent on historic ramps and older
    /// frame-aware project caches; their existing effective value is used as the
    /// backward-compatible preference.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::speed_ramp_timing::deserialize_preferred_segments"
    )]
    pub preferred_segments: Option<usize>,
    /// Project frame rate resolved when the ramp is committed. New ramps use
    /// this frame grid for one authoritative duration; absent ramps preserve
    /// the historic millisecond interpretation on replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timebase_fps: Option<f64>,
    /// Project audio rate resolved with `timebase_fps`; keeps each ramp slice
    /// on the same sample budget as its frame budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timebase_audio_rate: Option<u32>,
}

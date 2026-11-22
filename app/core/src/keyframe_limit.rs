//! Shared ceiling for one animated parameter's control points.

/// FFmpeg 8's expression parser permits 100 nested parse calls. The renderer
/// nests one `if` per point, plus the surrounding filter and easing expression;
/// 64 leaves room for those wrappers while covering ordinary hand edits.
pub const MAX_KEYFRAME_POINTS: usize = 64;

/// Absolute per-clip redaction ceilings, shared by edits and saved-project renders.
pub const MAX_REDACTION_REGIONS: usize = 256;
pub const MAX_REDACTION_MOTION_POINTS: usize = 200_000;

/// Absolute total across all animated parameter tracks on one clip.
pub const MAX_KEYFRAME_TOTAL_POINTS: usize = MAX_REDACTION_MOTION_POINTS;

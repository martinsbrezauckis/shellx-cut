//! Shared ceiling for one animated parameter's control points.

/// FFmpeg 8's expression parser permits 100 nested parse calls. A renderer that
/// nests one `if` per point (the separate recorded camera look) stays at 64.
pub const MAX_KEYFRAME_POINTS: usize = 64;

/// A new `edit.keyframe` track. Exports read these curves from a per-frame
/// command file, so the point count no longer meets the parser's depth; this
/// ceiling is the longest curve the export measurement covers.
pub const MAX_EDIT_KEYFRAME_POINTS: usize = 10_000;

/// Absolute per-clip redaction ceilings, shared by edits and saved-project renders.
pub const MAX_REDACTION_REGIONS: usize = 256;
pub const MAX_REDACTION_MOTION_POINTS: usize = 200_000;

/// Absolute total across all animated parameter tracks on one clip.
pub const MAX_KEYFRAME_TOTAL_POINTS: usize = MAX_REDACTION_MOTION_POINTS;

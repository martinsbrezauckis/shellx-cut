//! cut-export error type. public verb contract requires actionable errors: clip id +
//! timecode + cause wherever they exist. Callers map these onto the verb
//! error envelope `{code, message, clip_id?, at_ms?, cause, suggested_action?}`.

use thiserror::Error;

/// All failure modes of the export serializers. Pure-function crate: no I/O
/// errors here — path fencing and file writes live in the verb layer.
#[derive(Debug, Error)]
pub enum ExportError {
    /// Input JSON did not match the timeline/op-log contract timeline shape.
    #[error("invalid timeline JSON: {0}")]
    BadInput(String),

    /// `export.xml` format arg outside the enum.
    #[error("unknown export format '{0}' (expected fcpxml|premiere|resolve|mlt)")]
    BadFormat(String),

    /// fps that is neither integer nor NTSC (X*1000/1001) — unrepresentable
    /// in xmeml's timebase+ntsc encoding, so reject everywhere for consistency.
    #[error("unsupported fps {0}: must be integer (24, 25, 30…) or NTSC (23.976, 29.97, 59.94)")]
    BadFps(f64),

    /// Clip quantized to zero/negative frames — sub-frame source range.
    #[error(
        "clip '{clip_id}': non-positive duration after frame quantization \
         (src_in_ms={src_in_ms}, src_out_ms={src_out_ms} @ {fps} fps); \
         extend the range to at least one frame"
    )]
    EmptyClip {
        clip_id: String,
        src_in_ms: u64,
        src_out_ms: u64,
        fps: f64,
    },

    /// Clip references an asset id missing from the assets map.
    #[error("clip '{clip_id}': asset '{asset}' not found in timeline assets")]
    MissingAsset { clip_id: String, asset: String },

    /// Media clip missing required fields (asset/src_in_ms/src_out_ms).
    #[error("clip '{clip_id}': {cause}")]
    BadClip { clip_id: String, cause: String },

    /// Resolve's FCPXML import cannot address audio stream
    /// indexes > 0; erroring loudly beats emitting a file Resolve mis-reads.
    #[error(
        "clip '{clip_id}': resolve export cannot address audio stream {stream} > 0; \
         extract that stream to a side-car WAV and re-point the clip first"
    )]
    ResolveStream { clip_id: String, stream: i32 },

    /// A millisecond timestamp/range cannot be represented in the selected
    /// frame timebase without overflowing the interchange frame address type.
    #[error("time value {ms}ms is too large for timebase {fps_num}/{fps_den}")]
    TimeOverflow { ms: u64, fps_num: i64, fps_den: i64 },

    /// Nothing exportable on the timeline.
    #[error("timeline has no video or audio clips to export")]
    EmptyTimeline,

    /// `export.srt` with no caption track / no caption clips.
    #[error("timeline has no caption track; run captions.generate first")]
    NoCaptions,

    /// `captions.import` could not parse a subtitle file (no usable cues).
    #[error("could not parse subtitle file: {0}")]
    BadSubtitle(String),
}

//! cut-media — ShellX Cut media engine (media-engine contract).
//!
//! Role: everything that touches actual media bytes, via ffmpeg/ffprobe
//! SUBPROCESSES only (no lib linking — license + simplicity):
//! - probe   → normalized MediaProbe JSON
//! - proxy   → 960×540 h264 + same audio (fast preset)
//! - render  → EDL → filter_complex concat graph, caption burn-in, gain;
//!             deterministic (fixed params, no wall-clock metadata)
//! - frame   → single composed frame as JPEG (the agent's eyes)
//! - captions→ ASS generation for burn-in (interchange SRT/XML export is
//!             owned by the cut-export crate; this
//!             crate keeps only render-side caption serialization)
//! Primary callers: server (cutd) verb handlers + job system.

pub mod captions;
pub mod color_match;
pub mod ffmpeg;
pub mod filmstrip;
pub mod gif;
pub mod hwencode;
mod image_cache;
pub mod loudness;
pub mod mask;
pub mod paths;
pub mod poster;
pub mod preview;
pub mod probe;
pub mod proxy;
pub mod reframe;
pub mod render;
pub mod scopes;
pub mod sync;
pub mod title;
pub mod toolpath;
pub mod waveform;

pub use captions::{captions_to_ass, captions_to_ass_for_edl, captions_to_ass_karaoke};
pub use paths::{PathFence, OVERWRITABLE_SUFFIXES};
pub use preview::{render_preview_incremental, PreviewResult};
pub use probe::{probe, MediaProbe};
pub use proxy::{make_proxy, make_proxy_with_progress};
pub use render::{
    extract_frame, extract_scrub_frame, plan_scrub_frame, render_final, render_preview, Fit,
    ProgressFn, RenderOptions, RenderOutput, RenderPreset, Resolution, ScrubPlan,
    SCRUB_DEFAULT_HEIGHT,
};

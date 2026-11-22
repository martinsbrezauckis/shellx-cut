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

mod atomic_output;
pub use atomic_output::check_output_dir_policy;
cut_core::isolate_test_process!();

pub mod cache_trust;
pub mod captions;
pub mod color_match;
pub mod decode_check;
pub mod ffmpeg;
pub mod filmstrip;
pub mod folder_access;
pub mod gif;
pub mod host_memory;
pub mod hwencode;
mod hwencode_args;
pub mod hwencode_limits;
pub mod hwencode_runtime;
mod image_cache;
mod internal_files;
pub mod loudness;
pub mod mask;
pub mod paths;
pub mod poster;
pub mod preview;
pub mod private_work;
pub mod probe;
pub mod process_start;
pub mod proxy;
pub mod reframe;
pub mod render;
pub mod scopes;
pub mod sync;
pub mod title;
pub mod tool_setup;
pub mod toolpath;
pub mod waveform;

pub use captions::{captions_to_ass, captions_to_ass_for_edl, captions_to_ass_karaoke};
pub use paths::{PathFence, OVERWRITABLE_SUFFIXES};
pub use preview::{render_preview_incremental, PreviewResult};
pub use probe::{probe, probe_source, MediaProbe};
pub use proxy::{
    make_proxy, make_proxy_with_progress, make_proxy_with_progress_and_tone_map,
    make_proxy_with_progress_and_tone_map_bound,
};
pub use render::{
    extract_frame, extract_scrub_frame, plan_scrub_frame, render_final, render_preview, Fit,
    ProgressFn, RenderOptions, RenderOutput, RenderPreset, Resolution, ScrubPlan,
    SCRUB_DEFAULT_HEIGHT,
};

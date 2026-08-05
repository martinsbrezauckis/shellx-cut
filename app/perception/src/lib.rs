//! cut-perception — ShellX Cut perception crate (perception contract).
//!
//! Role: gives the agent measured FACTS about media instead of vibes:
//! - sidecar — orchestrates py/instruments.py (whisperX words, silero-vad
//!   silences, PySceneDetect scenes, lightweight energy beats, ebur128 loudness);
//!   validates + caches by asset hash → receipts/<asset>.perception.json
//! - types   — the typed perception.json shape (ms everywhere)
//! - checks  — the verify.checks battery in pure Rust over facts + EDL
//!   (cut_on_word, lufs, caption_presence, black_or_frozen_frames,
//!   uniform_border, silence_at_edges, duration_matches_edl), interpreted
//!   under a FootageProfile (talking_head | silent_screen_demo — the footage-profile contract;
//!   uniform_border gates on BOTH profiles)
//! Primary callers: server (cutd) job system + verify verbs; e2e.

pub mod candidates;
pub mod checks;
pub mod diarize;
pub mod sidecar;
pub mod types;

pub use candidates::{clip_candidates, CandidateOpts, ClipCandidate};
pub use checks::{
    brand_check, caption_qc, delivery, pacing, pregate, propose_profile, run_all,
    run_all_with_profile, uniform_border, BrandSpec, CaptionQcOpts, DeliveryOpts, FootageProfile,
    PregateOpts, PregateReport, PregateRisk, ProfileProposal, RenderFacts,
    CUT_ON_WORD_TOLERANCE_MS, FOOTAGE_PROFILE_CHECK, STUCK_RENDER_MIN_FRACTION,
    UNIFORM_BORDER_MAX_INSET_PX,
};
pub use diarize::{apply_diarization, assign_word_speakers, ASSIGN_TOL_MS};
pub use sidecar::{
    appdata_sidecar_dir, build_contact_sheet, build_qc_sheet, configured_sidecar_python,
    load_report, read_stt_setting, run_instruments, run_instruments_progress, run_subject,
    sidecar_paths, stt_settings_path, transcribe, transcribe_progress, write_stt_setting,
    InstrumentSet, SidecarProgress,
};
pub use types::{
    BeatGrid, ContentBbox, Diarization, Loudness, LoudnessWindow, PerceptionReport, SceneCut,
    SilenceSpan, SpeakerTurn, Transcript, VideoSpan, WordSpan, PERCEPTION_SCHEMA,
};

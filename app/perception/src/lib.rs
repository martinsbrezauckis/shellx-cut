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
//! Primary callers: server (cutd) job system + verify verbs.

pub mod analysis_attempts;
pub mod applicability;
pub mod candidates;
cut_core::isolate_test_process!();
pub mod checks;
pub mod diarize;
pub mod export_check_list;
pub mod output_checks;
pub mod prepared_stt;
pub mod sidecar;
pub mod types;

pub use applicability::{
    apply_request, apply_scope, timeline_scope, MeasuredSources, Requested, SourceFacts,
    TimelineScope,
};
pub use candidates::{clip_candidates, CandidateOpts, ClipCandidate};
pub use checks::{
    brand_check, caption_qc, delivery, pacing, pregate, propose_profile, run_all,
    run_all_for_export, run_all_with_profile, uniform_border, BrandSpec, CaptionQcOpts,
    DeliveryOpts, FootageProfile, PregateOpts, PregateReport, PregateRisk, ProfileProposal,
    RenderFacts, CUT_ON_WORD_TOLERANCE_MS, FOOTAGE_PROFILE_CHECK, STUCK_RENDER_MIN_FRACTION,
    UNIFORM_BORDER_MAX_INSET_PX,
};
pub use diarize::{apply_diarization, assign_word_speakers, ASSIGN_TOL_MS};
pub use output_checks::{output_checks_with_profile, OutputChecks};
pub use prepared_stt::{prepared_stt_model, PreparedSttModel};
pub use sidecar::captions_installation;
pub use sidecar::{
    appdata_sidecar_dir, apply_python_command_policy, build_contact_sheet,
    build_contact_sheet_owned, build_contact_sheet_recorded, build_qc_sheet, build_qc_sheet_owned,
    build_qc_sheet_recorded, captions_not_installed_error, captions_setup_action,
    configured_sidecar_python, effective_stt_selection, is_captions_not_installed, load_report,
    native_runtime_context, premium_sidecar_runtime, read_stt_setting, run_instruments,
    run_instruments_owned, run_instruments_owned_ephemeral, run_instruments_owned_progress,
    run_instruments_progress, run_subject, run_subject_owned, run_subject_recorded, sidecar_paths,
    sidecar_runtime, stt_settings_path, transcribe, transcribe_owned_progress, transcribe_progress,
    write_stt_setting, EffectiveSttSelection, InstrumentSet, SidecarProgress, SidecarRuntime,
};
pub use types::{
    BeatGrid, ContentBbox, Diarization, Loudness, LoudnessWindow, PerceptionReport, SceneCut,
    SilenceSpan, SpeakerTurn, Transcript, VideoSpan, WordSpan, PERCEPTION_SCHEMA,
};

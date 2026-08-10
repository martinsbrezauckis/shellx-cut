//! dispatch.rs — the single verb dispatcher under the zero-local-mutation contract.
//!
//! Role: EVERY surface funnels here — REST POST /api/verb/{name}, MCP
//! tools/call, and the `cutd verb` CLI all call `dispatch()`. No other code
//! path may mutate the project. Unknown verbs are rejected against the
//! registry; arg shapes are the verb's responsibility (handlers parse with
//! serde and return invalid_args with the parse error as cause).
//!
//! Current status: ALL verbs are wired against the committed core/media/perception
//! signatures. Where a dependency crate fn is still `todo!()`, the call is
//! guarded (`guard_call` / `run_blocking`) so the panic becomes a structured
//! `unimplemented` error naming the missing dependency — the server NEVER
//! panics on a verb, and each verb becomes live when its dependency is
//! implemented (zero dispatcher changes needed).
//!
//! Op-emission rules (public verb contract): every mutating verb funnels
//! through cut-core's commit paths — `ProjectStore::apply` for core edit.*
//! verbs (recompute-by-replay), `apply_lowered` for higher-layer verbs
//! (transcript.*, captions.* — recorded `lowered` steps keep replay/diff
//! working), `record_import`/`checkpoint`/`revert` for their special ops —
//! each committing the OpRecord + saving the cache; this module publishes
//! `op_applied` after every commit. media.import and project.checkpoint ARE
//! ops; project.revert appends one atomic materialized timeline op.
//!
//! Dependencies: state.rs, jobs.rs, events.rs, ui_bridge.rs, cut-core,
//! cut-media, cut-perception. Primary callers: http.rs, mcp.rs, main.rs.

use crate::events::Event;
use crate::generate_handlers;
use crate::output_paths::{
    fence_output_path, fence_project_output_path, fenced_existing_export_read,
    fenced_existing_file_under_dir, make_fence, publish_output_atomic,
    resolve_existing_project_file, temp_output_path_for_render, write_output_atomic,
    OutputPathPolicy,
};
use crate::recipes;
use crate::registry::verb_contract::DispatchTarget;
use crate::state::AppState;
use cut_core::{
    error_codes, Actor, CutError, InverseOp, OpEffect, OpRecord, ProjectStore, VerbResult,
};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

pub(crate) const ENV_ADAPTER_PYTHON: &str = "CUTD_ADAPTER_PYTHON";

pub(crate) fn configured_adapter_python() -> Option<PathBuf> {
    let explicit = std::env::var_os(ENV_ADAPTER_PYTHON)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from);
    adapter_python_for_platform(
        explicit,
        cut_perception::configured_sidecar_python(),
        find_python_on_path(),
        cfg!(target_os = "macos"),
    )
}

fn adapter_python_for_platform(
    explicit: Option<PathBuf>,
    managed: Option<PathBuf>,
    path_python: Option<PathBuf>,
    is_macos: bool,
) -> Option<PathBuf> {
    if explicit.is_some() {
        return explicit;
    }
    if managed.is_some() {
        return managed;
    }
    if is_macos {
        return None;
    }
    path_python
}

fn find_python_on_path() -> Option<PathBuf> {
    find_executable_on_path(&["python3", "python"])
}

fn find_executable_on_path(names: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
            #[cfg(windows)]
            {
                let exe = dir.join(format!("{name}.exe"));
                if exe.is_file() {
                    return Some(exe);
                }
            }
        }
    }
    None
}

/// Dispatch one verb. `actor` records who/which surface for the op-log.
/// Returns the universal envelope; never panics on bad input.
pub async fn dispatch(state: &AppState, name: &str, args: Value, actor: Actor) -> VerbResult {
    // Contract gate: only verbs in schema/verbs.json exist.
    let Some(spec) = state.registry.get(name) else {
        return VerbResult::err(CutError::new(
            error_codes::NOT_FOUND,
            format!("unknown verb '{name}'"),
            "verb is not in schema/verbs.json — see GET /api/verbs for the registry",
        ));
    };
    // Compile-once input contract gate. This runs before every route, including
    // nested recipe/plugin dispatch, so REST, CLI and MCP cannot diverge.
    if let Err(e) = state.registry.validate_args(spec, &args) {
        return VerbResult::err(e);
    }
    let dispatch_target = spec.behavior.dispatch;
    let prepared = match crate::request_control::prepare(name, args, actor) {
        Ok(prepared) => prepared,
        Err(error) => return VerbResult::err(error),
    };
    if prepared.controlled {
        let _request = state.request_gate.lock().await;
        match crate::request_control::preflight(state, name, &prepared.actor).await {
            Ok(Some(response)) => return response,
            Ok(None) => {}
            Err(error) => return VerbResult::err(error),
        }
        return dispatch_validated(
            state,
            name,
            dispatch_target,
            prepared.args,
            prepared.actor,
            true,
        )
        .await;
    }
    dispatch_validated(
        state,
        name,
        dispatch_target,
        prepared.args,
        prepared.actor,
        false,
    )
    .await
}

async fn dispatch_validated(
    state: &AppState,
    name: &str,
    dispatch_target: DispatchTarget,
    args: Value,
    actor: Actor,
    publish_request_receipt: bool,
) -> VerbResult {
    let request_actor = actor.clone();
    // `DispatchTarget` is generated from schema/verbs.json. This match has no
    // catch-all arm on purpose: the compiler proves every schema target has a
    // corresponding handler, and registry loading verifies every verb names
    // exactly one generated target.
    let mut result: VerbResult = match dispatch_target {
        // ------------------------------------------------------------------
        // project.*
        // ------------------------------------------------------------------
        DispatchTarget::ProjectCreate => project_create(state, args, actor).await.into(),
        DispatchTarget::ProjectOpen => project_open(state, args).await.into(),
        DispatchTarget::ProjectList => project_list(args).await.into(),
        DispatchTarget::ProjectForget => project_forget(args).await.into(),
        DispatchTarget::ProjectDelete => project_delete(state, args).await.into(),
        DispatchTarget::ProjectSave => project_save(state).await.into(),
        DispatchTarget::ProjectState => project_state(state, args).await.into(),
        DispatchTarget::ProjectHealth => project_health(state, args).await.into(),
        DispatchTarget::ProjectSequenceList => project_sequence_list(state).await.into(),
        DispatchTarget::ProjectSequenceIndex => project_sequence_index(state, args).await.into(),
        DispatchTarget::ProjectSequenceCreate => {
            project_sequence_create(state, args, actor).await.into()
        }
        DispatchTarget::ProjectSequenceSwitch => {
            project_sequence_switch(state, args, actor).await.into()
        }
        DispatchTarget::ProjectSequenceRename => {
            project_sequence_rename(state, args, actor).await.into()
        }
        DispatchTarget::ProjectSequenceDelete => {
            project_sequence_delete(state, args, actor).await.into()
        }
        DispatchTarget::ProjectOps => project_ops(state, args).await.into(),
        DispatchTarget::ProjectClose => project_close(state).await.into(),
        DispatchTarget::ProjectCheckpoint => project_checkpoint(state, args, actor).await.into(),
        DispatchTarget::ProjectRename => project_rename(state, args, actor).await.into(),
        DispatchTarget::ProjectFormat => project_format(state, args, actor).await.into(),
        DispatchTarget::ProjectColor => project_color(state, args, actor).await.into(),
        DispatchTarget::ProjectBrand => project_brand(state, args, actor).await.into(),
        DispatchTarget::ProjectSetOutputDir => crate::output_paths::project_set_output_dir(args)
            .await
            .into(),
        DispatchTarget::CommentAdd => comment_add(state, args, actor).await.into(),
        DispatchTarget::CommentExport => comment_export(state, args).await.into(),
        DispatchTarget::CommentImport => comment_import(state, args, actor).await.into(),
        DispatchTarget::CommentList => comment_list(state, args).await.into(),
        DispatchTarget::CommentResolve => comment_resolve(state, args, actor).await.into(),
        DispatchTarget::CommentDraft => comment_draft(state, args, actor).await.into(),
        DispatchTarget::CommentApply => comment_apply(state, args, actor).await.into(),
        DispatchTarget::ProjectRevert => project_revert(state, args, actor).await.into(),
        DispatchTarget::ProjectUndo => project_undo(state, actor).await.into(),
        DispatchTarget::ProjectRedo => project_redo(state, actor).await.into(),
        DispatchTarget::ProjectDiff => project_diff(state, args).await.into(),
        DispatchTarget::LibraryList => library_list(args).await.into(),
        DispatchTarget::LibraryAdd => library_add(state, args).await.into(),
        DispatchTarget::LibraryRelink => library_relink(args).await.into(),
        DispatchTarget::LibraryRemove => library_remove(args).await.into(),
        DispatchTarget::LibraryMove => library_move(args).await.into(),
        DispatchTarget::LibraryTag => library_tag(args).await.into(),
        DispatchTarget::LibraryFavorite => library_favorite(args).await.into(),
        DispatchTarget::LibraryUse => library_use(args).await.into(),
        DispatchTarget::LibraryAddToProject => {
            library_add_to_project(state, args, actor).await.into()
        }
        DispatchTarget::LibraryFolderAdd => library_folder_add(args).await.into(),
        DispatchTarget::LibraryFolderRename => library_folder_rename(args).await.into(),
        DispatchTarget::LibraryFolderRemove => library_folder_remove(args).await.into(),

        // ------------------------------------------------------------------
        // media.* — import kicks the probe→proxy→transcribe→perception chain
        // ------------------------------------------------------------------
        DispatchTarget::MediaImport => media_import(state, args, actor).await.into(),
        DispatchTarget::MediaRemove => media_remove(state, args, actor).await.into(),
        DispatchTarget::MediaRelink => media_relink(state, args, actor).await.into(),
        DispatchTarget::MediaCheck => media_check(state, args).await.into(),
        DispatchTarget::MediaBinSave => media_bin_save(state, args, actor).await.into(),
        DispatchTarget::MediaBinDelete => media_bin_delete(state, args, actor).await.into(),
        DispatchTarget::MediaBinList => media_bin_list(state, args).await.into(),
        DispatchTarget::MediaProbe => media_probe(state, args).await.into(),
        DispatchTarget::MediaWaveform => media_waveform(state, args).await.into(),
        DispatchTarget::MediaFilmstrip => media_filmstrip(state, args).await.into(),
        DispatchTarget::MediaTranscribe => media_transcribe(state, args).await.into(),
        DispatchTarget::MediaPerception => media_perception(state, args).await.into(),
        DispatchTarget::MediaDiarize => crate::diarize::media_diarize(state, args).await.into(),

        // ------------------------------------------------------------------
        // jobs.* (the background-job contract: jobs domain replaces media.status)
        // ------------------------------------------------------------------
        DispatchTarget::JobsStatus => jobs_status(state, args).await.into(),
        DispatchTarget::JobsList => jobs_list(state).await.into(),
        DispatchTarget::JobsCancel => jobs_cancel(state, args).await.into(),

        // ------------------------------------------------------------------
        // edit.* — thin arg-parse wrappers over cut_core::edit, one op each
        // ------------------------------------------------------------------
        DispatchTarget::EditSplit => edit_split(state, args, actor).await.into(),
        DispatchTarget::EditCutToBeat => edit_cut_to_beat(state, args, actor).await.into(),
        DispatchTarget::EditSplitAtScenes => edit_split_at_scenes(state, args, actor).await.into(),
        DispatchTarget::EditMarkScenes => edit_mark_scenes(state, args, actor).await.into(),
        DispatchTarget::EditTrimEdges => edit_trim_edges(state, args, actor).await.into(),
        DispatchTarget::EditRippleDelete => edit_ripple_delete(state, args, actor).await.into(),
        DispatchTarget::EditTrim => edit_trim(state, args, actor).await.into(),
        DispatchTarget::EditSpeed => edit_speed(state, args, actor).await.into(),
        DispatchTarget::EditSpeedRamp => edit_speed_ramp(state, args, actor).await.into(),
        DispatchTarget::EditMove => edit_move(state, args, actor).await.into(),
        DispatchTarget::EditInsert => edit_insert(state, args, actor).await.into(),
        DispatchTarget::EditDuplicate => edit_duplicate(state, args, actor).await.into(),
        DispatchTarget::EditNest => edit_nest(state, args, actor).await.into(),
        DispatchTarget::EditReplace => edit_replace(state, args, actor).await.into(),
        DispatchTarget::EditFitToFill => edit_fit_to_fill(state, args, actor).await.into(),
        DispatchTarget::EditDetachAudio => edit_detach_audio(state, args, actor).await.into(),
        DispatchTarget::EditSplitEdit => edit_split_edit(state, args, actor).await.into(),
        DispatchTarget::EditPaste => edit_paste(state, args, actor).await.into(),
        DispatchTarget::EditGain => edit_gain(state, args, actor).await.into(),
        DispatchTarget::EditFade => edit_fade(state, args, actor).await.into(),
        DispatchTarget::EditMuteRange => edit_mute_range(state, args, actor).await.into(),
        DispatchTarget::EditTransform => edit_transform(state, args, actor).await.into(),
        DispatchTarget::EditCrop => edit_crop(state, args, actor).await.into(),
        DispatchTarget::EditGrade => edit_grade(state, args, actor).await.into(),
        DispatchTarget::EditGradeStack => edit_grade_stack(state, args, actor).await.into(),
        DispatchTarget::EditGradeWindow => edit_grade_window(state, args, actor).await.into(),
        DispatchTarget::GradeSave => grade_save(state, args, actor).await.into(),
        DispatchTarget::GradeApply => grade_apply(state, args, actor).await.into(),
        DispatchTarget::GradeList => grade_list(state).await.into(),
        DispatchTarget::EditColorSpace => edit_color_space(state, args, actor).await.into(),
        DispatchTarget::EditColorMatch => edit_color_match(state, args, actor).await.into(),
        DispatchTarget::EditAutoBalance => edit_auto_balance(state, args, actor).await.into(),
        DispatchTarget::EditMatte => edit_matte(state, args, actor).await.into(),
        DispatchTarget::EditEffect => edit_effect(state, args, actor).await.into(),
        DispatchTarget::EditAdjustment => edit_adjustment(state, args, actor).await.into(),
        DispatchTarget::EditReverse => edit_reverse(state, args, actor).await.into(),
        DispatchTarget::EditStabilize => edit_stabilize(state, args, actor).await.into(),
        DispatchTarget::EditFreeze => edit_freeze(state, args, actor).await.into(),
        DispatchTarget::EditAnimate => edit_animate(state, args, actor).await.into(),
        DispatchTarget::EditKeyframe => edit_keyframe(state, args, actor).await.into(),
        DispatchTarget::EditAutoZoom => edit_auto_zoom(state, args, actor).await.into(),
        DispatchTarget::EditAddMask => edit_add_mask(state, args, actor).await.into(),
        DispatchTarget::EditRedact => edit_redact(state, args, actor).await.into(),
        DispatchTarget::EditTrack => edit_track(state, args).await.into(),
        DispatchTarget::EditMulticamSync => edit_multicam_sync(state, args).await.into(),
        DispatchTarget::EditMulticamSwitch => edit_multicam_switch(state, args, actor).await.into(),
        DispatchTarget::EditEq => edit_eq(state, args, actor).await.into(),
        DispatchTarget::AudioCleanupVoice => audio_cleanup_voice(state, args, actor).await.into(),
        DispatchTarget::EditSlide => edit_slide(state, args, actor).await.into(),
        DispatchTarget::TitleAdd => title_add(state, args, actor).await.into(),
        DispatchTarget::TitleUpdate => title_update(state, args, actor).await.into(),
        DispatchTarget::TitleTemplates => title_templates(state, args, actor).await.into(),
        DispatchTarget::EditAddShape => edit_add_shape(state, args, actor).await.into(),
        DispatchTarget::ShapeUpdate => shape_update(state, args, actor).await.into(),
        DispatchTarget::EditSeekMarker => edit_seek_marker(state, args, actor).await.into(),
        DispatchTarget::AssetsProviders => assets_providers(state, args, actor).await.into(),
        DispatchTarget::AssetsSearch => assets_search(state, args, actor).await.into(),
        DispatchTarget::AssetsFetch => assets_fetch(state, args, actor).await.into(),
        DispatchTarget::AssetsGenerate => assets_generate(state, args, actor).await.into(),
        DispatchTarget::AssetsGeneratedList => generated_assets::assets_generated_list(state, args)
            .await
            .into(),
        DispatchTarget::AgentChat => agent_chat(state, args, actor).await.into(),
        DispatchTarget::PluginsList => plugins_list(state, args, actor).await.into(),
        DispatchTarget::PluginsEnable => plugins_enable(state, args, actor).await.into(),
        DispatchTarget::PluginsCall => plugins_call(state, args, actor).await.into(),
        DispatchTarget::MediaIndexStatus => media_index_status(state, args).await.into(),
        DispatchTarget::MediaSearch => media_search(state, args, actor).await.into(),
        DispatchTarget::MediaIndex => media_index(state, args, actor).await.into(),
        DispatchTarget::EffectsList => effects_list(state, args, actor).await.into(),
        DispatchTarget::TransitionsList => transitions_list(state, args, actor).await.into(),
        DispatchTarget::CaptionsKinetic => captions_kinetic(state, args, actor).await.into(),
        DispatchTarget::EditCrossfade => edit_crossfade(state, args, actor).await.into(),
        DispatchTarget::EditDuck => edit_duck(state, args, actor).await.into(),
        DispatchTarget::EditAddTrack => edit_add_track(state, args, actor).await.into(),
        DispatchTarget::EditRemoveTrack => edit_remove_track(state, args, actor).await.into(),
        DispatchTarget::EditReorderTrack => edit_reorder_track(state, args, actor).await.into(),
        DispatchTarget::EditBlend => edit_blend(state, args, actor).await.into(),
        DispatchTarget::EditTrackVisible => edit_track_visible(state, args, actor).await.into(),
        DispatchTarget::EditTrackLock => edit_track_lock(state, args, actor).await.into(),
        DispatchTarget::EditMute => edit_mute(state, args, actor).await.into(),
        DispatchTarget::EditSolo => edit_solo(state, args, actor).await.into(),
        DispatchTarget::EditPan => edit_pan(state, args, actor).await.into(),
        DispatchTarget::EditSlip => edit_slip(state, args, actor).await.into(),
        DispatchTarget::EditRoll => edit_roll(state, args, actor).await.into(),
        DispatchTarget::EditSlideEdit => edit_slide_edit(state, args, actor).await.into(),
        DispatchTarget::EditPasteAttributes => {
            crate::paste_attributes::edit_paste_attributes(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::EditAddMarker => edit_add_marker(state, args, actor).await.into(),
        DispatchTarget::EditRemoveMarker => edit_remove_marker(state, args, actor).await.into(),
        DispatchTarget::EditMoveMarker => edit_move_marker(state, args, actor).await.into(),
        DispatchTarget::EditUpdateMarker => edit_update_marker(state, args, actor).await.into(),
        DispatchTarget::EditRestore => edit_restore(state, args, actor).await.into(),

        // ------------------------------------------------------------------
        // audio.* — music bed with auto-duck + beat markers
        // ------------------------------------------------------------------
        DispatchTarget::AudioAddMusic => audio_add_music(state, args, actor).await.into(),
        DispatchTarget::AudioDub => crate::dub::audio_dub(state, args, actor).await.into(),

        // ------------------------------------------------------------------
        // transcript.* — transcript-driven edits (one op per removed span)
        // ------------------------------------------------------------------
        DispatchTarget::TranscriptGet => speech_text::transcript_get(state, args).await.into(),
        DispatchTarget::TranscriptTimeline => {
            speech_text::transcript_timeline(state, args).await.into()
        }
        DispatchTarget::TranscriptCutWords => speech_text::transcript_cut_words(state, args, actor)
            .await
            .into(),
        DispatchTarget::TranscriptIgnoreWords => {
            speech_text::transcript_ignore_words(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::TranscriptMuteWords => {
            speech_text::transcript_mute_words(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::TranscriptAssemble => speech_text::transcript_assemble(state, args, actor)
            .await
            .into(),
        DispatchTarget::TranscriptSearch => {
            speech_text::transcript_search(state, args).await.into()
        }
        DispatchTarget::TranscriptChapters => {
            speech_text::transcript_chapters(state, args).await.into()
        }
        DispatchTarget::TranscriptRemoveSilences => {
            speech_text::transcript_remove_silences(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::TranscriptRemoveFillers => {
            speech_text::transcript_remove_fillers(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::TranscriptRemoveRetakes => {
            speech_text::transcript_remove_retakes(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::TranscriptTranslate => {
            speech_text::transcript_translate(state, args).await.into()
        }

        // ------------------------------------------------------------------
        // captions.*
        // ------------------------------------------------------------------
        DispatchTarget::CaptionsGenerate => captions_generate(state, args, actor).await.into(),
        DispatchTarget::CaptionsTranslate => captions_translate(state, args, actor).await.into(),
        DispatchTarget::CaptionsImport => captions_import(state, args, actor).await.into(),
        DispatchTarget::CaptionsAddText => speech_text::captions_add_text(state, args, actor)
            .await
            .into(),
        DispatchTarget::CaptionsSetStyle => speech_text::captions_set_style(state, args, actor)
            .await
            .into(),
        DispatchTarget::CaptionsSaveStyle => speech_text::captions_save_style(state, args, actor)
            .await
            .into(),
        DispatchTarget::CaptionsApplyStyle => speech_text::captions_apply_style(state, args, actor)
            .await
            .into(),
        DispatchTarget::CaptionsListStyles => {
            speech_text::captions_list_styles(state, args).await.into()
        }
        DispatchTarget::CaptionsReflow => captions_reflow(state, args, actor).await.into(),
        DispatchTarget::CaptionsShift => captions_shift(state, args, actor).await.into(),
        DispatchTarget::CaptionsSetRange => captions_set_range(state, args, actor).await.into(),
        DispatchTarget::CaptionsSetText => captions_set_text(state, args, actor).await.into(),

        // ------------------------------------------------------------------
        // render.* / verify.* / export.*
        // ------------------------------------------------------------------
        DispatchTarget::RenderPreview => render_preview(state, args).await.into(),
        DispatchTarget::RenderFrame => render_frame(state, args).await.into(),
        DispatchTarget::RenderStoryboard => render_storyboard(state, args).await.into(),
        DispatchTarget::RenderFinal => render_final(state, args, actor).await.into(),
        DispatchTarget::RenderReframe => render_reframe(state, args, actor).await.into(),
        DispatchTarget::RenderDirect => render_direct(state, args, actor).await.into(),
        DispatchTarget::RenderQc => render_qc(state, args, actor).await.into(),
        DispatchTarget::VerifyChecks => verify_checks(state, args).await.into(),
        DispatchTarget::VerifyRerun => verify_rerun(state, args).await.into(),
        DispatchTarget::VerifyPacing => verify_pacing(state).await.into(),
        DispatchTarget::VerifyCaptions => verify_captions(state, args).await.into(),
        DispatchTarget::VerifyDelivery => verify_delivery(state, args).await.into(),
        DispatchTarget::VerifyLoudness => verify_loudness(state, args).await.into(),
        DispatchTarget::VerifyScopes => verify_scopes(state, args).await.into(),
        DispatchTarget::VerifyBrand => verify_brand(state, args).await.into(),
        DispatchTarget::VerifyJudge => verify_judge(state, args).await.into(),
        DispatchTarget::VerifyPregate => verify_pregate(state, args).await.into(),
        DispatchTarget::ExportFrame => export_frame(state, args, actor).await.into(),
        DispatchTarget::ExportRange => export_range(state, args, actor).await.into(),
        DispatchTarget::ExportAudio => export_audio(state, args, actor).await.into(),
        DispatchTarget::ExportPublish => export_publish(state, args, actor).await.into(),
        DispatchTarget::ExportGif => export_gif(state, args, actor).await.into(),
        DispatchTarget::ExportXml => export_xml(state, args).await.into(),
        DispatchTarget::ExportOtio => export_otio(state, args).await.into(),
        DispatchTarget::ExportEdl => export_edl(state, args).await.into(),
        DispatchTarget::ImportOtio => import_otio(state, args, actor).await.into(),
        DispatchTarget::ExportSrt => export_srt(state, args).await.into(),
        DispatchTarget::ExportVtt => export_vtt(state, args).await.into(),
        DispatchTarget::ExportAss => export_ass(state, args).await.into(),
        DispatchTarget::ExportTranscript => export_transcript(state, args).await.into(),
        DispatchTarget::ExportChapters => export_chapters(state, args).await.into(),

        // ------------------------------------------------------------------
        // clip.* — social repurposing: rank shareable windows + bundle
        // ------------------------------------------------------------------
        DispatchTarget::ClipCandidates => clip_candidates(state, args).await.into(),
        DispatchTarget::RenderBundle => render_bundle(state, args, actor).await.into(),
        DispatchTarget::RenderQueue => render_queue(state, args, actor).await.into(),
        DispatchTarget::AutopilotRun => autopilot_run(state, args, actor).await.into(),

        // ------------------------------------------------------------------
        // generate.* — native Generate module foundation. Pure catalog reads,
        // non-mutating previews, native inserts, prompt planning, and storyboard
        // planning all live in one domain.
        // ------------------------------------------------------------------
        DispatchTarget::GenerateList => generate_handlers::generate_list(args).await.into(),
        DispatchTarget::GenerateDescribe => generate_handlers::generate_describe(args).await.into(),
        DispatchTarget::GeneratePreview => generate_handlers::generate_preview(state, args)
            .await
            .into(),
        DispatchTarget::GenerateInsert => generate_handlers::generate_insert(state, args, actor)
            .await
            .into(),
        DispatchTarget::GenerateFromPrompt => {
            generate_handlers::generate_from_prompt(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::GenerateStoryboard => {
            generate_handlers::generate_storyboard(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::MotionTemplateToCut => {
            crate::motion_bridge::motion_template_to_cut(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::MotionScriptToCut => {
            crate::motion_bridge::motion_script_to_cut(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::MotionJobGet => crate::motion_jobs::get(state, args).await.into(),
        DispatchTarget::MotionJobList => crate::motion_jobs::list(state, args).await.into(),
        DispatchTarget::MotionMapImport => {
            crate::motion_bridge::motion_map_import(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::MotionApplyImport => {
            crate::motion_bridge::motion_apply_import(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::MotionLinkRefresh => {
            crate::motion_bridge::motion_link_refresh(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::MotionLinkRelink => {
            crate::motion_bridge::motion_link_relink(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::MotionLinkEdit => {
            crate::motion_bridge::motion_link_edit(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::MotionLinkTrackingInventory => {
            crate::motion_tracking::inventory(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::MotionLinkTrackingRequest => {
            crate::motion_tracking::request(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::MotionLinkTrackingInspect => {
            crate::motion_tracking::inspect(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::MotionLinkTrackingApply => {
            crate::motion_tracking::apply(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::MotionLinkTrackingVerify => {
            crate::motion_tracking::verify(state, args, actor)
                .await
                .into()
        }
        DispatchTarget::MotionLinkTrackingDetach => {
            crate::motion_tracking::detach(state, args, actor)
                .await
                .into()
        }

        // ------------------------------------------------------------------
        // recipe.* — declarative pipeline manifests: list/describe (pure
        // reads) + run (a job; the pure orchestrator over the existing verbs).
        // ------------------------------------------------------------------
        DispatchTarget::RecipeList => recipe_list(args).await.into(),
        DispatchTarget::RecipeDescribe => recipe_describe(args).await.into(),
        DispatchTarget::RecipeRun => recipe_run(state, args, actor).await.into(),

        DispatchTarget::AssembleBroll => assemble_broll(state, args, actor).await.into(),
        DispatchTarget::AssembleRepurpose => {
            speech_text::assemble_repurpose(state, args).await.into()
        }
        DispatchTarget::AssembleShorts => speech_text::assemble_shorts(state, args).await.into(),
        DispatchTarget::AssembleFromScript => {
            speech_text::assemble_from_script(state, args).await.into()
        }
        DispatchTarget::ScoreClip => speech_text::score_clip(state, args).await.into(),

        // ------------------------------------------------------------------
        // screen_record.* — screen recorder integration (sidecar)
        // ------------------------------------------------------------------
        DispatchTarget::ScreenRecordDoctor => crate::screen_record::screen_record_doctor(args)
            .await
            .into(),
        DispatchTarget::ScreenRecordSystemAudioProbe => {
            crate::screen_record::system_audio_capture::probe_handler(args)
                .await
                .into()
        }
        DispatchTarget::ScreenRecordStart => crate::screen_record::screen_record_start(state, args)
            .await
            .into(),
        DispatchTarget::ScreenRecordRecoveryStatus => {
            crate::screen_record::recovery_status_handler(state, args)
                .await
                .into()
        }
        DispatchTarget::ScreenRecordStop => screen_record_stop(state, args, actor).await.into(),
        DispatchTarget::ScreenRecordStudioEvent => {
            crate::screen_record_studio::screen_record_studio_event(state, args)
                .await
                .into()
        }
        DispatchTarget::ScreenRecordAutoedit => {
            crate::screen_record::screen_record_autoedit(state, args)
                .await
                .into()
        }
        DispatchTarget::ScreenRecordPolish => screen_record_polish(state, args, actor).await.into(),
        DispatchTarget::ScreenRecordExport => {
            crate::screen_record::screen_record_export(state, args)
                .await
                .into()
        }

        // ------------------------------------------------------------------
        // ui.* — relayed to the connected UI client over WS (ui_bridge.rs)
        // ------------------------------------------------------------------
        DispatchTarget::UiState => ui_state(state).await.into(),
        DispatchTarget::UiScreenshot => ui_screenshot(state, args).await.into(),
        DispatchTarget::DebugScreenshot => debug_screenshot(args).await.into(),
        DispatchTarget::UiOpen
        | DispatchTarget::UiPlayhead
        | DispatchTarget::UiSelect
        | DispatchTarget::UiHighlight => ui_forward(state, name, args).await.into(),

        // ------------------------------------------------------------------
        // system.* — environment doctor + consented tool fetch
        // ------------------------------------------------------------------
        DispatchTarget::SystemMcpTest => crate::mcp::self_test(state).await.into(),
        DispatchTarget::SystemDoctor => system_doctor(state, args).await.into(),
        DispatchTarget::SystemSetFfmpeg => crate::ffmpeg_settings::system_set_ffmpeg(state, args)
            .await
            .into(),
        DispatchTarget::SystemSetSttModel => crate::stt_settings::system_set_stt_model(state, args)
            .await
            .into(),
        DispatchTarget::SystemFetchTool => system_fetch_tool(state, args, actor).await.into(),
        DispatchTarget::SystemSetupPerception => {
            system_setup_perception(state, args, actor).await.into()
        }
        DispatchTarget::SystemSetupMatte => system_setup_matte(state, args, actor).await.into(),
    };
    if result.ok {
        if let Some(op_ids) = result.op_ids.clone() {
            let warnings = {
                let mut project = state.project.write().await;
                project
                    .as_mut()
                    .map(|store| store.take_commit_warnings(&op_ids))
                    .unwrap_or_default()
            };
            result = result.with_warnings(warnings);
        }
    }
    crate::request_control::finalize(
        state,
        name,
        &request_actor,
        &mut result,
        publish_request_receipt,
    )
    .await;
    result
}

// ---------------------------------------------------------------------------
// Op plumbing — the ONE commit path for every mutating verb
// ---------------------------------------------------------------------------

/// Commit ONE core `edit.*` verb through ProjectStore::apply — the single
/// core commit path: transactional mutation and replayable effects, rationale
/// per the rationale-preservation contract — then publish `op_applied`. A
/// previous server-local commit path wrote `edit.restore` self-pointers, which
/// made server-created operations un-restorable; all mutating verbs now flow
/// through core's commit fns
/// (apply / apply_lowered / record_import / checkpoint / revert) so the log
/// stays replayable and undoable by construction.
async fn commit_core(
    state: &AppState,
    verb: &str,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    commit_core_with_project(state, verb, args, actor, |_, _| {}).await
}

/// Commit a core verb after deriving private, replay-stable args from the
/// project while holding the same write lock as the commit.
async fn commit_core_with_project<F>(
    state: &AppState,
    verb: &str,
    mut args: Value,
    actor: Actor,
    resolve: F,
) -> Result<VerbResult, CutError>
where
    F: FnOnce(&cut_core::Project, &mut Value),
{
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    resolve(&store.project, &mut args);
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    // Legacy compatibility option; capture before `args` moves into store.apply.
    let include_legacy_inverse = wants_legacy_inverse(&args);
    let op = guard_call(verb, || store.apply(verb, args, actor, rationale))?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    Ok(VerbResult::ok_with_ops(
        shape_core_result(verb, &op, include_legacy_inverse),
        vec![op_id],
    ))
}

/// Read the deprecated `{include_inverse:true}` compatibility option.
///
/// v0.6.108 continues to validate this public REST/CLI/MCP argument so old
/// callers do not break, but fresh records use recompute-by-replay and have no
/// inverse payload to return. It can only affect serialization of an already
/// historic snapshot-era record supplied to `op_for_result`.
pub(crate) fn wants_legacy_inverse(args: &Value) -> bool {
    args.get("include_inverse")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Serialize an OpRecord for a verb result.
///
/// Current records are inverse-free. For an already historic snapshot-era
/// record, the deprecated compatibility option can retain its legacy payload;
/// otherwise it is removed and marked so a reader can distinguish a trimmed
/// historic record from a current inverse-free record.
pub(crate) fn op_for_result(op: &OpRecord, include_legacy_inverse: bool) -> Value {
    let mut v = serde_json::to_value(op).unwrap_or(Value::Null);
    if !include_legacy_inverse {
        if let Value::Object(map) = &mut v {
            // Only mark+drop historic payloads that were actually present.
            // Fresh records and non-timeline metadata records serialize without
            // the key, via skip_serializing_if, and remain unmarked.
            if map.remove("inverse").is_some() {
                map.insert("inverse_omitted".into(), Value::Bool(true));
            }
        }
    }
    v
}

/// Build the result shape schema/verbs.json documents for a core edit verb
/// from the committed op's recorded effects (the schema promises e.g.
/// `{clip_ids:[left,right]}` for edit.split — dispatch used to return only
/// `{op}`, a contract drift). The full op record still rides along as `op`
/// (the review rail and tests read it; additive to the documented shape). A
/// historic inverse payload is shown only when its deprecated compatibility
/// option was requested. Unknown verbs fall back to `{op}` alone.
fn shape_core_result(verb: &str, op: &OpRecord, include_legacy_inverse: bool) -> Value {
    // First effect's detail — every core edit verb records at least one
    // effect; indexing Value::Null yields Null (never panics).
    let d0 = op
        .effects
        .first()
        .map(|e| Value::Object(e.detail.clone()))
        .unwrap_or(Value::Null);
    let mut result = match verb {
        "edit.split" => json!({"clip_ids": [d0["left"], d0["right"]]}),
        "edit.ripple_delete" => {
            let removed_ms = op
                .args
                .get("range_ms")
                .and_then(|r| r.as_array())
                .and_then(|r| Some(r.get(1)?.as_u64()?.saturating_sub(r.first()?.as_u64()?)))
                .unwrap_or(0);
            let tracks: Vec<&str> = op
                .effects
                .iter()
                .filter_map(|e| e.track.as_deref())
                .collect();
            // ripple: the RESOLVED mode from the first effect (core records it);
            // default true (close-gap) for older ops.
            let ripple = d0.get("ripple").and_then(|v| v.as_bool()).unwrap_or(true);
            json!({"removed_ms": removed_ms, "tracks": tracks, "ripple": ripple})
        }
        "edit.trim" => json!({
            "clip": d0["clip"],
            "src_in_ms": d0["new_src_ms"][0],
            "src_out_ms": d0["new_src_ms"][1],
            "linked": op.effects.iter().any(|e| e.detail.contains_key("linked_trim")),
            "linked_clip": op.effects.iter().find_map(|e| {
                e.detail.get("linked_trim")?.get("clip").cloned()
            }).unwrap_or(Value::Null),
            "linked_track": op.effects.iter().find_map(|e| {
                e.detail.get("linked_trim")?.get("track").cloned()
            }).unwrap_or(Value::Null),
        }),
        // ripple: the resolved AV-sync mode + the sibling tracks that
        // received the gap (effects carrying rippled_gap_ms).
        "edit.move" => json!({
            "clip": op.args["clip"],
            "track": op.args["to_track"],
            "at_ms": op.args["at_ms"],
            "ripple": op.args.get("ripple").and_then(|v| v.as_bool())
                .or_else(|| d0.get("ripple").and_then(|v| v.as_bool()))
                .unwrap_or(false),
            "linked": op.args.get("linked").and_then(|v| v.as_bool()).unwrap_or(false),
            "linked_clip": op.effects.iter().find_map(|e| {
                e.detail.get("linked_move")?.get("clip").cloned()
            }).unwrap_or(Value::Null),
            "linked_track": op.effects.iter().find_map(|e| {
                e.detail.get("linked_move")?.get("track").cloned()
            }).unwrap_or(Value::Null),
            "rippled_tracks": op
                .effects
                .iter()
                .filter(|e| e.detail.contains_key("rippled_gap_ms"))
                .filter_map(|e| e.track.clone())
                .collect::<Vec<_>>(),
        }),
        // ripple = the RESOLVED sibling-shift mode (the ripple-sync contract); rippled_tracks
        // lists siblings that received the gap (effects after the first).
        "edit.insert" => json!({
            "clip_id": d0["added_clip"],
            "ripple": d0["ripple"],
            "rippled_tracks": op
                .effects
                .iter()
                .skip(1)
                .filter(|e| e.detail.contains_key("rippled_gap_ms"))
                .filter_map(|e| e.track.clone())
                .collect::<Vec<_>>(),
        }),
        // detail: {added_clip, source_clip, added_ms, ripple} on the first effect;
        // the effect's own `track` field carries the host track id. `rippled_tracks`
        // (ripple:true) lists siblings that received the gap (effects after the
        // first), exactly like edit.insert.
        "edit.duplicate" => json!({
            "clip_id": d0["added_clip"],
            "source_clip": d0["source_clip"],
            "track": op.effects.first().and_then(|e| e.track.clone()),
            "at_ms": d0["added_ms"][0],
            "ripple": d0["ripple"],
            "rippled_tracks": op
                .effects
                .iter()
                .skip(1)
                .filter(|e| e.detail.contains_key("rippled_gap_ms"))
                .filter_map(|e| e.track.clone())
                .collect::<Vec<_>>(),
        }),
        // detail: {added_clip, added_nest, nested_clips, added_ms, span_ms} on the
        // (single) effect; the effect's `track` carries the host track id. nest_id is
        // the new sub-timeline; clip_id is the single nest clip that replaced the run.
        "edit.nest" => json!({
            "clip_id": d0["added_clip"],
            "nest_id": d0["added_nest"],
            "track": op.effects.first().and_then(|e| e.track.clone()),
            "at_ms": d0["added_ms"][0],
            "span_ms": d0["span_ms"],
            "nested_clips": d0["nested_clips"],
        }),
        // detail: {clip, old_asset, asset, old_src_ms, new_src_ms, slot_ms, gap_ms}
        // on the (single) effect; the effect's `track` carries the host track id.
        // gap_ms > 0 = the source was shorter than the slot and the remainder was
        // padded with a gap (the clamp/hold behaviour).
        "edit.replace" => json!({
            "clip": d0["clip"],
            "track": op.effects.first().and_then(|e| e.track.clone()),
            "old_asset": d0["old_asset"],
            "asset": d0["asset"],
            "new_src_ms": d0["new_src_ms"],
            "slot_ms": d0["slot_ms"],
            "gap_ms": d0["gap_ms"],
        }),
        // detail: {added_clip, at_ms, added_ms, slot_ms, src_range_ms, speed,
        // source_span_ms} — the placed clip's id, its slot, and the chosen speed.
        "edit.fit_to_fill" => json!({
            "clip_id": d0["added_clip"],
            "track": op.effects.first().and_then(|e| e.track.clone()),
            "at_ms": d0["at_ms"],
            "slot_ms": d0["slot_ms"],
            "src_range_ms": d0["src_range_ms"],
            "speed": d0["speed"],
            "source_span_ms": d0["source_span_ms"],
        }),
        // detail: {target:"clip"|"track", id, old_db, new_db} — the schema's
        // `target` is the thing that changed (the id); `kind` says which.
        "edit.gain" => json!({
            "target": d0["id"], "kind": d0["target"],
            "old_db": d0["old_db"], "new_db": d0["new_db"],
        }),
        // detail: {old_muted, muted} / {old_solo, solo} — the non-destructive
        // mute/solo FLAG (edit::set_track_muted/solo). Gain is untouched; the renderer
        // resolves audibility from these flags at mix time. `track` is read from the
        // verb ARGS (NOT the detail — a `track` detail key would collide with
        // OpEffect.track under serde flatten, same as edit.reorder_track).
        "edit.mute" => json!({
            "track": op.args["track"], "old_muted": d0["old_muted"], "muted": d0["muted"],
        }),
        "edit.solo" => json!({
            "track": op.args["track"], "old_solo": d0["old_solo"], "solo": d0["solo"],
        }),
        // detail: {id, action, range_ms, mute_ranges, old_count} (edit::mute_range) —
        // the resulting NORMALIZED list rides along so callers see the merge outcome.
        "edit.mute_range" => json!({
            "clip": d0["id"], "action": d0["action"], "range_ms": d0["range_ms"],
            "mute_ranges": d0["mute_ranges"],
        }),
        // detail per touched clip: {clip, old_fade, new_fade} (edit::fade) —
        // the track form may touch two clips (first in, last out).
        "edit.fade" => json!({
            "targets": op
                .effects
                .iter()
                .filter(|e| e.detail.contains_key("clip"))
                .map(|e| json!({
                    "clip": e.detail.get("clip"),
                    "track": e.track,
                    "old_fade": e.detail.get("old_fade"),
                    "fade": e.detail.get("new_fade"),
                }))
                .collect::<Vec<_>>(),
        }),
        "edit.add_track" => json!({"track_id": d0["added_track"], "kind": d0["kind"]}),
        "edit.remove_track" => {
            json!({"track_id": d0["removed_track"], "kind": d0["kind"], "clips_dropped": d0["clips_dropped"]})
        }
        // detail: {from, to} + the effect's own track field (edit::reorder_track).
        // `track` is read from the verb args (NOT the detail — a `track` detail key
        // would collide with OpEffect.track under serde flatten).
        "edit.reorder_track" => {
            json!({"track": op.args["track"], "from": d0["from"], "to": d0["to"]})
        }
        // detail: {old_blend_mode, blend_mode}; OpEffect.track carries the target
        // track. Public contract echoes the applied mode, with null normalized to
        // "normal" because that is the user-facing clear/default value.
        "edit.blend" => json!({
            "track": op.args["track"],
            "blend_mode": d0.get("blend_mode").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!("normal")),
            "old_blend_mode": d0["old_blend_mode"],
        }),
        "edit.track_visible" => json!({
            "track": op.args["track"], "old_visible": d0["old_visible"], "visible": d0["visible"],
        }),
        "edit.track_lock" => json!({
            "track": op.args["track"], "old_locked": d0["old_locked"], "locked": d0["locked"],
        }),
        // detail: {clip, old_transform, new_transform} (edit::transform).
        "edit.transform" => json!({
            "clip": d0["clip"],
            "old_transform": d0["old_transform"],
            "transform": d0["new_transform"],
        }),
        // detail: {clip, old_crop, new_crop} (edit::crop). Schema (verbs.json):
        // {clip, old_crop, crop}.
        "edit.crop" => json!({
            "clip": d0["clip"],
            "old_crop": d0["old_crop"],
            "crop": d0["new_crop"],
        }),
        // detail: {clip, old_factor, factor, new_timeline_duration_ms}
        // (edit::speed). Schema (verbs.json) mirrors these keys.
        "edit.speed" => json!({
            "clip": d0["clip"],
            "old_factor": d0["old_factor"],
            "factor": d0["factor"],
            "new_timeline_duration_ms": d0["new_timeline_duration_ms"],
        }),
        // detail: SET {clip, cleared:false, points, segments, n_points,
        // new_timeline_duration_ms, old_ramp} | CLEAR {clip, cleared:true, had_ramp}
        // (edit::speed_ramp). The HONEST receipt reports the resolved curve, the
        // segment granularity (`method`), and the realized remapped duration.
        "edit.speed_ramp" => {
            if d0["cleared"] == json!(true) {
                json!({
                    "clip": d0["clip"],
                    "cleared": true,
                    "had_ramp": d0["had_ramp"],
                    "method": "piecewise-constant-speed segments (EDL-time)",
                })
            } else {
                json!({
                    "clip": d0["clip"],
                    "points": d0["points"],
                    "segments": d0["segments"],
                    "new_duration_ms": d0["new_timeline_duration_ms"],
                    "preserve_pitch": true,
                    "method": "piecewise-constant-speed segments (EDL-time)",
                })
            }
        }
        // detail: {at_ms, left_clip, right_clip, old_xfade_ms, xfade_ms,
        // fades_cleared} (edit::crossfade).
        "edit.crossfade" => json!({
            "at_ms": d0["at_ms"],
            "left_clip": d0["left_clip"],
            "right_clip": d0["right_clip"],
            "old_xfade_ms": d0["old_xfade_ms"],
            "xfade_ms": d0["xfade_ms"],
            "transition": d0["transition"],
            "fades_cleared": d0["fades_cleared"],
        }),
        // detail: {marker_id, old_at_ms, at_ms} (edit::marker_move).
        "edit.move_marker" => json!({
            "marker_id": d0["marker_id"],
            "old_at_ms": d0["old_at_ms"],
            "at_ms": d0["at_ms"],
        }),
        // detail: {clip, old_range_ms, range_ms} (edit::caption_set_range).
        "captions.set_range" => json!({
            "clip": d0["clip"],
            "old_range_ms": d0["old_range_ms"],
            "range_ms": d0["range_ms"],
        }),
        // detail: {clip, old_text, text, style_ref} (edit::caption_set_text).
        "captions.set_text" => json!({
            "clip": d0["clip"],
            "old_text": d0["old_text"],
            "text": d0["text"],
            "style_ref": d0["style_ref"],
        }),
        // detail: {ducked_windows, replaced_windows, windows} (edit::duck).
        "edit.duck" => {
            let total: u64 = d0["windows"]
                .as_array()
                .map(|ws| {
                    ws.iter()
                        .filter_map(|w| {
                            let r = w.get("range_ms")?.as_array()?;
                            Some(r.get(1)?.as_u64()?.saturating_sub(r.first()?.as_u64()?))
                        })
                        .sum()
                })
                .unwrap_or(0);
            json!({
                "track_id": op.effects.first().and_then(|e| e.track.clone()),
                "windows_applied": d0["ducked_windows"],
                "replaced_windows": d0["replaced_windows"],
                "total_ducked_ms": total,
                "windows": d0["windows"],
            })
        }
        "edit.add_marker" => json!({"marker_id": d0["added_marker"]["id"]}),
        "edit.remove_marker" => json!({"removed": d0["removed_marker"]["id"]}),
        // detail (tip): {restored_op, undid_verb}. detail (rebase mode):
        // {restored_op, mode, rebased_over, rebase_new_timeline}. The schema
        // result is {restored_op_id, op_ids[, mode, rebased_over]} — the rebase
        // fields ride only on a rebase op (None/absent for a tip restore).
        "edit.restore" => {
            let mut r = json!({"restored_op_id": d0["restored_op"], "op_ids": [op.op_id]});
            if d0["mode"] == json!("rebase") {
                r["mode"] = json!("rebase");
                r["rebased_over"] = d0["rebased_over"].clone();
            }
            r
        }
        _ => json!({}),
    };
    result["op"] = op_for_result(op, include_legacy_inverse);
    result
}

/// Commit one HIGHER-LAYER verb (transcript.*, captions.*) via its lowered
/// core-edit steps (ProjectStore::apply_lowered) and publish `op_applied`.
/// The op keeps the honest verb name; the recorded `lowered` effects entry is
/// what makes the op replayable (store::apply_record escape hatch).
async fn commit_lowered(
    state: &AppState,
    verb: &str,
    args: Value,
    actor: Actor,
    steps: Vec<InverseOp>,
    extra_effects: Vec<OpEffect>,
) -> Result<VerbResult, CutError> {
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let include_legacy_inverse = wants_legacy_inverse(&args);
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let op = guard_call(verb, || {
        store.apply_lowered(verb, args, actor, rationale, steps, extra_effects)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    let result = if matches!(verb, "edit.move" | "edit.trim") {
        shape_core_result(verb, &op, include_legacy_inverse)
    } else {
        json!({"op": op_for_result(&op, include_legacy_inverse)})
    };
    Ok(VerbResult::ok_with_ops(result, vec![op_id]))
}

/// One free-form effect record (track optional, detail keys op-specific).
pub(crate) fn effect(track: Option<&str>, detail: Value) -> OpEffect {
    let mut detail = match detail {
        Value::Object(m) => m,
        other => {
            let mut m = Map::new();
            m.insert("detail".into(), other);
            m
        }
    };
    detail.remove("track");
    OpEffect {
        track: track.map(String::from),
        detail,
    }
}

fn validate_path_component_arg(field: &str, value: &str) -> Result<(), CutError> {
    let invalid = value.is_empty()
        || value
            .chars()
            .any(|c| matches!(c, '/' | '\\' | '\0') || c.is_control());
    if invalid {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("{field} must be a single filename-safe value"),
            "do not include path separators or control characters",
        ));
    }
    Ok(())
}

/// Shared "no project open" error.
pub(crate) fn no_project() -> CutError {
    CutError::new(
        error_codes::NO_PROJECT,
        "no project is open",
        "call project.create or project.open first",
    )
}

/// Parse verb args into a typed struct, mapping serde errors to invalid_args
/// with the parse failure as the actionable cause.
pub(crate) fn parse_args<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, CutError> {
    serde_json::from_value(args).map_err(|e| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "verb args did not match the schema",
            e.to_string(),
        )
        .with_suggested_action("GET /api/verbs shows the args schema for every verb")
    })
}

mod safety;
pub(crate) use safety::{
    guard_call, owned_job_process_control, run_blocking, run_blocking_cancellable,
    run_bounded_foreground_command,
};
mod motion_link_projection;
mod project_workspace;
use project_workspace::{
    comment_add, comment_apply, comment_draft, comment_list, comment_resolve, library_add,
    library_add_to_project, library_favorite, library_folder_add, library_folder_remove,
    library_folder_rename, library_list, library_move, library_relink, library_remove, library_tag,
    library_use, project_brand, project_checkpoint, project_close, project_color, project_create,
    project_delete, project_diff, project_forget, project_format, project_health, project_list,
    project_open, project_ops, project_redo, project_rename, project_revert, project_save,
    project_sequence_create, project_sequence_delete, project_sequence_list,
    project_sequence_rename, project_sequence_switch, project_state, project_undo,
};
mod sequence_index;
use sequence_index::project_sequence_index;
mod brand;
mod generated_assets;
mod review_handoff;
use review_handoff::{comment_export, comment_import};
mod media;
pub(crate) use media::{
    asset_info, media_import, project_paths, spawn_plain_import_chain, update_asset,
    verify_attested_media_source, ANALYSIS_MAX_RUNNING,
};
use media::{
    media_bin_delete, media_bin_list, media_bin_save, media_check, media_filmstrip,
    media_perception, media_probe, media_relink, media_remove, media_transcribe, media_waveform,
};
const RENDER_MAX_RUNNING: usize = 1;
const RENDER_QUEUE_MAX_RUNNING: usize = 1;
// ---------------------------------------------------------------------------
// jobs.* handlers (the background-job contract)
// ---------------------------------------------------------------------------

mod jobs_handlers;
use jobs_handlers::{jobs_cancel, jobs_list, jobs_status};

mod edit_tools;
#[cfg(test)]
use edit_tools::resolve_slide;
use edit_tools::{
    agent_chat, assemble_broll, assets_fetch, assets_generate, assets_providers, assets_search,
    audio_add_music, audio_cleanup_voice, captions_kinetic, captions_set_range, captions_set_text,
    edit_add_marker, edit_add_mask, edit_add_shape, edit_add_track, edit_adjustment, edit_animate,
    edit_auto_balance, edit_auto_zoom, edit_blend, edit_color_match, edit_color_space, edit_crop,
    edit_crossfade, edit_cut_to_beat, edit_detach_audio, edit_duck, edit_duplicate, edit_effect,
    edit_eq, edit_fade, edit_fit_to_fill, edit_freeze, edit_gain, edit_grade, edit_grade_stack,
    edit_grade_window, edit_insert, edit_keyframe, edit_mark_scenes, edit_matte, edit_move,
    edit_move_marker, edit_multicam_switch, edit_multicam_sync, edit_mute, edit_mute_range,
    edit_nest, edit_pan, edit_paste, edit_redact, edit_remove_marker, edit_remove_track,
    edit_reorder_track, edit_replace, edit_restore, edit_reverse, edit_ripple_delete, edit_roll,
    edit_seek_marker, edit_slide, edit_slide_edit, edit_slip, edit_solo, edit_speed,
    edit_speed_ramp, edit_split, edit_split_at_scenes, edit_split_edit, edit_stabilize, edit_track,
    edit_track_lock, edit_track_visible, edit_transform, edit_trim, edit_trim_edges,
    edit_update_marker, effects_list, grade_apply, grade_list, grade_save, media_index,
    media_index_status, media_search, plugins_call, plugins_enable, plugins_list, shape_update,
    title_add, title_templates, title_update, transitions_list,
};
pub(crate) use edit_tools::{build_shape_spec, build_title_spec, ShapeArgs, TitleArgs};

// ---------------------------------------------------------------------------
// transcript.* handlers
// ---------------------------------------------------------------------------

/// Load the word transcript for an asset from its sidecar file (real I/O —
/// live as soon as a transcribe run has produced the file).
pub(crate) async fn load_transcript(
    state: &AppState,
    asset_id: &str,
) -> Result<cut_perception::Transcript, CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let asset = store.project.assets.get(asset_id).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("no asset '{asset_id}'"),
            "unknown asset id",
        )
    })?;
    let rel = asset.transcript.as_ref().ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("asset '{asset_id}' has no transcript yet"),
            "transcription has not completed for this asset",
        )
        .with_suggested_action("call media.transcribe{asset} and wait for the job to finish")
    })?;
    let path = store.dir.join(rel);
    let t: cut_perception::Transcript = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
    Ok(t)
}

mod speech_text;

// ---------------------------------------------------------------------------
// captions.* handlers — fully live (pure project mutation, no todo deps)
// ---------------------------------------------------------------------------

/// Caption line-breaking limits: pro-subtitle conventions (~42 chars/line,
/// new caption after a >1.2s speech gap).
mod captions;
use captions::{
    captions_generate, captions_import, captions_reflow, captions_shift, captions_translate,
};
pub(crate) use captions::{harvest_timeline_words, translation_warnings_to_verb};
#[cfg(test)]
use captions::{
    reflow_cues, replace_caption_texts_by_identity, CaptionTranslateSrcCue, ReflowOpts,
};

// ---------------------------------------------------------------------------
// render.* / verify.* / export.* handlers
// ---------------------------------------------------------------------------

mod rendering;
use rendering::{
    autopilot_run, clip_candidates, export_audio, export_frame, export_gif, export_publish,
    export_range, render_bundle, render_direct, render_final, render_frame, render_preview,
    render_qc, render_queue, render_reframe, render_storyboard, snapshot_for_media_io,
};
#[cfg(test)]
use rendering::{
    dims_from_aspect, jpeg_dimensions, mark_output_checks_unmeasured, next_receipt_id_preview,
    publish_render_final_args, reserve_receipt_id, resolve_reframe_output_for_qc,
    storyboard_tile_width, target_size_video_kbps, write_bundle_caption_sidecars,
    write_storyboard_tiles, PublishArgs, PREVIEW_DEFAULT_DURATION_MS,
};
pub(crate) use rendering::{dispatch_send, read_receipt, scrub_frame_bytes, snapshot};

mod recipe_handlers;
#[cfg(test)]
use recipe_handlers::run_resolved_recipe;
use recipe_handlers::{poll_sub_job, recipe_describe, recipe_list, recipe_run};

mod verify_handlers;
#[cfg(test)]
use verify_handlers::attach_judge_to_receipt;
use verify_handlers::{
    resolve_receipt_path, verify_brand, verify_captions, verify_checks, verify_delivery,
    verify_judge, verify_loudness, verify_pacing, verify_pregate, verify_rerun, verify_scopes,
};

pub(crate) fn configured_judge_adapter() -> Option<PathBuf> {
    verify_handlers::find_judge_adapter()
}

mod export_formats;
use export_formats::{
    export_ass, export_chapters, export_edl, export_error, export_otio, export_srt,
    export_transcript, export_vtt, export_xml,
};
#[cfg(test)]
use export_formats::{export_richness_warnings, ExportWarningTarget};

mod otio_import;
use otio_import::import_otio;

mod screen_record_handlers;
#[cfg(test)]
use screen_record_handlers::screen_record_polish_subverb_error;
use screen_record_handlers::{screen_record_polish, screen_record_stop};

mod ui_system;
use ui_system::{
    debug_screenshot, system_doctor, system_fetch_tool, system_setup_matte,
    system_setup_perception, ui_forward, ui_screenshot, ui_state,
};

#[cfg(test)]
// Test-only project locks are uncontended within each single-threaded runtime.
#[allow(clippy::await_holding_lock)]
mod tests;

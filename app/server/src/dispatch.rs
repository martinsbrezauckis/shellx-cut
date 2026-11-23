//! dispatch.rs — the single verb dispatcher under the zero-local-mutation contract.
//!
//! Role: EVERY surface funnels here — REST POST /api/verb/{name}, MCP
//! tools/call, and the `cutd verb` CLI all call `dispatch()`. No other code
//! path may mutate the project. Unknown verbs are rejected against the
//! registry; arg shapes are the verb's responsibility (handlers parse with
//! serde and return invalid_args with the parse error as cause).
//!
//! Dependency boundaries use `guard_call` and `run_blocking` to translate
//! synchronous or blocking-task panics into structured errors. Unsupported
//! operations remain explicit refusals; these guards do not establish runtime
//! dependency availability or native qualification.
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
    fence_output_path, fence_project_output_path, fenced_existing_file_under_dir,
    fenced_existing_project_export_read, make_fence, publish_output_atomic,
    resolve_existing_project_file, temp_output_path_for_render, write_output_atomic,
    OutputPathPolicy,
};
use crate::recipes;
use crate::registry::verb_contract::DispatchTarget;
use crate::state::AppState;
use crate::voiceover_timeline_coordinator as voiceover;
use cut_core::{
    error_codes, Actor, CutError, InverseOp, OpEffect, OpRecord, ProjectStore, VerbResult,
};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

pub(crate) mod adapter_location;
pub(crate) mod adapter_python;
pub(crate) use adapter_python::ENV_ADAPTER_PYTHON;
#[cfg(test)]
use adapter_python::{adapter_python_for_platform, find_python_on_path};

/// Dispatch one verb. `actor` records who/which surface for the op-log.
/// Returns the universal envelope; never panics on bad input. A refusal that
/// names a missing tool or condition gets its name and way forward from
/// schema/requirements.json on the way out.
pub async fn dispatch(state: &AppState, name: &str, args: Value, actor: Actor) -> VerbResult {
    let result = crate::requirements::complete_result(
        Box::pin(dispatch_unfilled(state, name, args, actor)).await,
    );
    if result
        .error
        .as_ref()
        .is_some_and(|error| state.doctor_cache.invalidate_on_missing(error))
    {
        state.doctor_cached().await;
    }
    result
}

async fn dispatch_unfilled(state: &AppState, name: &str, args: Value, actor: Actor) -> VerbResult {
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
    // Then media verbs refuse before any handler when ffmpeg is missing.
    let validated = state.registry.validate_args(spec, &args);
    if let Err(e) = crate::media_tool_gate::admit(state, name, &args, validated).await {
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
        return Box::pin(dispatch_validated(
            state,
            name,
            dispatch_target,
            prepared.args,
            prepared.actor,
            true,
        ))
        .await;
    }
    Box::pin(dispatch_validated(
        state,
        name,
        dispatch_target,
        prepared.args,
        prepared.actor,
        false,
    ))
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
    let mut result: VerbResult = route_verb(state, name, dispatch_target, args, actor).await;
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

type VerbFuture<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = VerbResult> + Send + 'a>>;

/// Box one handler's future: `verb!(handler(state, args))` awaits the handler
/// and converts its result into the envelope.
macro_rules! verb {
    ($handler:expr) => {
        Box::pin(async move { $handler.await.into() })
    };
}

/// Start the handler of one verb target. This is a plain fn and every arm
/// boxes its own future: a debug build gives a poll function a separate stack
/// slot for every match arm's temporaries, and with ~280 arms inline in
/// `dispatch_validated` its poll frame was ~900 KB (Windows debug test build).
/// Nested verbs (`screen_record.polish` -> `edit.insert`) stacked two such
/// frames and overflowed the 2 MiB test thread. Here the arm temporaries live
/// only while the future is built, and a poll runs just the chosen handler.
fn route_verb<'a>(
    state: &'a AppState,
    name: &'a str,
    dispatch_target: DispatchTarget,
    args: Value,
    actor: Actor,
) -> VerbFuture<'a> {
    // Targets come from schema/verbs.json; no catch-all arm on purpose: the compiler proves every
    // target has a handler, and registry loading verifies each verb names exactly one target.
    match dispatch_target {
        // ------------------------------------------------------------------
        // project.*
        // ------------------------------------------------------------------
        DispatchTarget::ProjectCreate => verb!(project_create(state, args, actor)),
        DispatchTarget::ProjectOpen => verb!(project_open(state, args)),
        DispatchTarget::ProjectList => verb!(project_list(args)),
        DispatchTarget::ProjectForget => verb!(project_forget(args)),
        DispatchTarget::ProjectDelete => verb!(project_delete(state, args)),
        DispatchTarget::ProjectSave => verb!(project_save(state)),
        DispatchTarget::ProjectPackagePlan => verb!(project_package_plan(state, args)),
        DispatchTarget::ProjectPackageCreate => verb!(project_package_create(state, args)),
        DispatchTarget::ProjectState => verb!(project_state(state, args)),
        DispatchTarget::ProjectHealth => verb!(project_health(state, args)),
        DispatchTarget::ProjectCachePreview => verb!(project_cache_preview(state)),
        DispatchTarget::ProjectCacheRebuild => verb!(project_cache_rebuild(state, args)),
        DispatchTarget::ProjectCachePurge => verb!(project_cache_purge(state, args)),
        DispatchTarget::ProjectSequenceList => verb!(project_sequence_list(state)),
        DispatchTarget::ProjectSequenceIndex => verb!(project_sequence_index(state, args)),
        DispatchTarget::ProjectSequenceCreate => verb!(project_sequence_create(state, args, actor)),
        DispatchTarget::ProjectSequenceSwitch => verb!(project_sequence_switch(state, args, actor)),
        DispatchTarget::ProjectSequenceRename => verb!(project_sequence_rename(state, args, actor)),
        DispatchTarget::ProjectSequenceDelete => verb!(project_sequence_delete(state, args, actor)),
        DispatchTarget::ProjectOps => verb!(project_ops(state, args)),
        DispatchTarget::ProjectGroupPreview => verb!(project_group_preview(state, args)),
        DispatchTarget::ProjectGroupReject => verb!(project_group_reject(state, args, actor)),
        DispatchTarget::ProjectClose => verb!(project_close(state, args)),
        DispatchTarget::ProjectCheckpoint => verb!(project_checkpoint(state, args, actor)),
        DispatchTarget::ProjectRename => verb!(project_rename(state, args, actor)),
        DispatchTarget::ProjectFormat => verb!(project_format(state, args, actor)),
        DispatchTarget::ProjectColor => verb!(project_color(state, args, actor)),
        DispatchTarget::ProjectBrand => verb!(project_brand(state, args, actor)),
        DispatchTarget::ProjectSetOutputDir => {
            verb!(crate::output_paths::project_set_output_dir(state, args))
        }
        DispatchTarget::ProjectGetOutputDir => {
            verb!(crate::output_paths::project_get_output_dir())
        }
        DispatchTarget::CommentAdd => verb!(comment_add(state, args, actor)),
        DispatchTarget::CommentExport => verb!(comment_export(state, args)),
        DispatchTarget::CommentImport => verb!(comment_import(state, args, actor)),
        DispatchTarget::CommentList => verb!(comment_list(state, args)),
        DispatchTarget::CommentResolve => verb!(comment_resolve(state, args, actor)),
        DispatchTarget::CommentDraft => verb!(comment_draft(state, args, actor)),
        DispatchTarget::CommentApply => verb!(comment_apply(state, args, actor)),
        DispatchTarget::ProjectRevert => verb!(project_revert(state, args, actor)),
        DispatchTarget::ProjectUndo => verb!(project_undo(state, actor)),
        DispatchTarget::ProjectRedo => verb!(project_redo(state, actor)),
        DispatchTarget::ProjectDiff => verb!(project_diff(state, args)),
        DispatchTarget::LibraryList => verb!(library_list(args)),
        DispatchTarget::LibraryAdd => verb!(library_add(state, args)),
        DispatchTarget::LibraryRelink => verb!(library_relink(args)),
        DispatchTarget::LibraryRemove => verb!(library_remove(args)),
        DispatchTarget::LibraryMove => verb!(library_move(args)),
        DispatchTarget::LibraryTag => verb!(library_tag(args)),
        DispatchTarget::LibraryFavorite => verb!(library_favorite(args)),
        DispatchTarget::LibraryUse => verb!(library_use(args)),
        DispatchTarget::LibraryAddToProject => verb!(library_add_to_project(state, args, actor)),
        DispatchTarget::LibraryFolderAdd => verb!(library_folder_add(args)),
        DispatchTarget::LibraryFolderRename => verb!(library_folder_rename(args)),
        DispatchTarget::LibraryFolderRemove => verb!(library_folder_remove(args)),

        // ------------------------------------------------------------------
        // media.* — import kicks the probe→proxy→transcribe→perception chain
        // ------------------------------------------------------------------
        DispatchTarget::MediaImport => verb!(media_import(state, args, actor)),
        DispatchTarget::MediaRemove => verb!(media_remove(state, args, actor)),
        DispatchTarget::MediaRelink => verb!(media_relink(state, args, actor)),
        DispatchTarget::MediaRelinkPreview => verb!(media_relink_preview(state, args)),
        DispatchTarget::MediaRelinkApply => verb!(media_relink_apply(state, args, actor)),
        DispatchTarget::MediaCheck => verb!(media_check(state, args)),
        DispatchTarget::MediaBinSave => verb!(media_bin_save(state, args, actor)),
        DispatchTarget::MediaBinDelete => verb!(media_bin_delete(state, args, actor)),
        DispatchTarget::MediaBinList => verb!(media_bin_list(state, args)),
        DispatchTarget::MediaProbe => verb!(media_probe(state, args)),
        DispatchTarget::MediaWaveform => verb!(media_waveform(state, args)),
        DispatchTarget::MediaFilmstrip => verb!(media_filmstrip(state, args)),
        DispatchTarget::MediaTranscribe => verb!(media_transcribe(state, args)),
        DispatchTarget::MediaPerception => verb!(media_perception(state, args)),
        DispatchTarget::MediaDiarize => verb!(crate::diarize::media_diarize(state, args)),

        // ------------------------------------------------------------------
        // jobs.* (the background-job contract: jobs domain replaces media.status)
        // ------------------------------------------------------------------
        DispatchTarget::JobsStatus => verb!(jobs_status(state, args)),
        DispatchTarget::JobsList => verb!(jobs_list(state)),
        DispatchTarget::JobsCancel => verb!(jobs_cancel(state, args)),
        DispatchTarget::JobsRetry => verb!(jobs_retry(state, args)),
        DispatchTarget::JobsBeforeQuit => verb!(jobs_before_quit(state, args)),
        DispatchTarget::JobsInterrupted => verb!(jobs_interrupted(state, args)),
        DispatchTarget::JobsInterruptedDismiss => verb!(jobs_interrupted_dismiss(args)),

        // ------------------------------------------------------------------
        // edit.* — thin arg-parse wrappers over cut_core::edit, one op each
        // ------------------------------------------------------------------
        DispatchTarget::EditSplit => verb!(edit_split(state, args, actor)),
        DispatchTarget::EditCutToBeat => verb!(edit_cut_to_beat(state, args, actor)),
        DispatchTarget::EditSplitAtScenes => verb!(edit_split_at_scenes(state, args, actor)),
        DispatchTarget::EditMarkScenes => verb!(edit_mark_scenes(state, args, actor)),
        DispatchTarget::EditTrimEdges => verb!(edit_trim_edges(state, args, actor)),
        DispatchTarget::EditRippleDelete => verb!(edit_ripple_delete(state, args, actor)),
        DispatchTarget::EditTrim => verb!(edit_trim(state, args, actor)),
        DispatchTarget::EditSpeed => verb!(edit_speed(state, args, actor)),
        DispatchTarget::EditSpeedRamp => verb!(edit_speed_ramp(state, args, actor)),
        DispatchTarget::EditMove => verb!(edit_move(state, args, actor)),
        DispatchTarget::EditInsert => verb!(edit_insert(state, args, actor)),
        DispatchTarget::EditInsertLinked => verb!(edit_insert_linked(state, args, actor)),
        DispatchTarget::EditOverwrite => verb!(edit_overwrite(state, args, actor)),
        DispatchTarget::EditDuplicate => verb!(edit_duplicate(state, args, actor)),
        DispatchTarget::EditNest => verb!(edit_nest(state, args, actor)),
        DispatchTarget::EditReplace => verb!(edit_replace(state, args, actor)),
        DispatchTarget::EditFitToFill => verb!(edit_fit_to_fill(state, args, actor)),
        DispatchTarget::EditDetachAudio => verb!(edit_detach_audio(state, args, actor)),
        DispatchTarget::EditSplitEdit => verb!(edit_split_edit(state, args, actor)),
        DispatchTarget::EditPaste => verb!(edit_paste(state, args, actor)),
        DispatchTarget::EditGain => verb!(edit_gain(state, args, actor)),
        DispatchTarget::EditFade => verb!(edit_fade(state, args, actor)),
        DispatchTarget::EditMuteRange => verb!(edit_mute_range(state, args, actor)),
        DispatchTarget::EditTransform => verb!(edit_transform(state, args, actor)),
        DispatchTarget::EditCrop => verb!(edit_crop(state, args, actor)),
        DispatchTarget::EditGrade => verb!(edit_grade(state, args, actor)),
        DispatchTarget::EditGradeStack => verb!(edit_grade_stack(state, args, actor)),
        DispatchTarget::EditGradeWindow => verb!(edit_grade_window(state, args, actor)),
        DispatchTarget::GradeSave => verb!(grade_save(state, args, actor)),
        DispatchTarget::GradeApply => verb!(grade_apply(state, args, actor)),
        DispatchTarget::GradeList => verb!(grade_list(state)),
        DispatchTarget::EditColorSpace => verb!(edit_color_space(state, args, actor)),
        DispatchTarget::EditColorMatch => verb!(edit_color_match(state, args, actor)),
        DispatchTarget::EditAutoBalance => verb!(edit_auto_balance(state, args, actor)),
        DispatchTarget::EditMatte => verb!(edit_matte(state, args, actor)),
        DispatchTarget::EditEffect => verb!(edit_effect(state, args, actor)),
        DispatchTarget::EditAdjustment => verb!(edit_adjustment(state, args, actor)),
        DispatchTarget::EditReverse => verb!(edit_reverse(state, args, actor)),
        DispatchTarget::EditStabilize => verb!(edit_stabilize(state, args, actor)),
        DispatchTarget::EditFreeze => verb!(edit_freeze(state, args, actor)),
        DispatchTarget::EditAnimate => verb!(edit_animate(state, args, actor)),
        DispatchTarget::EditKeyframe => verb!(edit_keyframe(state, args, actor)),
        DispatchTarget::EditAutoZoom => verb!(edit_auto_zoom(state, args, actor)),
        DispatchTarget::EditAddMask => verb!(edit_add_mask(state, args, actor)),
        DispatchTarget::EditRedact => verb!(edit_redact(state, args, actor)),
        DispatchTarget::EditTrack => verb!(edit_track(state, args)),
        DispatchTarget::EditMulticamSync => verb!(edit_multicam_sync(state, args)),
        DispatchTarget::EditMulticamSwitch => verb!(edit_multicam_switch(state, args, actor)),
        DispatchTarget::EditEq => verb!(edit_eq(state, args, actor)),
        DispatchTarget::AudioCleanupVoice => verb!(audio_cleanup_voice(state, args, actor)),
        DispatchTarget::EditSlide => verb!(edit_slide(state, args, actor)),
        DispatchTarget::TitleAdd => verb!(title_add(state, args, actor)),
        DispatchTarget::TitleUpdate => verb!(title_update(state, args, actor)),
        DispatchTarget::TitleTemplates => verb!(title_templates(state, args, actor)),
        DispatchTarget::EditAddShape => verb!(edit_add_shape(state, args, actor)),
        DispatchTarget::ShapeUpdate => verb!(shape_update(state, args, actor)),
        DispatchTarget::EditSeekMarker => verb!(edit_seek_marker(state, args, actor)),
        DispatchTarget::AssetsProviders => verb!(assets_providers(state, args, actor)),
        DispatchTarget::AssetsSearch => verb!(assets_search(state, args, actor)),
        DispatchTarget::AssetsFetch => verb!(assets_fetch(state, args, actor)),
        DispatchTarget::AssetsGenerate => verb!(assets_generate(state, args, actor)),
        DispatchTarget::AssetsGeneratedList => {
            verb!(generated_assets::assets_generated_list(state, args))
        }
        DispatchTarget::AgentChat => verb!(agent_chat(state, args, actor)),
        DispatchTarget::PluginsList => verb!(plugins_list(state, args, actor)),
        DispatchTarget::PluginsEnable => verb!(plugins_enable(state, args, actor)),
        DispatchTarget::PluginsCall => verb!(plugins_call(state, args, actor)),
        DispatchTarget::MediaIndexStatus => verb!(media_index_status(state, args)),
        DispatchTarget::MediaSearch => verb!(media_search(state, args, actor)),
        DispatchTarget::MediaIndex => verb!(media_index(state, args, actor)),
        DispatchTarget::MediaIntelligenceStatus => verb!(media_intelligence_status(state, args)),
        DispatchTarget::MediaIntelligenceRebuild => verb!(media_intelligence_rebuild(state, args)),
        DispatchTarget::MediaIntelligenceSearch => verb!(media_intelligence_search(state, args)),
        DispatchTarget::InspectMedia => verb!(inspect_media(state, args)),
        DispatchTarget::InspectRange => verb!(inspect_range(state, args)),
        DispatchTarget::EffectsList => verb!(effects_list(state, args, actor)),
        DispatchTarget::TransitionsList => verb!(transitions_list(state, args, actor)),
        DispatchTarget::CaptionsKinetic => verb!(captions_kinetic(state, args, actor)),
        DispatchTarget::EditCrossfade => verb!(edit_crossfade(state, args, actor)),
        DispatchTarget::EditDuck => verb!(edit_duck(state, args, actor)),
        DispatchTarget::EditAddTrack => verb!(edit_add_track(state, args, actor)),
        DispatchTarget::EditRemoveTrack => verb!(edit_remove_track(state, args, actor)),
        DispatchTarget::EditReorderTrack => verb!(edit_reorder_track(state, args, actor)),
        DispatchTarget::EditBlend => verb!(edit_blend(state, args, actor)),
        DispatchTarget::EditTrackVisible => verb!(edit_track_visible(state, args, actor)),
        DispatchTarget::EditTrackLock => verb!(edit_track_lock(state, args, actor)),
        DispatchTarget::EditMute => verb!(edit_mute(state, args, actor)),
        DispatchTarget::EditSolo => verb!(edit_solo(state, args, actor)),
        DispatchTarget::EditPan => verb!(edit_pan(state, args, actor)),
        DispatchTarget::EditSlip => verb!(edit_slip(state, args, actor)),
        DispatchTarget::EditRoll => verb!(edit_roll(state, args, actor)),
        DispatchTarget::EditSlideEdit => verb!(edit_slide_edit(state, args, actor)),
        DispatchTarget::EditPasteAttributes => verb!(
            crate::paste_attributes::edit_paste_attributes(state, args, actor)
        ),
        DispatchTarget::EditAddMarker => verb!(edit_add_marker(state, args, actor)),
        DispatchTarget::EditRemoveMarker => verb!(edit_remove_marker(state, args, actor)),
        DispatchTarget::EditMoveMarker => verb!(edit_move_marker(state, args, actor)),
        DispatchTarget::EditUpdateMarker => verb!(edit_update_marker(state, args, actor)),
        DispatchTarget::EditRestore => verb!(edit_restore(state, args, actor)),

        // ------------------------------------------------------------------
        // audio.* — music bed with auto-duck + beat markers
        // ------------------------------------------------------------------
        DispatchTarget::AudioAddMusic => verb!(audio_add_music(state, args, actor)),
        DispatchTarget::AudioDub => verb!(crate::dub::audio_dub(state, args, actor)),

        // ------------------------------------------------------------------
        // transcript.* — transcript-driven edits (one op per removed span)
        // ------------------------------------------------------------------
        DispatchTarget::TranscriptGet => verb!(speech_text::transcript_get(state, args)),
        DispatchTarget::TranscriptTimeline => verb!(speech_text::transcript_timeline(state, args)),
        DispatchTarget::TranscriptCutWords => {
            verb!(speech_text::transcript_cut_words(state, args, actor))
        }
        DispatchTarget::TranscriptIgnoreWords => {
            verb!(speech_text::transcript_ignore_words(state, args, actor))
        }
        DispatchTarget::TranscriptMuteWords => {
            verb!(speech_text::transcript_mute_words(state, args, actor))
        }
        DispatchTarget::TranscriptAssemble => {
            verb!(speech_text::transcript_assemble(state, args, actor))
        }
        DispatchTarget::TranscriptSearch => verb!(speech_text::transcript_search(state, args)),
        DispatchTarget::TranscriptChapters => verb!(speech_text::transcript_chapters(state, args)),
        DispatchTarget::TranscriptRemoveSilences => {
            verb!(speech_text::transcript_remove_silences(state, args, actor))
        }
        DispatchTarget::TranscriptRemoveFillers => {
            verb!(speech_text::transcript_remove_fillers(state, args, actor))
        }
        DispatchTarget::TranscriptRemoveRetakes => {
            verb!(speech_text::transcript_remove_retakes(state, args, actor))
        }
        DispatchTarget::TranscriptTranslate => {
            verb!(speech_text::transcript_translate(state, args))
        }

        // ------------------------------------------------------------------
        // captions.*
        // ------------------------------------------------------------------
        DispatchTarget::CaptionsGenerate => verb!(captions_generate(state, args, actor)),
        DispatchTarget::CaptionsTranslate => verb!(captions_translate(state, args, actor)),
        DispatchTarget::CaptionsImport => verb!(captions_import(state, args, actor)),
        DispatchTarget::CaptionsAddText => {
            verb!(speech_text::captions_add_text(state, args, actor))
        }
        DispatchTarget::CaptionsSetStyle => {
            verb!(speech_text::captions_set_style(state, args, actor))
        }
        DispatchTarget::CaptionsSaveStyle => {
            verb!(speech_text::captions_save_style(state, args, actor))
        }
        DispatchTarget::CaptionsApplyStyle => {
            verb!(speech_text::captions_apply_style(state, args, actor))
        }
        DispatchTarget::CaptionsListStyles => verb!(speech_text::captions_list_styles(state, args)),
        DispatchTarget::CaptionsReflow => verb!(captions_reflow(state, args, actor)),
        DispatchTarget::CaptionsShift => verb!(captions_shift(state, args, actor)),
        DispatchTarget::CaptionsSetRange => verb!(captions_set_range(state, args, actor)),
        DispatchTarget::CaptionsSetText => verb!(captions_set_text(state, args, actor)),
        DispatchTarget::CaptionsBulkPreview => verb!(captions_bulk_preview(state, args)),
        DispatchTarget::CaptionsBulkApply => verb!(captions_bulk_apply(state, args, actor)),

        // ------------------------------------------------------------------
        // render.* / verify.* / export.*
        // ------------------------------------------------------------------
        DispatchTarget::RenderPreview => verb!(render_preview(state, args)),
        DispatchTarget::RenderFrame => verb!(render_frame(state, args)),
        DispatchTarget::RenderCompare => verb!(render_compare(state, args)),
        DispatchTarget::RenderStoryboard => verb!(render_storyboard(state, args)),
        DispatchTarget::RenderFinal => verb!(render_final(state, args, actor)),
        DispatchTarget::RenderReframe => verb!(render_reframe(state, args, actor)),
        DispatchTarget::RenderDirect => verb!(render_direct(state, args, actor)),
        DispatchTarget::RenderQc => verb!(render_qc(state, args, actor)),
        DispatchTarget::VerifyChecks => verb!(verify_checks(state, args)),
        DispatchTarget::VerifyRerun => verb!(verify_rerun(state, args)),
        DispatchTarget::VerifyPacing => verb!(verify_pacing(state)),
        DispatchTarget::VerifyCaptions => verb!(verify_captions(state, args)),
        DispatchTarget::VerifyDelivery => verb!(verify_delivery(state, args)),
        DispatchTarget::VerifyLoudness => verb!(verify_loudness(state, args)),
        DispatchTarget::VerifyScopes => verb!(verify_scopes(state, args)),
        DispatchTarget::VerifyBrand => verb!(verify_brand(state, args)),
        DispatchTarget::VerifyJudge => verb!(verify_judge(state, args)),
        DispatchTarget::VerifyPregate => verb!(verify_pregate(state, args)),
        DispatchTarget::ExportFrame => verb!(export_frame(state, args, actor)),
        DispatchTarget::ExportRange => verb!(export_range(state, args, actor)),
        DispatchTarget::ExportAudio => verb!(export_audio(state, args, actor)),
        DispatchTarget::ExportPublish => verb!(export_publish(state, args, actor)),
        DispatchTarget::ExportGif => verb!(export_gif(state, args, actor)),
        DispatchTarget::ExportXml => verb!(export_xml(state, args)),
        DispatchTarget::ExportOtio => verb!(export_otio(state, args)),
        DispatchTarget::ExportEdl => verb!(export_edl(state, args)),
        DispatchTarget::ImportOtio => verb!(import_otio(state, args, actor)),
        DispatchTarget::ExportSrt => verb!(export_srt(state, args)),
        DispatchTarget::ExportVtt => verb!(export_vtt(state, args)),
        DispatchTarget::ExportAss => verb!(export_ass(state, args)),
        DispatchTarget::ExportTranscript => verb!(export_transcript(state, args)),
        DispatchTarget::ExportChapters => verb!(export_chapters(state, args)),

        // ------------------------------------------------------------------
        // clip.* — social repurposing: rank shareable windows + bundle
        // ------------------------------------------------------------------
        DispatchTarget::ClipCandidates => verb!(clip_candidates(state, args)),
        DispatchTarget::RenderBundle => verb!(render_bundle(state, args, actor)),
        DispatchTarget::RenderQueue => verb!(render_queue(state, args, actor)),
        DispatchTarget::AutopilotRun => verb!(autopilot_run(state, args, actor)),

        // ------------------------------------------------------------------
        // generate.* — native Generate module foundation. Pure catalog reads,
        // non-mutating previews, native inserts, prompt planning, and storyboard
        // planning all live in one domain.
        // ------------------------------------------------------------------
        DispatchTarget::GenerateList => verb!(generate_handlers::generate_list(args)),
        DispatchTarget::GenerateDescribe => verb!(generate_handlers::generate_describe(args)),
        DispatchTarget::GeneratePreview => verb!(generate_handlers::generate_preview(state, args)),
        DispatchTarget::GenerateInsert => {
            verb!(generate_handlers::generate_insert(state, args, actor))
        }
        DispatchTarget::GenerateFromPrompt => {
            verb!(generate_handlers::generate_from_prompt(state, args, actor))
        }
        DispatchTarget::GenerateStoryboard => {
            verb!(generate_handlers::generate_storyboard(state, args, actor))
        }
        DispatchTarget::MotionTemplateToCut => verb!(crate::motion_bridge::motion_template_to_cut(
            state, args, actor
        )),
        DispatchTarget::MotionScriptToCut => verb!(crate::motion_bridge::motion_script_to_cut(
            state, args, actor
        )),
        DispatchTarget::MotionJobGet => verb!(crate::motion_jobs::get(state, args)),
        DispatchTarget::MotionJobList => verb!(crate::motion_jobs::list(state, args)),
        DispatchTarget::MotionMapImport => {
            verb!(crate::motion_bridge::motion_map_import(state, args, actor))
        }
        DispatchTarget::MotionApplyImport => verb!(crate::motion_bridge::motion_apply_import(
            state, args, actor
        )),
        DispatchTarget::MotionLinkRefresh => verb!(crate::motion_bridge::motion_link_refresh(
            state, args, actor
        )),
        DispatchTarget::MotionLinkRelink => {
            verb!(crate::motion_bridge::motion_link_relink(state, args, actor))
        }
        DispatchTarget::MotionLinkEdit => {
            verb!(crate::motion_bridge::motion_link_edit(state, args, actor))
        }
        DispatchTarget::MotionLinkTrackingInventory => {
            verb!(crate::motion_tracking::inventory(state, args, actor))
        }
        DispatchTarget::MotionLinkTrackingRequest => {
            verb!(crate::motion_tracking::request(state, args, actor))
        }
        DispatchTarget::MotionLinkTrackingInspect => {
            verb!(crate::motion_tracking::inspect(state, args, actor))
        }
        DispatchTarget::MotionLinkTrackingApply => {
            verb!(crate::motion_tracking::apply(state, args, actor))
        }
        DispatchTarget::MotionLinkTrackingVerify => {
            verb!(crate::motion_tracking::verify(state, args, actor))
        }
        DispatchTarget::MotionLinkTrackingDetach => {
            verb!(crate::motion_tracking::detach(state, args, actor))
        }

        // ------------------------------------------------------------------
        // recipe.* — declarative pipeline manifests: list/describe (pure
        // reads) + run (a job; the pure orchestrator over the existing verbs).
        // ------------------------------------------------------------------
        DispatchTarget::RecipeList => verb!(recipe_list(args)),
        DispatchTarget::RecipeDescribe => verb!(recipe_describe(args)),
        DispatchTarget::RecipeRun => verb!(recipe_run(state, args, actor)),

        DispatchTarget::AssembleBroll => verb!(assemble_broll(state, args, actor)),
        DispatchTarget::AssembleRepurpose => {
            verb!(speech_text::assemble_repurpose(state, args, actor))
        }
        DispatchTarget::AssembleShorts => verb!(speech_text::assemble_shorts(state, args, actor)),
        DispatchTarget::AssembleFromScript => {
            verb!(speech_text::assemble_from_script(state, args, actor))
        }
        DispatchTarget::ScoreClip => verb!(speech_text::score_clip(state, args)),

        // screen_record.* keeps the exhaustive domain list here; bounded adapters own routing.
        target @ (DispatchTarget::ScreenRecordDoctor
        | DispatchTarget::ScreenRecordMicrophoneSelection
        | DispatchTarget::ScreenRecordSystemAudioProbe
        | DispatchTarget::ScreenRecordRehearsalStart
        | DispatchTarget::ScreenRecordRehearsalDiscard
        | DispatchTarget::ScreenRecordStart
        | DispatchTarget::ScreenRecordSceneActivate
        | DispatchTarget::ScreenRecordSceneTimer
        | DispatchTarget::ScreenRecordPause
        | DispatchTarget::ScreenRecordResume
        | DispatchTarget::ScreenRecordRecoveryStatus
        | DispatchTarget::ScreenRecordActive
        | DispatchTarget::ScreenRecordPending
        | DispatchTarget::ScreenRecordPendingDismiss
        | DispatchTarget::ScreenRecordRecover
        | DispatchTarget::ScreenRecordStatus
        | DispatchTarget::ScreenRecordLiveFrame
        | DispatchTarget::ScreenRecordPreviewCapability
        | DispatchTarget::ScreenRecordPreviewStart
        | DispatchTarget::ScreenRecordPreviewStatus
        | DispatchTarget::ScreenRecordPreviewFrame
        | DispatchTarget::ScreenRecordPreviewPause
        | DispatchTarget::ScreenRecordPreviewResume
        | DispatchTarget::ScreenRecordPreviewHide
        | DispatchTarget::ScreenRecordPreviewStop
        | DispatchTarget::ScreenRecordStop
        | DispatchTarget::ScreenRecordStudioEvent
        | DispatchTarget::ScreenRecordAutoedit
        | DispatchTarget::ScreenRecordPolish
        | DispatchTarget::ScreenRecordCameraReset
        | DispatchTarget::ScreenRecordTakeLink
        | DispatchTarget::ScreenRecordZoomList
        | DispatchTarget::ScreenRecordZoomSet
        | DispatchTarget::ScreenRecordZoomAdd
        | DispatchTarget::ScreenRecordZoomUpdate
        | DispatchTarget::ScreenRecordZoomRemove
        | DispatchTarget::ScreenRecordZoomAuto
        | DispatchTarget::ScreenRecordTakeRefresh
        | DispatchTarget::ScreenRecordExport
        | DispatchTarget::ScreenRecordCopyRaw) => {
            Box::pin(
                async move { screen_record_routes::dispatch(target, state, args, actor).await },
            )
        }
        DispatchTarget::VoiceoverStart => verb!(voiceover::start(state, args, actor)),
        DispatchTarget::VoiceoverTick => verb!(voiceover::tick(state, args, actor)),
        DispatchTarget::VoiceoverStop => verb!(voiceover::stop(state, args, actor)),
        DispatchTarget::VoiceoverCancel => verb!(voiceover::cancel(state, args, actor)),
        DispatchTarget::VoiceoverObservePlayhead => {
            verb!(voiceover::observe_playhead(state, args, actor))
        }

        // ------------------------------------------------------------------
        // ui.* — relayed to the connected UI client over WS (ui_bridge.rs)
        // ------------------------------------------------------------------
        DispatchTarget::UiState => verb!(ui_state(state)),
        DispatchTarget::UiScreenshot => verb!(ui_screenshot(state, args)),
        DispatchTarget::DebugScreenshot => verb!(debug_screenshot(state, args)),
        DispatchTarget::UiOpen
        | DispatchTarget::UiPlayhead
        | DispatchTarget::UiSelect
        | DispatchTarget::UiPreview
        | DispatchTarget::UiHighlight => verb!(ui_forward(state, name, args)),

        // ------------------------------------------------------------------
        // system.* — environment doctor + consented tool fetch
        // ------------------------------------------------------------------
        DispatchTarget::SystemMcpTest => verb!(crate::mcp::self_test(state)),
        DispatchTarget::SystemDoctor => verb!(system_doctor(state, args)),
        DispatchTarget::SystemMotionStatus => verb!(system_motion_status(args)),
        DispatchTarget::SystemSetFfmpeg => {
            verb!(crate::ffmpeg_settings::system_set_ffmpeg(state, args))
        }
        DispatchTarget::SystemSetSttModel => {
            verb!(crate::stt_settings::system_set_stt_model(state, args))
        }
        DispatchTarget::SystemFetchTool => verb!(system_fetch_tool(state, args, actor)),
        DispatchTarget::SystemSetupPerception => verb!(system_setup_perception(state, args, actor)),
        DispatchTarget::SystemSetupMatte => verb!(system_setup_matte(state, args, actor)),
    }
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
            // Item 176: how the clip landed, and any clip the overwrite
            // replaced completely (older logged moves are "insert").
            "placement": op.args.get("placement").cloned().unwrap_or_else(|| json!("insert")),
            "removed_clips": op.effects.iter()
                .filter_map(|e| e.detail.get("removed_clip_ids")?.as_array())
                .flatten().cloned().collect::<Vec<_>>(),
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
        "edit.overwrite" => json!({
            "at_ms": op.args["at_ms"],
            "src_range_ms": op.args["src_range_ms"],
            "duration_ms": op.args["src_range_ms"][1].as_u64()
                .zip(op.args["src_range_ms"][0].as_u64())
                .map(|(end, start)| end.saturating_sub(start))
                .unwrap_or(0),
            "linked_av": op.args.get("video_track").and_then(Value::as_str).is_some()
                && op.args.get("audio_track").and_then(Value::as_str).is_some(),
            "tracks": op.effects.iter().filter_map(|effect| {
                if !effect.detail.contains_key("added_clip") { return None; }
                let track = effect.track.clone()?;
                Some(json!({
                    "track": track,
                    "clip_id": effect.detail.get("added_clip").cloned().unwrap_or(Value::Null),
                    "added_ms": effect.detail.get("added_ms").cloned().unwrap_or(Value::Null),
                    "overwritten_existing_ms": effect.detail.get("overwritten_existing_ms").cloned().unwrap_or(Value::Null),
                    "overlapped_clip_ids": effect.detail.get("overlapped_clip_ids").cloned().unwrap_or_else(|| json!([])),
                    "removed_clip_ids": effect.detail.get("removed_clip_ids").cloned().unwrap_or_else(|| json!([])),
                    "overwritten_gap_ms": effect.detail.get("overwritten_gap_ms").cloned().unwrap_or_else(|| json!(0)),
                    "tail_gap_ms": effect.detail.get("tail_gap_ms").cloned().unwrap_or_else(|| json!(0)),
                    "tail_extended_ms": effect.detail.get("tail_extended_ms").cloned().unwrap_or_else(|| json!(0)),
                }))
            }).collect::<Vec<_>>(),
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
            "shape": d0["shape"],
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
    .with_requirement("open_project")
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
    library_use, project_brand, project_cache_preview, project_cache_purge, project_cache_rebuild,
    project_checkpoint, project_close, project_color, project_create, project_delete, project_diff,
    project_forget, project_format, project_health, project_list, project_open, project_ops,
    project_redo, project_rename, project_revert, project_save, project_sequence_create,
    project_sequence_delete, project_sequence_list, project_sequence_rename,
    project_sequence_switch, project_state, project_undo,
};
pub(crate) use project_workspace::{open_at_start, path_free_project_identity};
mod project_package;
use project_package::{project_package_create, project_package_plan};
mod group_review;
use group_review::{project_group_preview, project_group_reject};
mod sequence_index;
use sequence_index::project_sequence_index;
mod brand;
mod generated_assets;
mod review_handoff;
use review_handoff::{comment_export, comment_import};
mod media;
pub(crate) use media::{
    asset_info, media_import, project_paths, proxy_binding, spawn_plain_import_chain, update_asset,
    verify_attested_media_source, PlacementTurns, ANALYSIS_MAX_RUNNING,
};
use media::{
    media_bin_delete, media_bin_list, media_bin_save, media_check, media_filmstrip,
    media_perception, media_probe, media_relink, media_remove, media_transcribe, media_waveform,
};
mod media_relink;
use media_relink::{media_relink_apply, media_relink_preview};
const RENDER_MAX_RUNNING: usize = 1;
const RENDER_QUEUE_MAX_RUNNING: usize = 1;
// ---------------------------------------------------------------------------
// jobs.* handlers (the background-job contract)
// ---------------------------------------------------------------------------

mod jobs_handlers;
use jobs_handlers::{jobs_cancel, jobs_list, jobs_retry, jobs_status};
mod jobs_quit;
use jobs_quit::{jobs_before_quit, jobs_interrupted, jobs_interrupted_dismiss};

mod media_intelligence;
use media_intelligence::{
    inspect_media, inspect_range, media_intelligence_rebuild, media_intelligence_search,
    media_intelligence_status,
};

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
    edit_grade_window, edit_insert, edit_insert_linked, edit_keyframe, edit_mark_scenes,
    edit_matte, edit_move, edit_move_marker, edit_multicam_switch, edit_multicam_sync, edit_mute,
    edit_mute_range, edit_nest, edit_overwrite, edit_pan, edit_paste, edit_redact,
    edit_remove_marker, edit_remove_track, edit_reorder_track, edit_replace, edit_restore,
    edit_reverse, edit_ripple_delete, edit_roll, edit_seek_marker, edit_slide, edit_slide_edit,
    edit_slip, edit_solo, edit_speed, edit_speed_ramp, edit_split, edit_split_at_scenes,
    edit_split_edit, edit_stabilize, edit_track, edit_track_lock, edit_track_visible,
    edit_transform, edit_trim, edit_trim_edges, edit_update_marker, effects_list, grade_apply,
    grade_list, grade_save, media_index, media_index_status, media_search, plugins_call,
    plugins_enable, plugins_list, shape_update, title_add, title_templates, title_update,
    transitions_list,
};
pub(crate) use edit_tools::{build_shape_spec, build_title_spec, ShapeArgs, TitleArgs};

// transcript.* handlers live in transcript_load.rs.
mod transcript_load;
pub(crate) use transcript_load::load_transcript;

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

mod caption_bulk;
use caption_bulk::{captions_bulk_apply, captions_bulk_preview};

// ---------------------------------------------------------------------------
// render.* / verify.* / export.* handlers
// ---------------------------------------------------------------------------

mod rendering;
use rendering::{
    autopilot_run, clip_candidates, export_audio, export_frame, export_gif, export_publish,
    export_range, render_bundle, render_compare, render_direct, render_final, render_frame,
    render_preview, render_qc, render_queue, render_reframe, render_storyboard,
    snapshot_for_media_io,
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
use verify_handlers::receipt_file::render_receipt_bytes;
use verify_handlers::{
    resolve_receipt_path, verify_brand, verify_captions, verify_checks, verify_delivery,
    verify_judge, verify_loudness, verify_pacing, verify_pregate, verify_rerun, verify_scopes,
};

pub(crate) fn configured_judge_adapter() -> Option<PathBuf> {
    verify_handlers::find_judge_adapter()
}

/// The exact PATH augmentation used for every ladder adapter invocation.
/// Doctor passes it to the adapter's no-model detect protocol so off-PATH
/// installs resolve identically for admission and actual review.
pub(crate) fn configured_judge_cli_path() -> Option<std::ffi::OsString> {
    verify_handlers::judge_cli_path()
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

mod screen_record_routes;

mod ui_system;
use ui_system::{
    debug_screenshot, system_doctor, system_fetch_tool, system_motion_status, system_setup_matte,
    system_setup_perception, ui_forward, ui_screenshot, ui_state,
};

#[cfg(test)]
// Test-only project locks are uncontended within each single-threaded runtime.
#[allow(clippy::await_holding_lock)]
mod tests;

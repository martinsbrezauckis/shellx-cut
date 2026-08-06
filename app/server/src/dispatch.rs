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
//! verbs (snapshot inverse), `apply_lowered` for higher-layer verbs
//! (transcript.*, captions.* — recorded `lowered` steps keep replay/diff
//! working), `record_import`/`checkpoint`/`revert` for their special ops —
//! each committing the OpRecord + saving the cache; this module publishes
//! `op_applied` after every commit. media.import and project.checkpoint ARE
//! ops; project.revert appends restore ops.
//!
//! Dependencies: state.rs, jobs.rs, events.rs, ui_bridge.rs, cut-core,
//! cut-media, cut-perception. Primary callers: http.rs, mcp.rs, main.rs.

use crate::events::Event;
use crate::generate_handlers;
use crate::output_paths::{
    fence_output_path, fence_project_output_path, fenced_existing_export_read,
    fenced_existing_file_under_dir, make_fence, publish_output_atomic,
    resolve_existing_project_file, temp_output_path_for_render, write_output_atomic,
};
use crate::recipes;
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
    match name {
        // ------------------------------------------------------------------
        // project.*
        // ------------------------------------------------------------------
        "project.create" => project_create(state, args).await.into(),
        "project.open" => project_open(state, args).await.into(),
        "project.list" => project_list(args).await.into(),
        "project.forget" => project_forget(args).await.into(),
        "project.delete" => project_delete(state, args).await.into(),
        "project.save" => project_save(state).await.into(),
        "project.state" => project_state(state).await.into(),
        "project.sequence_list" => project_sequence_list(state).await.into(),
        "project.sequence_index" => project_sequence_index(state, args).await.into(),
        "project.sequence_create" => project_sequence_create(state, args, actor).await.into(),
        "project.sequence_switch" => project_sequence_switch(state, args, actor).await.into(),
        "project.sequence_rename" => project_sequence_rename(state, args, actor).await.into(),
        "project.sequence_delete" => project_sequence_delete(state, args, actor).await.into(),
        "project.ops" => project_ops(state, args).await.into(),
        "project.close" => project_close(state).await.into(),
        "project.checkpoint" => project_checkpoint(state, args, actor).await.into(),
        "project.rename" => project_rename(state, args, actor).await.into(),
        "project.format" => project_format(state, args, actor).await.into(),
        "project.color" => project_color(state, args, actor).await.into(),
        "project.brand" => project_brand(state, args, actor).await.into(),
        "project.set_output_dir" => crate::output_paths::project_set_output_dir(args)
            .await
            .into(),
        "comment.add" => comment_add(state, args, actor).await.into(),
        "comment.export" => comment_export(state, args).await.into(),
        "comment.import" => comment_import(state, args, actor).await.into(),
        "comment.list" => comment_list(state, args).await.into(),
        "comment.resolve" => comment_resolve(state, args, actor).await.into(),
        "comment.draft" => comment_draft(state, args, actor).await.into(),
        "comment.apply" => comment_apply(state, args, actor).await.into(),
        "project.revert" => project_revert(state, args, actor).await.into(),
        "project.undo" => project_undo(state, actor).await.into(),
        "project.redo" => project_redo(state, actor).await.into(),
        "project.diff" => project_diff(state, args).await.into(),
        "library.list" => library_list(args).await.into(),
        "library.add" => library_add(state, args).await.into(),
        "library.relink" => library_relink(args).await.into(),
        "library.remove" => library_remove(args).await.into(),
        "library.move" => library_move(args).await.into(),
        "library.tag" => library_tag(args).await.into(),
        "library.favorite" => library_favorite(args).await.into(),
        "library.use" => library_use(args).await.into(),
        "library.add_to_project" => library_add_to_project(state, args, actor).await.into(),
        "library.folder_add" => library_folder_add(args).await.into(),
        "library.folder_rename" => library_folder_rename(args).await.into(),
        "library.folder_remove" => library_folder_remove(args).await.into(),

        // ------------------------------------------------------------------
        // media.* — import kicks the probe→proxy→transcribe→perception chain
        // ------------------------------------------------------------------
        "media.import" => media_import(state, args, actor).await.into(),
        "media.remove" => media_remove(state, args, actor).await.into(),
        "media.relink" => media_relink(state, args, actor).await.into(),
        "media.check" => media_check(state, args).await.into(),
        "media.bin_save" => media_bin_save(state, args, actor).await.into(),
        "media.bin_delete" => media_bin_delete(state, args, actor).await.into(),
        "media.bin_list" => media_bin_list(state, args).await.into(),
        "media.probe" => media_probe(state, args).await.into(),
        "media.waveform" => media_waveform(state, args).await.into(),
        "media.filmstrip" => media_filmstrip(state, args).await.into(),
        "media.transcribe" => media_transcribe(state, args).await.into(),
        "media.perception" => media_perception(state, args).await.into(),
        "media.diarize" => crate::diarize::media_diarize(state, args).await.into(),

        // ------------------------------------------------------------------
        // jobs.* (the background-job contract: jobs domain replaces media.status)
        // ------------------------------------------------------------------
        "jobs.status" => jobs_status(state, args).await.into(),
        "jobs.list" => jobs_list(state).await.into(),
        "jobs.cancel" => jobs_cancel(state, args).await.into(),

        // ------------------------------------------------------------------
        // edit.* — thin arg-parse wrappers over cut_core::edit, one op each
        // ------------------------------------------------------------------
        "edit.split" => edit_split(state, args, actor).await.into(),
        "edit.cut_to_beat" => edit_cut_to_beat(state, args, actor).await.into(),
        "edit.split_at_scenes" => edit_split_at_scenes(state, args, actor).await.into(),
        "edit.mark_scenes" => edit_mark_scenes(state, args, actor).await.into(),
        "edit.trim_edges" => edit_trim_edges(state, args, actor).await.into(),
        "edit.ripple_delete" => edit_ripple_delete(state, args, actor).await.into(),
        "edit.trim" => edit_trim(state, args, actor).await.into(),
        "edit.speed" => edit_speed(state, args, actor).await.into(),
        "edit.speed_ramp" => edit_speed_ramp(state, args, actor).await.into(),
        "edit.move" => edit_move(state, args, actor).await.into(),
        "edit.insert" => edit_insert(state, args, actor).await.into(),
        "edit.duplicate" => edit_duplicate(state, args, actor).await.into(),
        "edit.nest" => edit_nest(state, args, actor).await.into(),
        "edit.replace" => edit_replace(state, args, actor).await.into(),
        "edit.fit_to_fill" => edit_fit_to_fill(state, args, actor).await.into(),
        "edit.detach_audio" => edit_detach_audio(state, args, actor).await.into(),
        "edit.split_edit" => edit_split_edit(state, args, actor).await.into(),
        "edit.paste" => edit_paste(state, args, actor).await.into(),
        "edit.gain" => edit_gain(state, args, actor).await.into(),
        "edit.fade" => edit_fade(state, args, actor).await.into(),
        "edit.mute_range" => edit_mute_range(state, args, actor).await.into(),
        "edit.transform" => edit_transform(state, args, actor).await.into(),
        "edit.crop" => edit_crop(state, args, actor).await.into(),
        "edit.grade" => edit_grade(state, args, actor).await.into(),
        "edit.grade_stack" => edit_grade_stack(state, args, actor).await.into(),
        "edit.grade_window" => edit_grade_window(state, args, actor).await.into(),
        "grade.save" => grade_save(state, args, actor).await.into(),
        "grade.apply" => grade_apply(state, args, actor).await.into(),
        "grade.list" => grade_list(state).await.into(),
        "edit.color_space" => edit_color_space(state, args, actor).await.into(),
        "edit.color_match" => edit_color_match(state, args, actor).await.into(),
        "edit.auto_balance" => edit_auto_balance(state, args, actor).await.into(),
        "edit.matte" => edit_matte(state, args, actor).await.into(),
        "edit.effect" => edit_effect(state, args, actor).await.into(),
        "edit.adjustment" => edit_adjustment(state, args, actor).await.into(),
        "edit.reverse" => edit_reverse(state, args, actor).await.into(),
        "edit.stabilize" => edit_stabilize(state, args, actor).await.into(),
        "edit.freeze" => edit_freeze(state, args, actor).await.into(),
        "edit.animate" => edit_animate(state, args, actor).await.into(),
        "edit.keyframe" => edit_keyframe(state, args, actor).await.into(),
        "edit.auto_zoom" => edit_auto_zoom(state, args, actor).await.into(),
        "edit.add_mask" => edit_add_mask(state, args, actor).await.into(),
        "edit.redact" => edit_redact(state, args, actor).await.into(),
        "edit.track" => edit_track(state, args).await.into(),
        "edit.multicam_sync" => edit_multicam_sync(state, args).await.into(),
        "edit.multicam_switch" => edit_multicam_switch(state, args, actor).await.into(),
        "edit.eq" => edit_eq(state, args, actor).await.into(),
        "audio.cleanup_voice" => audio_cleanup_voice(state, args, actor).await.into(),
        "edit.slide" => edit_slide(state, args, actor).await.into(),
        "title.add" => title_add(state, args, actor).await.into(),
        "title.update" => title_update(state, args, actor).await.into(),
        "title.templates" => title_templates(state, args, actor).await.into(),
        "edit.add_shape" => edit_add_shape(state, args, actor).await.into(),
        "shape.update" => shape_update(state, args, actor).await.into(),
        "edit.seek_marker" => edit_seek_marker(state, args, actor).await.into(),
        "assets.providers" => assets_providers(state, args, actor).await.into(),
        "assets.search" => assets_search(state, args, actor).await.into(),
        "assets.fetch" => assets_fetch(state, args, actor).await.into(),
        "assets.generate" => assets_generate(state, args, actor).await.into(),
        "assets.generated_list" => generated_assets::assets_generated_list(state, args)
            .await
            .into(),
        "agent.chat" => agent_chat(state, args, actor).await.into(),
        "plugins.list" => plugins_list(state, args, actor).await.into(),
        "plugins.enable" => plugins_enable(state, args, actor).await.into(),
        "plugins.call" => plugins_call(state, args, actor).await.into(),
        "media.index_status" => media_index_status(state, args).await.into(),
        "media.search" => media_search(state, args, actor).await.into(),
        "media.index" => media_index(state, args, actor).await.into(),
        "effects.list" => effects_list(state, args, actor).await.into(),
        "transitions.list" => transitions_list(state, args, actor).await.into(),
        "captions.kinetic" => captions_kinetic(state, args, actor).await.into(),
        "edit.crossfade" => edit_crossfade(state, args, actor).await.into(),
        "edit.duck" => edit_duck(state, args, actor).await.into(),
        "edit.add_track" => edit_add_track(state, args, actor).await.into(),
        "edit.remove_track" => edit_remove_track(state, args, actor).await.into(),
        "edit.reorder_track" => edit_reorder_track(state, args, actor).await.into(),
        "edit.blend" => edit_blend(state, args, actor).await.into(),
        "edit.track_visible" => edit_track_visible(state, args, actor).await.into(),
        "edit.track_lock" => edit_track_lock(state, args, actor).await.into(),
        "edit.mute" => edit_mute(state, args, actor).await.into(),
        "edit.solo" => edit_solo(state, args, actor).await.into(),
        "edit.pan" => edit_pan(state, args, actor).await.into(),
        "edit.slip" => edit_slip(state, args, actor).await.into(),
        "edit.roll" => edit_roll(state, args, actor).await.into(),
        "edit.slide_edit" => edit_slide_edit(state, args, actor).await.into(),
        "edit.paste_attributes" => {
            crate::paste_attributes::edit_paste_attributes(state, args, actor)
                .await
                .into()
        }
        "edit.add_marker" => edit_add_marker(state, args, actor).await.into(),
        "edit.remove_marker" => edit_remove_marker(state, args, actor).await.into(),
        "edit.move_marker" => edit_move_marker(state, args, actor).await.into(),
        "edit.update_marker" => edit_update_marker(state, args, actor).await.into(),
        "edit.restore" => edit_restore(state, args, actor).await.into(),

        // ------------------------------------------------------------------
        // audio.* — music bed with auto-duck + beat markers
        // ------------------------------------------------------------------
        "audio.add_music" => audio_add_music(state, args, actor).await.into(),
        "audio.dub" => crate::dub::audio_dub(state, args, actor).await.into(),

        // ------------------------------------------------------------------
        // transcript.* — transcript-driven edits (one op per removed span)
        // ------------------------------------------------------------------
        "transcript.get" => speech_text::transcript_get(state, args).await.into(),
        "transcript.timeline" => speech_text::transcript_timeline(state, args).await.into(),
        "transcript.cut_words" => speech_text::transcript_cut_words(state, args, actor)
            .await
            .into(),
        "transcript.ignore_words" => speech_text::transcript_ignore_words(state, args, actor)
            .await
            .into(),
        "transcript.mute_words" => speech_text::transcript_mute_words(state, args, actor)
            .await
            .into(),
        "transcript.assemble" => speech_text::transcript_assemble(state, args, actor)
            .await
            .into(),
        "transcript.search" => speech_text::transcript_search(state, args).await.into(),
        "transcript.chapters" => speech_text::transcript_chapters(state, args).await.into(),
        "transcript.remove_silences" => speech_text::transcript_remove_silences(state, args, actor)
            .await
            .into(),
        "transcript.remove_fillers" => speech_text::transcript_remove_fillers(state, args, actor)
            .await
            .into(),
        "transcript.remove_retakes" => speech_text::transcript_remove_retakes(state, args, actor)
            .await
            .into(),
        "transcript.translate" => speech_text::transcript_translate(state, args).await.into(),

        // ------------------------------------------------------------------
        // captions.*
        // ------------------------------------------------------------------
        "captions.generate" => captions_generate(state, args, actor).await.into(),
        "captions.translate" => captions_translate(state, args, actor).await.into(),
        "captions.import" => captions_import(state, args, actor).await.into(),
        "captions.add_text" => speech_text::captions_add_text(state, args, actor)
            .await
            .into(),
        "captions.set_style" => speech_text::captions_set_style(state, args, actor)
            .await
            .into(),
        "captions.save_style" => speech_text::captions_save_style(state, args, actor)
            .await
            .into(),
        "captions.apply_style" => speech_text::captions_apply_style(state, args, actor)
            .await
            .into(),
        "captions.list_styles" => speech_text::captions_list_styles(state, args).await.into(),
        "captions.reflow" => captions_reflow(state, args, actor).await.into(),
        "captions.shift" => captions_shift(state, args, actor).await.into(),
        "captions.set_range" => captions_set_range(state, args, actor).await.into(),
        "captions.set_text" => captions_set_text(state, args, actor).await.into(),

        // ------------------------------------------------------------------
        // render.* / verify.* / export.*
        // ------------------------------------------------------------------
        "render.preview" => render_preview(state, args).await.into(),
        "render.frame" => render_frame(state, args).await.into(),
        "render.storyboard" => render_storyboard(state, args).await.into(),
        "render.final" => render_final(state, args, actor).await.into(),
        "render.reframe" => render_reframe(state, args, actor).await.into(),
        "render.direct" => render_direct(state, args, actor).await.into(),
        "render.qc" => render_qc(state, args, actor).await.into(),
        "verify.checks" => verify_checks(state, args).await.into(),
        "verify.pacing" => verify_pacing(state).await.into(),
        "verify.captions" => verify_captions(state, args).await.into(),
        "verify.delivery" => verify_delivery(state, args).await.into(),
        "verify.loudness" => verify_loudness(state, args).await.into(),
        "verify.scopes" => verify_scopes(state, args).await.into(),
        "verify.brand" => verify_brand(state, args).await.into(),
        "verify.judge" => verify_judge(state, args).await.into(),
        "verify.pregate" => verify_pregate(state, args).await.into(),
        "export.frame" => export_frame(state, args, actor).await.into(),
        "export.range" => export_range(state, args, actor).await.into(),
        "export.audio" => export_audio(state, args, actor).await.into(),
        "export.publish" => export_publish(state, args, actor).await.into(),
        "export.gif" => export_gif(state, args, actor).await.into(),
        "export.xml" => export_xml(state, args).await.into(),
        "export.otio" => export_otio(state, args).await.into(),
        "export.edl" => export_edl(state, args).await.into(),
        "import.otio" => import_otio(state, args, actor).await.into(),
        "export.srt" => export_srt(state, args).await.into(),
        "export.vtt" => export_vtt(state, args).await.into(),
        "export.ass" => export_ass(state, args).await.into(),
        "export.transcript" => export_transcript(state, args).await.into(),
        "export.chapters" => export_chapters(state, args).await.into(),

        // ------------------------------------------------------------------
        // clip.* — social repurposing: rank shareable windows + bundle
        // ------------------------------------------------------------------
        "clip.candidates" => clip_candidates(state, args).await.into(),
        "render.bundle" => render_bundle(state, args, actor).await.into(),
        "render.queue" => render_queue(state, args, actor).await.into(),
        "autopilot.run" => autopilot_run(state, args, actor).await.into(),

        // ------------------------------------------------------------------
        // generate.* — native Generate module foundation. Pure catalog reads,
        // non-mutating previews, native inserts, prompt planning, and storyboard
        // planning all live in one domain.
        // ------------------------------------------------------------------
        "generate.list" => generate_handlers::generate_list(args).await.into(),
        "generate.describe" => generate_handlers::generate_describe(args).await.into(),
        "generate.preview" => generate_handlers::generate_preview(state, args)
            .await
            .into(),
        "generate.insert" => generate_handlers::generate_insert(state, args, actor)
            .await
            .into(),
        "generate.from_prompt" => generate_handlers::generate_from_prompt(state, args, actor)
            .await
            .into(),
        "generate.storyboard" => generate_handlers::generate_storyboard(state, args, actor)
            .await
            .into(),
        "motion.template_to_cut" => {
            crate::motion_bridge::motion_template_to_cut(state, args, actor)
                .await
                .into()
        }
        "motion.script_to_cut" => crate::motion_bridge::motion_script_to_cut(state, args, actor)
            .await
            .into(),
        "motion.job.get" => crate::motion_jobs::get(state, args).await.into(),
        "motion.job.list" => crate::motion_jobs::list(state, args).await.into(),
        "motion.map_import" => crate::motion_bridge::motion_map_import(state, args, actor)
            .await
            .into(),
        "motion.apply_import" => crate::motion_bridge::motion_apply_import(state, args, actor)
            .await
            .into(),
        "motion.link.refresh" => crate::motion_bridge::motion_link_refresh(state, args, actor)
            .await
            .into(),
        "motion.link.relink" => crate::motion_bridge::motion_link_relink(state, args, actor)
            .await
            .into(),
        "motion.link.edit" => crate::motion_bridge::motion_link_edit(state, args, actor)
            .await
            .into(),
        "motion.link.tracking.inventory" => crate::motion_tracking::inventory(state, args, actor)
            .await
            .into(),
        "motion.link.tracking.request" => crate::motion_tracking::request(state, args, actor)
            .await
            .into(),
        "motion.link.tracking.inspect" => crate::motion_tracking::inspect(state, args, actor)
            .await
            .into(),
        "motion.link.tracking.apply" => crate::motion_tracking::apply(state, args, actor)
            .await
            .into(),
        "motion.link.tracking.verify" => crate::motion_tracking::verify(state, args, actor)
            .await
            .into(),
        "motion.link.tracking.detach" => crate::motion_tracking::detach(state, args, actor)
            .await
            .into(),

        // ------------------------------------------------------------------
        // recipe.* — declarative pipeline manifests: list/describe (pure
        // reads) + run (a job; the pure orchestrator over the existing verbs).
        // ------------------------------------------------------------------
        "recipe.list" => recipe_list(args).await.into(),
        "recipe.describe" => recipe_describe(args).await.into(),
        "recipe.run" => recipe_run(state, args, actor).await.into(),

        "assemble.broll" => assemble_broll(state, args, actor).await.into(),
        "assemble.repurpose" => speech_text::assemble_repurpose(state, args).await.into(),
        "assemble.shorts" => speech_text::assemble_shorts(state, args).await.into(),
        "assemble.from_script" => speech_text::assemble_from_script(state, args).await.into(),
        "score.clip" => speech_text::score_clip(state, args).await.into(),

        // ------------------------------------------------------------------
        // screen_record.* — screen recorder integration (sidecar)
        // ------------------------------------------------------------------
        "screen_record.doctor" => crate::screen_record::screen_record_doctor(args)
            .await
            .into(),
        "screen_record.start" => crate::screen_record::screen_record_start(state, args)
            .await
            .into(),
        "screen_record.stop" => screen_record_stop(state, args, actor).await.into(),
        "screen_record.studio_event" => {
            crate::screen_record_studio::screen_record_studio_event(state, args)
                .await
                .into()
        }
        "screen_record.autoedit" => crate::screen_record::screen_record_autoedit(state, args)
            .await
            .into(),
        "screen_record.polish" => screen_record_polish(state, args, actor).await.into(),
        "screen_record.export" => crate::screen_record::screen_record_export(state, args)
            .await
            .into(),

        // ------------------------------------------------------------------
        // ui.* — relayed to the connected UI client over WS (ui_bridge.rs)
        // ------------------------------------------------------------------
        "ui.state" => ui_state(state).await.into(),
        "ui.screenshot" => ui_screenshot(state, args).await.into(),
        "debug.screenshot" => debug_screenshot(args).await.into(),
        "ui.open" | "ui.playhead" | "ui.select" | "ui.highlight" => {
            ui_forward(state, name, args).await.into()
        }

        // ------------------------------------------------------------------
        // system.* — environment doctor + consented tool fetch
        // ------------------------------------------------------------------
        "system.mcp_test" => crate::mcp::self_test(state).await.into(),
        "system.doctor" => system_doctor(state, args).await.into(),
        "system.set_ffmpeg" => crate::ffmpeg_settings::system_set_ffmpeg(state, args)
            .await
            .into(),
        "system.set_stt_model" => crate::stt_settings::system_set_stt_model(state, args)
            .await
            .into(),
        "system.fetch_tool" => system_fetch_tool(state, args, actor).await.into(),
        "system.setup_perception" => system_setup_perception(state, args, actor).await.into(),
        "system.setup_matte" => system_setup_matte(state, args, actor).await.into(),

        // Registry said it exists but no arm matched — a contract drift bug.
        other => VerbResult::err(CutError::new(
            error_codes::UNIMPLEMENTED,
            format!("verb '{other}' is registered but has no dispatch arm"),
            "schema/verbs.json and dispatch.rs are out of sync — fix dispatch.rs",
        )),
    }
}

// ---------------------------------------------------------------------------
// Op plumbing — the ONE commit path for every mutating verb
// ---------------------------------------------------------------------------

/// Commit ONE core `edit.*` verb through ProjectStore::apply — the single
/// core commit path: transactional mutation, SNAPSHOT inverse (the only
/// inverse cut_core::edit::restore understands), rationale per the rationale-preservation contract —
/// then publish `op_applied`. A previous server-local commit path wrote
/// `edit.restore` self-pointers as inverses, which made every server-created
/// op un-restorable; all mutating verbs now flow through core's commit fns
/// (apply / apply_lowered / record_import / checkpoint / revert) so the log
/// stays replayable and undoable by construction.
async fn commit_core(
    state: &AppState,
    verb: &str,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    // the inverse-payload contract: capture the opt-in before `args` is moved into store.apply.
    let include_inverse = wants_inverse(&args);
    let op = guard_call(verb, || store.apply(verb, args, actor, rationale))?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    Ok(VerbResult::ok_with_ops(
        shape_core_result(verb, &op, include_inverse),
        vec![op_id],
    ))
}

/// the inverse-payload contract — does the caller want the op's full `inverse` (the ~45 KB
/// full-timeline snapshot) echoed back in the verb RESULT? Default NO: the
/// inverse is already persisted to ops.jsonl AND carried on the `op_applied`
/// event (the history-rail / undo source of truth), so echoing it in every
/// mutating-verb response is pure token burn for an agent driving the REST
/// surface (a single edit.crop/edit.trim response was ~45 KB). Opt back in with
/// `{include_inverse:true}` on the verb args for the rare caller that needs it
/// inline. The executable verb schema accepts only a JSON boolean.
pub(crate) fn wants_inverse(args: &Value) -> bool {
    args.get("include_inverse")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// the inverse-payload contract — serialize an OpRecord for embedding in a verb RESULT, dropping the
/// fat `inverse` snapshot unless `include_inverse` was requested. The op's
/// canonical copy (ops.jsonl + the `op_applied` event) ALWAYS keeps the inverse
/// — this trims ONLY the response echo, so undo/rebase/replay are unaffected.
/// A `"inverse_omitted": true` marker is left in place of the dropped field so a
/// reader can tell "trimmed for size" from "this op has no inverse" (imports,
/// checkpoints — those carry no inverse and get no marker).
pub(crate) fn op_for_result(op: &OpRecord, include_inverse: bool) -> Value {
    let mut v = serde_json::to_value(op).unwrap_or(Value::Null);
    if !include_inverse {
        if let Value::Object(map) = &mut v {
            // Only mark+drop when an inverse was actually present (a non-undoable
            // op like import/checkpoint serializes WITHOUT the key, via
            // skip_serializing_if — leave those untouched).
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
/// (the review rail and tests read it; additive to the documented shape) — but
/// with its fat `inverse` snapshot dropped by default (the inverse-payload contract; `include_inverse`
/// forwards the caller's opt-in). Unknown verbs fall back to `{op}` alone.
fn shape_core_result(verb: &str, op: &OpRecord, include_inverse: bool) -> Value {
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
    result["op"] = op_for_result(op, include_inverse); // the inverse-payload contract: drop fat inverse by default
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
    let include_inverse = wants_inverse(&args); // the inverse-payload contract: capture before args moves
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let op = guard_call(verb, || {
        store.apply_lowered(verb, args, actor, rationale, steps, extra_effects)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    let result = if matches!(verb, "edit.move" | "edit.trim") {
        shape_core_result(verb, &op, include_inverse)
    } else {
        json!({"op": op_for_result(&op, include_inverse)})
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
pub(crate) use safety::{guard_call, run_blocking};
mod motion_link_projection;
mod project_workspace;
use project_workspace::{
    comment_add, comment_apply, comment_draft, comment_list, comment_resolve, library_add,
    library_add_to_project, library_favorite, library_folder_add, library_folder_remove,
    library_folder_rename, library_list, library_move, library_relink, library_remove, library_tag,
    library_use, project_brand, project_checkpoint, project_close, project_color, project_create,
    project_delete, project_diff, project_forget, project_format, project_list, project_open,
    project_ops, project_redo, project_rename, project_revert, project_save,
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
    verify_judge, verify_loudness, verify_pacing, verify_pregate, verify_scopes,
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

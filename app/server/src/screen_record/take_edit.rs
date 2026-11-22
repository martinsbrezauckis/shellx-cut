//! Recording-clock zoom editing. The persisted operation always contains the
//! resolved list and immutable pending plan; replay does not touch capture files.
use crate::{events::Event, state::AppState};
use cut_core::{
    error_codes,
    recording::{RecordingTake, ZoomSegment},
    Actor, CutError, VerbResult,
};
use record_core::zoom_segments::{self, ZoomOrigin};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub(crate) fn bad(message: &str) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        message,
        "use screen_record.zoom_list on a new polished recording",
    )
}
fn convert<T: serde::de::DeserializeOwned>(value: &impl serde::Serialize) -> Result<T, CutError> {
    Ok(serde_json::from_value(serde_json::to_value(value)?)?)
}
pub(crate) fn take<'a>(
    project: &'a cut_core::Project,
    clip: &str,
) -> Result<&'a RecordingTake, CutError> {
    cut_core::recording::take_for_clip(project, clip).ok_or_else(|| {
        bad("this clip has no editable recording take (old recordings keep their baked zooms)")
    })
}
pub(crate) fn file(dir: &Path, path: &str) -> Result<PathBuf, CutError> {
    super::plain_existing_file_under_project(
        dir,
        path,
        "recording input",
        "keep the recording source and plan in this project",
    )
}
pub(crate) fn plan(dir: &Path, take: &RecordingTake) -> Result<record_core::EditPlan, CutError> {
    super::polish::load_plan(&file(dir, &take.pending_plan)?)
}
fn segments_to_record(
    segments: &[ZoomSegment],
) -> Result<Vec<record_core::zoom_segments::ZoomSegment>, CutError> {
    convert(&segments)
}
fn record_error(error: record_core::RecordError) -> CutError {
    bad(&error.to_string())
}

pub(crate) fn publish(
    dir: &Path,
    take: &RecordingTake,
    segments: &[ZoomSegment],
) -> Result<String, CutError> {
    let mut pending = plan(dir, take)?;
    let zoom = zoom_segments::segments_to_keys(&segments_to_record(segments)?, pending.duration_ms)
        .map_err(record_error)?;
    // In particular: leave the exact original bytes and cache identity alone.
    if zoom == pending.zoom {
        return Ok(take.pending_plan.clone());
    }
    if take.pending_plan != take.applied_plan {
        let applied = super::polish::load_plan(&file(dir, &take.applied_plan)?)?;
        if zoom == applied.zoom {
            return Ok(take.applied_plan.clone());
        }
    }
    pending.zoom = zoom;
    pending.validate().map_err(record_error)?;
    let cache = super::screen_record_cache_dir(dir)?;
    let stage = super::plan_output::PlanStage::new(&cache)?;
    super::cache_output::write(stage.path(), &serde_json::to_vec_pretty(&pending)?)?;
    Ok(stage
        .publish(&file(dir, &take.events)?)?
        .display()
        .to_string())
}

fn auto_segments(
    dir: &Path,
    take: &RecordingTake,
    pending: &record_core::EditPlan,
    args: &Value,
) -> Result<Vec<zoom_segments::ZoomSegment>, CutError> {
    let events = super::polish::load_events(&file(dir, &take.events)?)?;
    // Each isolated click can emit four zoom keys. Keep the generated plan
    // below its 200,000-key validation ceiling before doing the work.
    if events.duration_ms == 0
        || events.duration_ms > 7 * 24 * 60 * 60 * 1_000
        || events.duration_ms != pending.duration_ms
        || events.clicks.len() > 40_000
    {
        return Err(bad(
            "recording events exceed zoom_auto limits or plan duration",
        ));
    }
    let mut config = serde_json::to_value(record_engine::EngineConfig::default())?;
    if let Some(patch) = args["config"].as_object() {
        for (key, value) in patch {
            if config.get(key).is_none() {
                return Err(bad("unknown autoedit config field"));
            }
            config[key] = value.clone();
        }
    }
    let config: record_engine::EngineConfig = serde_json::from_value(config)?;
    super::polish::validate_autoedit_config(&config)?;
    let keys = record_engine::build_zoom(&events, &config);
    zoom_segments::keys_to_segments(&keys).map_err(record_error)
}

pub(crate) async fn handle(
    state: &AppState,
    verb: &str,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let clip = args["clip"]
        .as_str()
        .ok_or_else(|| bad("clip is required"))?;
    let prepared_auto = if verb == "screen_record.zoom_auto" {
        let (take, dir) = {
            let guard = state.project.read().await;
            let store = guard.as_ref().ok_or_else(crate::dispatch::no_project)?;
            (take(&store.project, clip)?.clone(), store.dir.clone())
        };
        let pending = plan(&dir, &take)?;
        let automatic = auto_segments(&dir, &take, &pending, &args)?;
        Some((take, dir, pending, automatic))
    } else {
        None
    };
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(crate::dispatch::no_project)?;
    let take = take(&store.project, clip)?.clone();
    let pending = if let Some((initial_take, initial_dir, pending, _)) = &prepared_auto {
        if &take != initial_take || &store.dir != initial_dir {
            return Err(bad(
                "recording changed while zoom_auto was prepared; try again",
            ));
        }
        (*pending).clone()
    } else {
        plan(&store.dir, &take)?
    };
    if verb == "screen_record.zoom_list" {
        let edl = cut_core::edl_from_project(&store.project);
        let mut segments = Vec::new();
        for segment in &take.segments {
            let mut value = serde_json::to_value(segment)?;
            value["timeline"] = timeline_range(&edl, clip, segment.start_ms, segment.end_ms);
            segments.push(value);
        }
        let mut result = json!({
            "clip": clip, "take": take.id,
            "source": {"w": pending.source_w, "h": pending.source_h},
            "duration_ms": pending.duration_ms, "segments": segments,
            "stale": take.stale(), "applied_plan": take.applied_plan,
            "pending_plan": take.pending_plan
        });
        video_status(&store.project, &take, &mut result);
        return Ok(VerbResult::ok(result));
    }
    let mut segments = segments_to_record(&take.segments)?;
    let mut selected = None;
    let mut removed = Vec::new();
    match verb {
        "screen_record.zoom_set" => {
            let mut incoming: Vec<zoom_segments::ZoomSegment> =
                serde_json::from_value(args["segments"].clone())?;
            // SET is also the drag gesture endpoint. A client need not know
            // the pinning policy: new or changed segments become the person's.
            for segment in &mut incoming {
                if !segments.iter().any(|old| old == segment) {
                    segment.origin = ZoomOrigin::Manual;
                }
            }
            segments = incoming;
        }
        "screen_record.zoom_add" => {
            let start = match (args.get("at_ms"), args.get("timeline_ms")) {
                (Some(at), None) => at
                    .as_u64()
                    .ok_or_else(|| bad("at_ms must be an unsigned integer"))?,
                (None, Some(time)) => recording_time(
                    &cut_core::edl_from_project(&store.project),
                    clip,
                    time.as_u64()
                        .ok_or_else(|| bad("timeline_ms must be an unsigned integer"))?,
                )?,
                _ => {
                    return Err(bad("supply exactly one of at_ms or timeline_ms")
                        .with_suggested_action(
                            "Choose recording time at_ms or timeline time timeline_ms, not both.",
                        ))
                }
            };
            let end = start
                .checked_add(args["duration_ms"].as_u64().unwrap_or(2000))
                .ok_or_else(|| bad("zoom time overflow"))?;
            if end > pending.duration_ms {
                return Err(bad("zoom extends beyond the recording"));
            }
            let (cx, cy) = pointer_target(&pending, start);
            let next = segments
                .iter()
                .filter_map(|s| s.id.strip_prefix('z')?.parse::<u64>().ok())
                .max()
                .unwrap_or(0)
                + 1;
            let id = format!("z{next}");
            let length = end - start;
            let ramp_in = 550.min(length / 2);
            segments.push(zoom_segments::ZoomSegment {
                id: id.clone(),
                start_ms: start,
                end_ms: end,
                scale: args["scale"].as_f64().unwrap_or(2.0),
                cx: args["cx"].as_f64().unwrap_or(cx),
                cy: args["cy"].as_f64().unwrap_or(cy),
                ramp_in_ms: ramp_in,
                ramp_out_ms: 500.min(length - ramp_in),
                origin: ZoomOrigin::Manual,
            });
            selected = Some(id);
        }
        "screen_record.zoom_update" => {
            let id = args["id"].as_str().ok_or_else(|| bad("id is required"))?;
            let segment = segments
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or_else(|| bad("unknown zoom id"))?;
            let mut value = serde_json::to_value(&segment)?;
            for key in [
                "start_ms",
                "end_ms",
                "scale",
                "cx",
                "cy",
                "ramp_in_ms",
                "ramp_out_ms",
            ] {
                if let Some(v) = args.get(key) {
                    value[key] = v.clone();
                }
            }
            *segment = serde_json::from_value(value)?;
            if let Some(target) = args.get("target") {
                if target.as_str() != Some("pointer") {
                    return Err(bad("target must be pointer"));
                }
                (segment.cx, segment.cy) = pointer_target(&pending, segment.start_ms);
            }
            segment.origin = ZoomOrigin::Manual;
            selected = Some(id.into());
        }
        "screen_record.zoom_remove" => {
            if let Some(id) = args["id"].as_str() {
                removed.push(id.to_string());
            }
            if let Some(ids) = args["ids"].as_array() {
                for id in ids {
                    removed.push(
                        id.as_str()
                            .ok_or_else(|| bad("ids must be strings"))?
                            .into(),
                    );
                }
            }
            removed.sort();
            removed.dedup();
            if removed.is_empty()
                || removed
                    .iter()
                    .any(|id| !segments.iter().any(|s| &s.id == id))
            {
                return Err(bad("supply existing zoom id or ids"));
            }
            segments.retain(|s| !removed.contains(&s.id));
        }
        "screen_record.zoom_auto" => {
            let mut automatic = prepared_auto.unwrap().3;
            segments.retain(|s| {
                args["keep_manual"].as_bool().unwrap_or(true) && s.origin == ZoomOrigin::Manual
            });
            automatic.retain(|a| {
                !segments
                    .iter()
                    .any(|m| a.start_ms < m.end_ms && m.start_ms < a.end_ms)
            });
            let next = segments
                .iter()
                .filter_map(|s| s.id.strip_prefix('z')?.parse::<u64>().ok())
                .max()
                .unwrap_or(0)
                + 1;
            for (offset, segment) in automatic.iter_mut().enumerate() {
                segment.id = format!("z{}", next + offset as u64);
            }
            segments.extend(automatic);
        }
        _ => return Err(bad("unknown recording zoom verb")),
    }
    segments.sort_by_key(|s| s.start_ms);
    let segments = zoom_segments::normalize_segments(&segments).map_err(record_error)?;
    let resolved: Vec<ZoomSegment> = convert(&segments)?;
    let pending_plan = publish(&store.dir, &take, &resolved)?;
    let op = store.apply(
        "screen_record.zoom_set",
        json!({"take":take.id,"segments":resolved,"pending_plan":pending_plan}),
        actor,
        args["rationale"].as_str().map(String::from),
    )?;
    let stale = pending_plan != take.applied_plan;
    let result = if let Some(id) = selected {
        json!({"segment":segments.iter().find(|s|s.id == id),"stale":stale})
    } else if verb == "screen_record.zoom_remove" {
        json!({"removed":removed,"stale":stale})
    } else if verb == "screen_record.zoom_set" {
        json!({"clip":clip,"segments":segments,"stale":stale})
    } else {
        json!({"segments":segments,"stale":stale})
    };
    state.events.publish(Event::OpApplied { op: op.clone() });
    Ok(VerbResult::ok_with_ops(result, vec![op.op_id]))
}

fn pointer_target(plan: &record_core::EditPlan, time: u64) -> (f64, f64) {
    let pointer = plan.cursor.smoothed.iter().rev().find(|p| p.t_ms <= time);
    (
        pointer.map_or(0.5, |p| p.x / plan.source_w.max(1) as f64),
        pointer.map_or(0.5, |p| p.y / plan.source_h.max(1) as f64),
    )
}

fn recording_time(edl: &cut_core::Edl, clip: &str, time: u64) -> Result<u64, CutError> {
    let slice = edl.segments.iter().find(|s| {
        s.clip_id.as_deref() == Some(clip) && time >= s.timeline_in_ms && time < s.timeline_out_ms
    }).ok_or_else(|| bad("timeline_ms is outside the selected clip")
        .with_suggested_action("Move the playhead inside the selected recording clip, or use recording time at_ms."))?;
    if slice.freeze.is_some() {
        return Err(bad(
            "timeline_ms is inside a frozen stretch; there is no moving recording time",
        )
        .with_suggested_action(
            "Unfreeze the clip or use recording time at_ms to edit the take's zooms.",
        ));
    }
    let (Some(a), Some(b)) = (slice.src_in_ms, slice.src_out_ms) else {
        return Err(bad("selected clip has no recording time mapping"));
    };
    let fraction = (time - slice.timeline_in_ms) as f64
        / (slice.timeline_out_ms - slice.timeline_in_ms) as f64;
    let offset = ((b - a) as f64 * fraction).round() as u64;
    Ok(if slice.reverse {
        b - offset
    } else {
        a + offset
    })
}

fn video_status(project: &cut_core::Project, take: &RecordingTake, value: &mut Value) {
    let missing = project.assets.get(&take.asset).is_none_or(|asset| {
        let path = Path::new(&asset.path);
        // New take bakes are absolute. Also resolve project-relative repairs
        // using the capture's known cache/screen_record ancestor.
        let root = Path::new(&take.capture_folder)
            .ancestors()
            .find(|p| p.ends_with("cache/screen_record"))
            .and_then(Path::parent)
            .and_then(Path::parent);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.unwrap_or(Path::new("")).join(path)
        };
        !path.is_file()
    });
    value["missing_video"] = json!(missing);
    value["missing_video_reason"] = if missing {
        json!("The take's baked video file is missing (it may have been deleted or the cache cleared).")
    } else {
        Value::Null
    };
    value["next_action"] = if missing {
        json!("Update video rebuilds it (screen_record.take_refresh).")
    } else {
        Value::Null
    };
}

/// Map the visible intersection through the actual EDL expansion (including
/// frame-grid speed-ramp slices). Frozen pictures have no moving zoom lane.
fn timeline_range(edl: &cut_core::Edl, clip: &str, start: u64, end: u64) -> Value {
    let mut low = u64::MAX;
    let mut high = 0;
    for s in &edl.segments {
        if s.clip_id.as_deref() != Some(clip) || s.freeze.is_some() {
            continue;
        }
        let (Some(a), Some(b)) = (s.src_in_ms, s.src_out_ms) else {
            continue;
        };
        let left = start.max(a);
        let right = end.min(b);
        if right <= left || b <= a {
            continue;
        }
        let map = |t: u64| {
            let offset = if s.reverse { b - t } else { t - a };
            s.timeline_in_ms
                + ((offset as f64 / (b - a) as f64) * (s.timeline_out_ms - s.timeline_in_ms) as f64)
                    .round() as u64
        };
        let (x, y) = (map(left), map(right));
        low = low.min(x.min(y));
        high = high.max(x.max(y));
    }
    if low == u64::MAX {
        Value::Null
    } else {
        json!({"start_ms":low,"end_ms":high})
    }
}

pub(crate) fn annotate(project: &cut_core::Project, value: &mut Value) {
    if let Some(tracks) = value["tracks"].as_array_mut() {
        for track in tracks {
            if let Some(clips) = track["clips"].as_array_mut() {
                for clip in clips {
                    if let Some(take) = clip["asset"].as_str().and_then(|asset| {
                        project.recordings.iter().find(|t| {
                            t.camera_look.is_some() && t.camera_asset.as_deref() == Some(asset)
                        })
                    }) {
                        clip["recording"] = json!({"take":take.id,"camera":true});
                        continue;
                    }
                    if let Some(take) = clip["asset"]
                        .as_str()
                        .and_then(|asset| project.recordings.iter().find(|t| t.asset == asset))
                    {
                        let mut recording = json!({
                            "take": take.id, "stale": take.stale(),
                            "zoom_count": take.segments.len(), "camera_clip": take.camera_clip
                        });
                        video_status(project, take, &mut recording);
                        clip["recording"] = recording;
                    }
                }
            }
        }
    }
}

pub(crate) async fn link_polished(
    state: &AppState,
    dir: &Path,
    source: &Path,
    plan_path: &Path,
    asset: &str,
    camera_clip: Option<String>,
    camera_asset: Option<String>,
    camera_artifact: Option<Value>,
    camera_look: Option<cut_core::recording::CameraLook>,
    actor: Actor,
) -> Result<(), CutError> {
    use sha2::{Digest, Sha256};
    let capture = source
        .parent()
        .ok_or_else(|| bad("recording capture folder is missing"))?;
    let events = capture.join("events.json");
    // Legacy/imported polished files have no sealed event journal and never
    // acquire editable provenance merely by opening an old project.
    if !events.exists() {
        return Ok(());
    }
    let events = file(dir, &events.display().to_string())?;
    let original = super::polish::load_plan(plan_path)?;
    let segments = zoom_segments::keys_to_segments(&original.zoom).map_err(record_error)?;
    let relative = capture
        .strip_prefix(dir)
        .map_err(|_| bad("capture folder is outside the project"))?;
    let id = format!(
        "take-{:x}",
        Sha256::digest(relative.to_string_lossy().as_bytes())
    );
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(crate::dispatch::no_project)?;
    if store.dir != dir {
        return Err(bad(
            "project changed before recording provenance could be linked",
        ));
    }
    if store.project.recordings.iter().any(|t| t.id == id) {
        return Ok(());
    }
    let clip_ids: Vec<_> = store
        .project
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .filter_map(|c| match c {
            cut_core::Clip::Media(m) if m.asset == asset => Some(m.id.clone()),
            _ => None,
        })
        .collect();
    let op = store.apply("screen_record.take_link",json!({"recording":{"id":id,"capture_folder":capture,"source":source,"events":events,"pending_plan":plan_path,"applied_plan":plan_path,"clip_ids":clip_ids,"asset":asset,"camera_clip":camera_clip,"camera_asset":camera_asset,"camera_artifact":camera_artifact,"camera_look":camera_look,"segments":segments}}),actor,None)?;
    state.events.publish(Event::OpApplied { op });
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    pub(crate) fn actor() -> Actor {
        serde_json::from_value(json!({"kind":"human","name":"test","via":"test"})).unwrap()
    }
    pub(crate) async fn call(state: &AppState, verb: &str, args: Value) -> Value {
        let result = crate::dispatch::dispatch(state, verb, args, actor()).await;
        assert!(result.ok, "{verb}: {:?}", result.error);
        result.result.unwrap()
    }
    pub(crate) async fn fixture() -> (tempfile::TempDir, AppState, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("take.cutproj");
        let state = AppState::new();
        call(&state, "project.create", json!({"name":"take","dir":dir})).await;
        let capture = super::super::screen_record_cache_dir(&dir)
            .unwrap()
            .join("take");
        std::fs::create_dir_all(&capture).unwrap();
        let mut plan = record_core::EditPlan::empty(320, 180, 6000, 30.0);
        plan.cursor.smoothed = vec![
            record_core::CursorSample {
                t_ms: 0,
                x: 80.0,
                y: 90.0,
            },
            record_core::CursorSample {
                t_ms: 1000,
                x: 240.0,
                y: 135.0,
            },
        ];
        let path = capture.join("plan.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
        std::fs::write(capture.join("events.json"), b"{}").unwrap();
        std::fs::write(capture.join("source.mp4"), b"source").unwrap();
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        let asset: Asset = serde_json::from_value(json!({"path":capture.join("old.mp4"),"hash":"sha256:old","probe":{"duration_ms":6000}})).unwrap();
        store
            .apply_atomic_media_insert_plan(
                &"d".repeat(64),
                vec![asset],
                vec![json!({"track":"v1","at_ms":2000,"src_range_ms":[1000,5000]})],
                actor(),
                None,
            )
            .unwrap();
        store.apply("screen_record.take_link",json!({"recording":{"id":"round2","capture_folder":capture,"source":capture.join("source.mp4"),"events":capture.join("events.json"),"pending_plan":path,"applied_plan":path,"clip_ids":["c1"],"asset":"a1","segments":[]}}),actor(),None).unwrap();
        drop(guard);
        (root, state, capture)
    }
    use cut_core::Asset;

    #[tokio::test]
    async fn pointer_reset_is_manual_and_one_operation() {
        let (_root, state, _) = fixture().await;
        let added = call(
            &state,
            "screen_record.zoom_add",
            json!({"clip":"c1","at_ms":1000,"duration_ms":1000,"cx":0.25,"cy":0.25}),
        )
        .await;
        let before = state
            .project
            .read()
            .await
            .as_ref()
            .unwrap()
            .log
            .read_all()
            .unwrap()
            .len();
        let updated = call(
            &state,
            "screen_record.zoom_update",
            json!({"clip":"c1","id":added["segment"]["id"],"target":"pointer"}),
        )
        .await;
        assert_eq!(updated["segment"]["cx"], 0.75);
        assert_eq!(updated["segment"]["cy"], 0.75);
        assert_eq!(updated["segment"]["origin"], "manual");
        assert_eq!(
            state
                .project
                .read()
                .await
                .as_ref()
                .unwrap()
                .log
                .read_all()
                .unwrap()
                .len(),
            before + 1
        );
    }

    #[tokio::test]
    async fn timeline_add_maps_speed_reverse_ramp_and_refuses_ambiguous_frozen_or_outside() {
        let (_root, state, _) = fixture().await;
        call(&state, "edit.speed", json!({"clip":"c1","factor":2})).await;
        let a = call(
            &state,
            "screen_record.zoom_add",
            json!({"clip":"c1","timeline_ms":2500,"duration_ms":500}),
        )
        .await;
        assert_eq!(a["segment"]["start_ms"], 2000);
        call(
            &state,
            "screen_record.zoom_set",
            json!({"clip":"c1","segments":[]}),
        )
        .await;
        state
            .project
            .write()
            .await
            .as_mut()
            .unwrap()
            .apply(
                "edit.reverse",
                json!({"clip":"c1","enabled":true}),
                actor(),
                None,
            )
            .unwrap();
        let a = call(
            &state,
            "screen_record.zoom_add",
            json!({"clip":"c1","timeline_ms":2500,"duration_ms":500}),
        )
        .await;
        assert_eq!(a["segment"]["start_ms"], 4000);
        call(
            &state,
            "screen_record.zoom_set",
            json!({"clip":"c1","segments":[]}),
        )
        .await;
        state
            .project
            .write()
            .await
            .as_mut()
            .unwrap()
            .apply(
                "edit.reverse",
                json!({"clip":"c1","enabled":false}),
                actor(),
                None,
            )
            .unwrap();
        call(&state, "edit.speed", json!({"clip":"c1","factor":1})).await;
        call(
            &state,
            "edit.speed_ramp",
            json!({"clip":"c1","points":[{"at_ms":0,"factor":1},{"at_ms":4000,"factor":3}]}),
        )
        .await;
        let edl = cut_core::edl_from_project(&state.project.read().await.as_ref().unwrap().project);
        let slice = edl
            .segments
            .iter()
            .filter(|s| s.clip_id.as_deref() == Some("c1"))
            .nth(5)
            .unwrap();
        let t = (slice.timeline_in_ms + slice.timeline_out_ms) / 2;
        let expected = slice.src_in_ms.unwrap()
            + (((t - slice.timeline_in_ms) as f64
                / (slice.timeline_out_ms - slice.timeline_in_ms) as f64)
                * (slice.src_out_ms.unwrap() - slice.src_in_ms.unwrap()) as f64)
                .round() as u64;
        let a = call(
            &state,
            "screen_record.zoom_add",
            json!({"clip":"c1","timeline_ms":t,"duration_ms":500}),
        )
        .await;
        assert_eq!(a["segment"]["start_ms"], expected);
        for args in [
            json!({"clip":"c1","at_ms":1000,"timeline_ms":2500}),
            json!({"clip":"c1"}),
            json!({"clip":"c1","timeline_ms":0}),
        ] {
            let result =
                crate::dispatch::dispatch(&state, "screen_record.zoom_add", args, actor()).await;
            assert!(!result.ok);
        }
        call(&state, "edit.speed_ramp", json!({"clip":"c1","points":[]})).await;
        call(&state, "edit.freeze", json!({"clip":"c1","at_ms":100})).await;
        let result = crate::dispatch::dispatch(
            &state,
            "screen_record.zoom_add",
            json!({"clip":"c1","timeline_ms":2500}),
            actor(),
        )
        .await;
        assert!(!result.ok);
        let error = result.error.unwrap();
        assert!(error.message.contains("frozen"));
        assert!(error.suggested_action.is_some());
    }
}

pub(crate) fn guard_export(project: &cut_core::Project, allow: bool) -> Result<(), CutError> {
    if !allow && project.recordings.iter().any(|take| take.stale()) {
        return Err(bad("This recording has unapplied zoom changes. Update video first (screen_record.take_refresh), or explicitly pass allow_stale_recordings:true to export the old video."));
    }
    Ok(())
}

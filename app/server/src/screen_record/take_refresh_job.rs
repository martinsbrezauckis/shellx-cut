//! Project-bound Update video: bake/probe/hash outside the project lock,
//! then atomically swap asset references. A missing bake can always be rebuilt.
use super::{
    polish_job::{self, BakeInput},
    take_edit,
};
use crate::{events::Event, state::AppState};
use cut_core::{recording::RecordingTake, Actor, Asset, CutError, Project, VerbResult};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub(crate) fn check_duration(old: u64, new: u64, fps: f32) -> Result<(), CutError> {
    if old.abs_diff(new) as f64 > 1000.0 / fps as f64 {
        return Err(take_edit::bad(
            "Update video refused: the new recording duration differs by more than one frame; no clips were changed",
        ));
    }
    Ok(())
}

struct Refresh {
    clip: String,
    dir: PathBuf,
    take: RecordingTake,
    baked: PathBuf,
    cached: bool,
    repair_needed: bool,
    old_path: Option<PathBuf>,
    old_duration: Option<u64>,
    plan_path: PathBuf,
    old_markers: Option<Vec<super::clip_markers::MarkerSeed>>,
    input: BakeInput,
}

impl Refresh {
    fn prepare(project: &Project, dir: PathBuf, clip: String) -> Result<Self, CutError> {
        let take = take_edit::take(project, &clip)?.clone();
        let source = take_edit::file(&dir, &take.source)?;
        let plan_path = take_edit::file(&dir, &take.pending_plan)?;
        let plan = super::plan_inputs::load(&dir, project, &plan_path)?;
        let plan = super::camera_look::plan_to_bake(&take, plan);
        // An earlier render the person removed in Settings, or one that was pruned,
        // has no file or asset any more (undo can bring the take back to it):
        // that is rebuilt like a missing file.
        let old = project.assets.get(&take.asset);
        let old_path = match old {
            None => None,
            Some(old) => {
                let candidate = Path::new(&old.path);
                let candidate = if candidate.is_absolute() {
                    candidate.to_owned()
                } else {
                    dir.join(candidate)
                };
                // Absence is recoverable. Existing inputs still pass the normal plain,
                // project-contained file admission before a worker can probe them.
                match std::fs::symlink_metadata(&candidate) {
                    Ok(_) => Some(take_edit::file(&dir, &old.path)?),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => return Err(error.into()),
                }
            }
        };
        let repair_needed = old_path.is_none();
        let old_duration = old
            .and_then(|a| a.probe.as_ref())
            .and_then(|p| p["duration_ms"].as_u64());
        let old_markers = old_path.as_deref().zip(old).and_then(|(path, asset)| {
            super::clip_markers::read_baked_sidecar(
                &super::strip_verbatim_prefix(path),
                &asset.hash,
            )
        });
        let source_dir = source
            .parent()
            .ok_or_else(|| take_edit::bad("source has no capture folder"))?;
        let audio = super::optional_plain_file_in_dir(
            source_dir,
            "mic.wav",
            "microphone audio",
            "repair the recording capture folder",
        )?;
        let tag = super::plan_cache_tag(&plan_path)?;
        let hash = cut_core::hash_file(&source)?;
        let name = format!(
            "{}_{tag}_{}.mp4",
            hash.replace(':', "_"),
            if audio.is_some() { "av" } else { "v" },
        );
        let baked = super::render_cleanup::destination(&dir, &take, std::ffi::OsStr::new(&name))?;
        let cached = super::cache_output::reusable_bake(&baked)?;
        let input = BakeInput {
            cached,
            raw: false,
            source,
            baked: baked.clone(),
            audio,
            plan: Some(plan),
        };
        Ok(Self {
            clip,
            dir,
            take,
            baked,
            cached,
            repair_needed,
            old_path,
            old_duration,
            plan_path,
            old_markers,
            input,
        })
    }

    fn result(&self, job: &str, asset: Option<&str>) -> Value {
        json!({
            "job_id": job, "clip": self.clip, "asset": asset,
            "baked": self.baked, "cached": self.cached,
            "applied_plan": self.take.pending_plan
        })
    }
}

fn write_refreshed_markers_sidecar(
    source_dir: &Path,
    source: &Path,
    output: &Path,
    plan_path: &Path,
    new_duration: u64,
    old_markers: Option<Vec<super::clip_markers::MarkerSeed>>,
) {
    let has_studio = source_dir
        .join(crate::screen_record_studio_journal::STUDIO_EVENTS_FILENAME)
        .exists()
        || source_dir
            .join(crate::screen_record_studio::LEGACY_STUDIO_EVENTS_FILENAME)
            .exists();
    if has_studio {
        let _ = super::clip_markers::for_polish(source_dir, source, output, plan_path, false);
        return;
    }
    if let Some(markers) = old_markers {
        let carried: Vec<_> = markers
            .into_iter()
            .filter(|marker| marker.source_ms < new_duration)
            .collect();
        let _ = super::clip_markers::write_file_sidecar(output, carried, None);
    }
}

/// All slow I/O, including hashing, finishes before the checked swap takes a
/// project write lock. Keep the initiating snapshot for the concurrent-edit gate.
async fn bake_and_probe(
    state: &AppState,
    job: &str,
    refresh: &mut Refresh,
) -> Result<Asset, CutError> {
    let plan = refresh.input.plan.as_ref().expect("prepared refresh plan");
    let fps = plan.fps;
    let plan_duration = plan.duration_ms;
    let cached = refresh.cached;
    let plan_path = refresh.plan_path.clone();
    let old_markers = refresh.old_markers.clone();
    let input = BakeInput {
        cached: refresh.cached,
        raw: false,
        source: refresh.input.source.clone(),
        baked: refresh.baked.clone(),
        audio: refresh.input.audio.clone(),
        plan: refresh.input.plan.take(),
    };
    let output = refresh.baked.clone();
    let source = input.source.clone();
    let old_path = refresh.old_path.clone();
    let old_duration = refresh.old_duration;
    let jobs = state.jobs.clone();
    let progress_id = job.to_owned();
    let work = polish_job::bake_work(input);
    state
        .jobs
        .with_limit(
            "screen_record.export",
            1,
            crate::dispatch::run_blocking_cancellable(
                "screen_record.take_refresh",
                move |cancel| {
                    let control = record_render::ffmpeg::ProcessControl::bounded(
                        Duration::from_secs(30 * 60),
                        move || cancel.is_cancelled(),
                    );
                    work(&control, &mut |done, total| {
                        jobs.progress(
                            &progress_id,
                            done as f32 / total.max(1) as f32 * 0.95,
                            Some(format!("Updating video: frame {done} of {total}")),
                        );
                    })?;
                    let probe = cut_media::probe(&output)?;
                    let new_duration = probe
                        .duration_ms
                        .ok_or_else(|| take_edit::bad("new bake duration is unknown"))?;
                    let reference = match (old_duration, old_path) {
                        (Some(duration), _) => duration,
                        (None, Some(path)) => cut_media::probe(&path)?
                            .duration_ms
                            .ok_or_else(|| take_edit::bad("old bake duration is unknown"))?,
                        (None, None) if plan_duration > 0 => plan_duration,
                        (None, None) => cut_media::probe(&source)?
                            .duration_ms
                            .ok_or_else(|| take_edit::bad("recording duration is unknown"))?,
                    };
                    check_duration(reference, new_duration, fps)?;
                    let clean_output = super::strip_verbatim_prefix(&output);
                    let hash = cut_core::hash_file(&output)?;
                    if !cached
                        || super::clip_markers::read_baked_sidecar(&clean_output, &hash).is_none()
                    {
                        write_refreshed_markers_sidecar(
                            source.parent().unwrap_or_else(|| Path::new(".")),
                            &source,
                            &output,
                            &plan_path,
                            new_duration,
                            old_markers,
                        );
                    }
                    Ok(Asset {
                        // `ProjectStore::open` canonicalizes its root. On Windows
                        // that adds a verbatim prefix, which is unsuitable for
                        // the project-visible path of a refreshed take.
                        path: clean_output.display().to_string(),
                        hash,
                        probe: Some(serde_json::to_value(probe)?),
                        transcript: None,
                        perception: None,
                        proxy: None,
                        filmstrip: None,
                    })
                },
            ),
        )
        .await
}

async fn checked_swap(
    state: &AppState,
    refresh: &Refresh,
    asset: Asset,
    actor: Actor,
) -> Result<(String, Value), CutError> {
    let _transition = state.project_transition.lock().await;
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(crate::dispatch::no_project)?;
    if store.dir != refresh.dir {
        return Err(take_edit::bad(
            "project changed while Update video was running; no clips changed",
        ));
    }
    let take = &refresh.take;
    // Remember only newly encoded, fully verified output; never adopt a cache
    // hit from a received project as a deletion capability.
    let ownership = if !refresh.cached {
        super::render_cleanup::remember(&refresh.dir, take, &refresh.baked)
    } else {
        Ok(())
    };
    let current = store
        .project
        .recordings
        .iter()
        .find(|t| t.id == take.id)
        .ok_or_else(|| take_edit::bad("recording was removed while baking"))?;
    if current.pending_plan != take.pending_plan || current.asset != take.asset {
        return Err(take_edit::bad(
            "recording changed while baking; Update video again",
        ));
    }
    let unchanged = refresh.cached && !take.stale() && !refresh.repair_needed;
    let asset_id = if unchanged {
        take.asset.clone()
    } else {
        store.next_asset_ids(1)?.remove(0)
    };
    if !unchanged {
        let args = json!({
            "take": take.id, "asset_id": asset_id,
            "asset": asset, "applied_plan": take.pending_plan
        });
        let op = store.apply("screen_record.take_refresh", args, actor, None)?;
        state.events.publish(Event::OpApplied { op });
    }
    // Update video never deletes the earlier render: Undo must bring the earlier
    // picture back at once. Older recording videos are removed only when the
    // person confirms it in Settings.
    let earlier = if ownership.is_err() {
        json!({"status":"kept", "reason":"ownership could not be recorded", "message":"Earlier video kept: ownership could not be recorded. Review old recording videos in Settings > Old recording renders."})
    } else {
        json!({"status":"kept", "reason":"kept so Undo can bring it back", "message":"Earlier video kept so Undo brings it back at once. To remove older recording videos, open Settings > Old recording renders."})
    };
    Ok((asset_id, earlier))
}

async fn run(
    state: &AppState,
    job: &str,
    mut refresh: Refresh,
    actor: Actor,
) -> Result<Value, CutError> {
    let asset = bake_and_probe(state, job, &mut refresh).await?;
    let (asset_id, earlier) = checked_swap(state, &refresh, asset, actor).await?;
    let mut result = refresh.result(job, Some(&asset_id));
    result["earlier_render"] = earlier;
    Ok(result)
}

pub(crate) async fn start(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let clip = args["clip"]
        .as_str()
        .ok_or_else(|| take_edit::bad("clip is required"))?
        .to_owned();
    let (project, _, dir, _) = crate::dispatch::snapshot(state).await?;
    let refresh = Refresh::prepare(&project, dir, clip)?;
    let job_id = state.jobs.create("screen_record_take_refresh").job_id;
    let reply = refresh.result(&job_id, None);
    let state = state.clone();
    let jobs = state.jobs.clone();
    let task_id = job_id.clone();
    jobs.clone().spawn(&job_id, async move {
        match run(&state, &task_id, refresh, actor).await {
            Ok(value) => jobs.finish(&task_id, value),
            Err(error) => jobs.fail(&task_id, error),
        }
    });
    Ok(VerbResult::ok(reply))
}

#[cfg(test)]
mod tests {
    use super::super::take_edit::tests::{call, fixture};
    use super::*;
    async fn refresh(state: &AppState) -> Value {
        let reply = call(state, "screen_record.take_refresh", json!({"clip":"c1"})).await;
        tokio::time::timeout(Duration::from_secs(300), async {
            loop {
                let job = call(state, "jobs.status", json!({"job_id":reply["job_id"]})).await;
                if job["state"] == "done" {
                    return job["result"].clone();
                }
                assert_ne!(job["state"], "failed", "{job}");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap()
    }
    #[tokio::test]
    async fn undo_to_missing_bake_reports_missing_and_refresh_repairs_trimmed_sped_clip() {
        let (_root, state, capture) = fixture().await;
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=320x180:r=30",
                "-t",
                "6",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-y",
            ])
            .arg(capture.join("source.mp4"))
            .status()
            .unwrap();
        assert!(status.success());
        call(&state, "edit.speed", json!({"clip":"c1","factor":2})).await;
        let added = call(
            &state,
            "screen_record.zoom_add",
            json!({"clip":"c1","at_ms":1000,"duration_ms":1000}),
        )
        .await;
        let first = refresh(&state).await;
        assert!(
            first["earlier_render"]["message"]
                .as_str()
                .is_some_and(|message| message.starts_with("Earlier video kept")),
            "{first}"
        );
        let first_asset = first["asset"].as_str().unwrap().to_string();
        let first_path = first["baked"].as_str().unwrap().to_string();
        let mut latest = first_asset.clone();
        for scale in [3, 4] {
            call(
                &state,
                "screen_record.zoom_update",
                json!({"clip":"c1","id":added["segment"]["id"],"scale":scale}),
            )
            .await;
            let result = refresh(&state).await;
            assert_eq!(result["earlier_render"]["status"], "kept", "{result}");
            assert!(result["earlier_render"]["message"]
                .as_str()
                .unwrap()
                .contains("Settings"));
            latest = result["asset"].as_str().unwrap().to_string();
        }
        // Update video removes nothing by itself: all three renders stay.
        assert_eq!(
            std::fs::read_dir(Path::new(&first_path).parent().unwrap())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("mp4"))
                .count(),
            3
        );
        assert!(Path::new(&first_path).exists());
        // The person removes the unused older videos in Settings.
        let preview = call(&state, "screen_record.render_cache_preview", json!({})).await;
        assert_eq!(preview["files"].as_array().unwrap().len(), 2, "{preview}");
        assert!(preview["files"][0]["bytes"].as_u64().unwrap() > 0);
        let removed = call(
            &state,
            "screen_record.render_cache_remove",
            json!({"plan_id":preview["plan_id"],"confirm":true}),
        )
        .await;
        assert_eq!(removed["results"][0]["status"], "removed", "{removed}");
        assert!(!Path::new(&first_path).exists());
        {
            let guard = state.project.read().await;
            let ids: Vec<_> = guard
                .as_ref()
                .unwrap()
                .project
                .assets
                .keys()
                .cloned()
                .collect();
            assert!(!ids.contains(&first_asset), "{ids:?}");
            assert!(ids.contains(&latest), "{ids:?}");
        }
        assert_ne!(latest, first_asset);
        // Each update consists of a zoom SET and a refresh. Undo both pairs
        // to reach the first applied plan, not an earlier asset with pending edits.
        // Two more undos first take back the two Settings removals (assets return).
        for step in 0..8 {
            let reached = state
                .project
                .read()
                .await
                .as_ref()
                .unwrap()
                .project
                .recordings[0]
                .asset
                == first_asset;
            if reached && step >= 4 {
                break;
            }
            call(&state, "project.undo", json!({})).await;
        }
        assert_eq!(
            state
                .project
                .read()
                .await
                .as_ref()
                .unwrap()
                .project
                .recordings[0]
                .asset,
            first_asset
        );
        let list = call(&state, "screen_record.zoom_list", json!({"clip":"c1"})).await;
        assert_eq!(list["missing_video"], true);
        assert!(list["next_action"]
            .as_str()
            .unwrap()
            .contains("Update video rebuilds it"));
        let snapshot = call(&state, "project.state", json!({})).await;
        let recording = &snapshot["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == "c1")
            .unwrap()["recording"];
        assert_eq!(recording["missing_video"], true);
        let before = state
            .project
            .read()
            .await
            .as_ref()
            .unwrap()
            .project
            .tracks
            .clone();
        let repaired = refresh(&state).await;
        let path = repaired["baked"].as_str().unwrap();
        assert_eq!(path, first_path);
        assert!(
            cut_media::probe(Path::new(path))
                .unwrap()
                .duration_ms
                .unwrap()
                > 0
        );
        let guard = state.project.read().await;
        let project = &guard.as_ref().unwrap().project;
        let mut expected = before;
        for track in &mut expected {
            for clip in &mut track.clips {
                if let cut_core::Clip::Media(m) = clip {
                    m.asset = project.recordings[0].asset.clone();
                }
            }
        }
        assert_eq!(project.tracks, expected);
        drop(guard);
        assert_eq!(
            call(&state, "screen_record.zoom_list", json!({"clip":"c1"})).await["missing_video"],
            false
        );
        let mut guard = state.project.write().await;
        let project = &mut guard.as_mut().unwrap().project;
        let asset = project
            .assets
            .get_mut(&project.recordings[0].asset)
            .unwrap();
        asset.probe = None;
        std::fs::remove_file(&asset.path).unwrap();
        drop(guard);
        refresh(&state).await;
    }

    #[tokio::test]
    async fn undo_right_after_update_video_plays_the_earlier_render_and_redo_the_new_one() {
        let (_root, state, capture) = fixture().await;
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=320x180:r=30",
                "-t",
                "6",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-y",
            ])
            .arg(capture.join("source.mp4"))
            .status()
            .unwrap();
        assert!(status.success());
        let added = call(
            &state,
            "screen_record.zoom_add",
            json!({"clip":"c1","at_ms":1000,"duration_ms":1000}),
        )
        .await;
        let first = refresh(&state).await;
        let first_asset = first["asset"].as_str().unwrap().to_string();
        call(
            &state,
            "screen_record.zoom_update",
            json!({"clip":"c1","id":added["segment"]["id"],"scale":3}),
        )
        .await;
        let second = refresh(&state).await;
        let second_asset = second["asset"].as_str().unwrap().to_string();
        assert_ne!(first_asset, second_asset);
        assert_eq!(second["earlier_render"]["status"], "kept", "{second}");
        async fn clip_plays(state: &AppState, asset_id: &str) {
            {
                let guard = state.project.read().await;
                let project = &guard.as_ref().unwrap().project;
                let asset = project
                    .assets
                    .get(asset_id)
                    .expect("asset stays registered");
                assert!(Path::new(&asset.path).is_file(), "{}", asset.path);
                assert_eq!(project.recordings[0].asset, asset_id);
                let (track, index) = project.find_clip("c1").unwrap();
                let cut_core::Clip::Media(m) = &project.track(track).unwrap().clips[index] else {
                    panic!("media clip")
                };
                assert_eq!(m.asset, asset_id);
            }
            let list = call(state, "screen_record.zoom_list", json!({"clip":"c1"})).await;
            assert_eq!(list["missing_video"], false, "{list}");
        }
        clip_plays(&state, &second_asset).await;
        // Undo the refresh only (the last journal entry).
        call(&state, "project.undo", json!({})).await;
        clip_plays(&state, &first_asset).await;
        call(&state, "project.redo", json!({})).await;
        clip_plays(&state, &second_asset).await;
    }

    #[tokio::test]
    async fn settings_removal_drops_the_unused_asset_so_no_source_is_missing_and_undo_shows_missing_video(
    ) {
        let (_root, state, capture) = fixture().await;
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=320x180:r=30",
                "-t",
                "6",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-y",
            ])
            .arg(capture.join("source.mp4"))
            .status()
            .unwrap();
        assert!(status.success());
        let added = call(
            &state,
            "screen_record.zoom_add",
            json!({"clip":"c1","at_ms":1000,"duration_ms":1000}),
        )
        .await;
        let before = call(&state, "project.health", json!({"limit": 128})).await;
        assert_eq!(before["media"]["page"]["offline"], 1, "{before}");
        assert_eq!(before["media"]["assets"][0]["asset"], "a1");
        let first = refresh(&state).await;
        let first_asset = first["asset"].as_str().unwrap().to_string();
        let first_path = first["baked"].as_str().unwrap().to_string();
        call(
            &state,
            "screen_record.zoom_update",
            json!({"clip":"c1","id":added["segment"]["id"],"scale":3}),
        )
        .await;
        let second = refresh(&state).await;
        let second_asset = second["asset"].as_str().unwrap().to_string();
        let preview = call(&state, "screen_record.render_cache_preview", json!({})).await;
        assert_eq!(preview["files"].as_array().unwrap().len(), 1, "{preview}");
        assert!(preview["files"][0]["bytes"].as_u64().unwrap() > 0);
        let removed = call(
            &state,
            "screen_record.render_cache_remove",
            json!({"plan_id":preview["plan_id"],"confirm":true}),
        )
        .await;
        assert_eq!(removed["results"][0]["status"], "removed", "{removed}");
        assert!(!Path::new(&first_path).exists());
        {
            let guard = state.project.read().await;
            let project = &guard.as_ref().unwrap().project;
            assert!(!project.assets.contains_key(&first_asset));
            assert!(project.assets.contains_key(&second_asset));
            assert!(project.assets.values().all(|a| !a
                .path
                .ends_with(first_path.rsplit(['/', '\\']).next().unwrap())));
        }
        let health = call(&state, "project.health", json!({"limit": 128})).await;
        // Only the fixture's own source asset a1 is offline (it was before any
        // Update video); no render asset is.
        let offline: Vec<_> = health["media"]["assets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["source"] == "offline")
            .map(|a| a["asset"].clone())
            .collect();
        assert_eq!(offline, vec![json!("a1")], "{health}");
        // Undo the removal and the refresh: the old asset returns, its file is gone.
        for _ in 0..2 {
            call(&state, "project.undo", json!({})).await;
        }
        let list = call(&state, "screen_record.zoom_list", json!({"clip":"c1"})).await;
        assert_eq!(list["missing_video"], true, "{list}");
        assert!(list["next_action"]
            .as_str()
            .unwrap()
            .contains("Update video rebuilds it"));
        refresh(&state).await;
        assert_eq!(
            call(&state, "screen_record.zoom_list", json!({"clip":"c1"})).await["missing_video"],
            false
        );
    }

    #[tokio::test]
    async fn refresh_carries_markers_sidecar_to_new_render_keeps_old_and_keeps_markers_on_reinsert()
    {
        let (_root, state, capture) = fixture().await;
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=320x180:r=30",
                "-t",
                "6",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-y",
            ])
            .arg(capture.join("source.mp4"))
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::write(
            capture.join("studio-events.jsonl"),
            format!(
                "{}\n",
                json!({
                    "version": 1,
                    "logical_ts": 1,
                    "event": {
                        "logical_ts": 1,
                        "t_ms": 1_500,
                        "source": "recording",
                        "kind": "marker",
                        "label": "Chapter 1"
                    }
                })
            ),
        )
        .unwrap();

        let added = call(
            &state,
            "screen_record.zoom_add",
            json!({"clip":"c1","at_ms":1000,"duration_ms":1000}),
        )
        .await;
        let first = refresh(&state).await;
        let first_path = first["baked"].as_str().unwrap().to_string();
        let first_sidecar = PathBuf::from(format!("{first_path}.clip-markers.json"));
        assert!(
            first_sidecar.is_file(),
            "first refreshed render must write .clip-markers.json sidecar"
        );

        call(
            &state,
            "screen_record.zoom_update",
            json!({"clip":"c1","id":added["segment"]["id"],"scale":3}),
        )
        .await;
        let second = refresh(&state).await;
        assert_eq!(second["earlier_render"]["status"], "kept", "{second}");
        let second_path = second["baked"].as_str().unwrap().to_string();
        let second_sidecar = PathBuf::from(format!("{second_path}.clip-markers.json"));
        assert!(
            first_sidecar.is_file(),
            "the kept earlier render keeps its .clip-markers.json"
        );
        assert!(
            second_sidecar.is_file(),
            "second refreshed render must keep .clip-markers.json sidecar"
        );

        // Even when studio-events.jsonl is absent later, the sidecar carries forward from the previous render.
        std::fs::remove_file(capture.join("studio-events.jsonl")).unwrap();
        call(
            &state,
            "screen_record.zoom_update",
            json!({"clip":"c1","id":added["segment"]["id"],"scale":4}),
        )
        .await;
        let third = refresh(&state).await;
        assert_eq!(third["earlier_render"]["status"], "kept", "{third}");
        let third_asset = third["asset"].as_str().unwrap().to_string();
        let third_path = third["baked"].as_str().unwrap().to_string();
        let third_sidecar = PathBuf::from(format!("{third_path}.clip-markers.json"));
        assert!(
            second_sidecar.is_file(),
            "second render's .clip-markers.json stays with its kept render"
        );
        assert!(
            third_sidecar.is_file(),
            "third refreshed render must carry .clip-markers.json forward from previous render"
        );

        let inserted = call(
            &state,
            "edit.insert",
            json!({"asset": third_asset, "track": "v1", "at_ms": 10_000}),
        )
        .await;
        let inserted_id = inserted["clip_id"].as_str().unwrap().to_string();
        let guard = state.project.read().await;
        let project = &guard.as_ref().unwrap().project;
        let (tr, idx) = project.find_clip(&inserted_id).unwrap();
        let cut_core::Clip::Media(clip) = &project.track(tr).unwrap().clips[idx] else {
            panic!("expected media clip");
        };
        assert!(
            clip.markers.iter().any(|m| m.label == "Chapter 1"),
            "reinserting refreshed take asset must retain clip markers: {:?}",
            clip.markers
        );
    }
}

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
    input: BakeInput,
}

impl Refresh {
    fn prepare(project: &Project, dir: PathBuf, clip: String) -> Result<Self, CutError> {
        let take = take_edit::take(project, &clip)?.clone();
        let source = take_edit::file(&dir, &take.source)?;
        let plan_path = take_edit::file(&dir, &take.pending_plan)?;
        let plan = super::plan_inputs::load(&dir, project, &plan_path)?;
        let plan = super::camera_look::plan_to_bake(&take, plan);
        let old = project
            .assets
            .get(&take.asset)
            .ok_or_else(|| take_edit::bad("recording asset is missing"))?;
        let candidate = Path::new(&old.path);
        let candidate = if candidate.is_absolute() {
            candidate.to_owned()
        } else {
            dir.join(candidate)
        };
        // Absence is recoverable. Existing inputs still pass the normal plain,
        // project-contained file admission before a worker can probe them.
        let old_path = match std::fs::symlink_metadata(&candidate) {
            Ok(_) => Some(take_edit::file(&dir, &old.path)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let repair_needed = old_path.is_none();
        let old_duration = old.probe.as_ref().and_then(|p| p["duration_ms"].as_u64());
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
        let baked = super::screen_record_cache_dir(&dir)?.join(format!(
            "{}_{tag}_{}.mp4",
            hash.replace(':', "_"),
            if audio.is_some() { "av" } else { "v" },
        ));
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
                    Ok(Asset {
                        // `ProjectStore::open` canonicalizes its root. On Windows
                        // that adds a verbatim prefix, which is unsuitable for
                        // the project-visible path of a refreshed take.
                        path: super::strip_verbatim_prefix(&output).display().to_string(),
                        hash: cut_core::hash_file(&output)?,
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
) -> Result<String, CutError> {
    let _transition = state.project_transition.lock().await;
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(crate::dispatch::no_project)?;
    if store.dir != refresh.dir {
        return Err(take_edit::bad(
            "project changed while Update video was running; no clips changed",
        ));
    }
    let take = &refresh.take;
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
    Ok(asset_id)
}

async fn run(
    state: &AppState,
    job: &str,
    mut refresh: Refresh,
    actor: Actor,
) -> Result<Value, CutError> {
    let asset = bake_and_probe(state, job, &mut refresh).await?;
    let asset_id = checked_swap(state, &refresh, asset, actor).await?;
    Ok(refresh.result(job, Some(&asset_id)))
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
        tokio::time::timeout(Duration::from_secs(60), async {
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
        let first_asset = first["asset"].as_str().unwrap().to_string();
        let first_path = first["baked"].as_str().unwrap().to_string();
        for scale in [3, 4] {
            call(
                &state,
                "screen_record.zoom_update",
                json!({"clip":"c1","id":added["segment"]["id"],"scale":scale}),
            )
            .await;
            refresh(&state).await;
        }
        assert!(Path::new(&first_path).exists());
        std::fs::remove_file(&first_path).unwrap();
        // Each update consists of a zoom SET and a refresh. Undo both pairs
        // to reach the first applied plan, not an earlier asset with pending edits.
        for _ in 0..4 {
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
}

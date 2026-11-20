//! `screen_record.recover{capture_id}`: turn an interrupted recording into a
//! clip on the open project's timeline.
//!
//! The recording's own folder is the only input:
//! - video: the normal `source.mp4` of a recording that finished, or the
//!   `recovered.mp4` that recovery stitches from the checkpoints the recorder
//!   closes every 15 seconds (an interrupted capture without a receipt is
//!   sealed here first, exactly like the scan at project open);
//! - sound: `mic.wav` / `system.wav`, or a playable copy of the unfinished
//!   `.mic-wav-*.part` / `.system-wav-*.part` files (see `audio_parts`).
//!
//! Video and sound are joined into `recovered-take.mp4` beside them, imported,
//! and placed after the last clip so nothing already on the timeline moves.
//! When there is sound but no video, the sound is placed on its own track.
//! Nothing in the recording folder is deleted.

use std::path::{Path, PathBuf};

use cut_core::{error_codes, Actor, CutError, VerbResult};
use record_recovery::{
    read_manifest, recover_interrupted, ManifestError, OwnerState, RecoveryState,
};
use serde_json::{json, Value};

use super::recovery::private_output::{publish, RunDir};
use super::recovery::{audio_parts, owner::effective_owner, validate_capture_id};
use crate::dispatch::{dispatch, parse_args, snapshot};
use crate::state::AppState;
use cut_core::store::{plain_file_state, PlainFileState};

const TAKE_FILE: &str = "recovered-take.mp4";
/// Written after a recovered take reached the timeline, so the capture is no
/// longer offered for recovery.
pub(crate) const ADDED_MARKER: &str = "recovered-to-timeline.json";
const RECOVERED_AUDIO_TRACK: &str = "a_recovered";
/// System sound recovered without video goes on its own track, beside the
/// microphone, so both start at the same moment as they were recorded.
const RECOVERED_SYSTEM_TRACK: &str = "a_recovered_system";

struct Media {
    video: Option<PathBuf>,
    mic: Option<PathBuf>,
    system: Option<PathBuf>,
    /// Inputs left out because they were links or not ordinary files.
    skipped: Vec<String>,
}

fn refusal(code: &str, message: &str, cause: String, action: &str) -> CutError {
    CutError::new(code, message, cause).with_suggested_action(action)
}

fn recovery_refusal(error: ManifestError) -> CutError {
    match error {
        ManifestError::StitchLimit(reason) => refusal(
            error_codes::IO,
            "this recording cannot be joined within the safety limit",
            reason.into(),
            "keep the original capture folder and contact support for recovery",
        ),
        error => refusal(
            error_codes::IO,
            "the saved parts of this recording could not be joined",
            error.to_string(),
            "the files stay in files_folder; try again, or dismiss the notice",
        ),
    }
}

/// Seal the capture when no receipt exists yet and its owner is gone.
fn ensure_sealed(capture_dir: &Path, capture_id: &str) -> Result<(), CutError> {
    let manifest = read_manifest(capture_dir).map_err(|error| {
        refusal(
            error_codes::IO,
            "this recording's journal cannot be read",
            error.to_string(),
            "the files stay in files_folder; recover them by hand or dismiss the notice",
        )
    })?;
    if manifest.receipt.is_some() {
        return Ok(());
    }
    match effective_owner(&manifest.start, capture_id) {
        OwnerState::Dead => {}
        OwnerState::Alive | OwnerState::Ambiguous => {
            return Err(refusal(
                error_codes::CONFLICT,
                "another running ShellX Cut still owns this recording",
                format!(
                    "process {} that recorded it is still running or cannot be checked",
                    manifest.start.owner_pid
                ),
                "close the other ShellX Cut window, then recover again",
            ))
        }
    }
    let ffmpeg = cut_media::toolpath::ffmpeg();
    let ffprobe = cut_media::toolpath::ffprobe();
    recover_interrupted(
        capture_dir,
        &ffmpeg.to_string_lossy(),
        &ffprobe.to_string_lossy(),
        OwnerState::Dead,
    )
    .map_err(recovery_refusal)?;
    Ok(())
}

/// A finished sound file, or a playable copy of the unfinished one. Every
/// input must be an ordinary single-name file: a planted link is skipped (and
/// reported), never read into the project.
fn sound(
    capture_dir: &Path,
    finished: &str,
    prefix: &str,
    copy: &str,
    skipped: &mut Vec<String>,
) -> Option<PathBuf> {
    let finished_path = capture_dir.join(finished);
    match plain_file_state(&finished_path) {
        PlainFileState::Plain => return Some(finished_path),
        PlainFileState::NotPlain => {
            skipped.push(format!(
                "{finished} is a link or not an ordinary file; it was not used"
            ));
            return None;
        }
        PlainFileState::Missing => {}
    }
    let part = audio_parts::find_part(capture_dir, prefix)?;
    let copy = capture_dir.join(copy);
    match audio_parts::repair_wav_copy(&part, &copy) {
        Ok(audio_parts::Repair::Saved(_frames)) => Some(copy),
        Ok(audio_parts::Repair::Empty) => None,
        Ok(audio_parts::Repair::Refused(reason)) => {
            skipped.push(reason);
            None
        }
        Err(error) => {
            skipped.push(format!("unfinished sound could not be copied: {error}"));
            None
        }
    }
}

fn collect(capture_dir: &Path) -> Result<Media, CutError> {
    let receipt = read_manifest(capture_dir)
        .ok()
        .and_then(|manifest| manifest.receipt);
    let video = receipt
        .as_ref()
        .filter(|receipt| {
            matches!(
                receipt.state,
                RecoveryState::Complete | RecoveryState::Recovered | RecoveryState::Quarantined
            )
        })
        .and_then(|receipt| receipt.source.as_deref())
        .map(|name| capture_dir.join(name));
    let video = match video {
        Some(path) => match plain_file_state(&path) {
            PlainFileState::Plain => Some(path),
            PlainFileState::Missing => None,
            PlainFileState::NotPlain => {
                return Err(refusal(
                    error_codes::CONFLICT,
                    "recovery refused: the saved video in this recording folder is a link or not an ordinary file",
                    format!(
                        "{} was left untouched and nothing outside the project was read",
                        path.display()
                    ),
                    "delete that file from the recording folder, then recover again",
                ))
            }
        },
        None => None,
    };
    let mut skipped = Vec::new();
    let mic = sound(
        capture_dir,
        "mic.wav",
        "mic-wav",
        "mic.recovered.wav",
        &mut skipped,
    );
    let system = sound(
        capture_dir,
        "system.wav",
        "system-wav",
        "system.recovered.wav",
        &mut skipped,
    );
    Ok(Media {
        video,
        mic,
        system,
        skipped,
    })
}

async fn import_and_wait(state: &AppState, actor: &Actor, path: &Path) -> Result<String, CutError> {
    let imported = Box::pin(dispatch(
        state,
        "media.import",
        json!({"path": path.display().to_string(), "rationale": "auto: screen_record.recover"}),
        actor.clone(),
    ))
    .await;
    if let Some(error) = imported.error.clone() {
        return Err(error);
    }
    let result = imported.result.unwrap_or(Value::Null);
    let asset_id = result["asset_id"].as_str().unwrap_or_default().to_string();
    if let Some(job) = result["job_id"].as_str() {
        for _ in 0..200 {
            let status = Box::pin(dispatch(
                state,
                "jobs.status",
                json!({"job_id": job}),
                actor.clone(),
            ))
            .await;
            let job_state = status
                .result
                .as_ref()
                .and_then(|value| value["state"].as_str())
                .unwrap_or("");
            if job_state == "done" || job_state == "failed" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
    }
    Ok(asset_id)
}

/// Where the timeline ends now: recovered media starts here, so nothing
/// already on the timeline moves.
async fn timeline_end(state: &AppState) -> u64 {
    let guard = state.project.read().await;
    guard
        .as_ref()
        .map(|store| store.project.duration_ms())
        .unwrap_or(0)
}

/// Place `asset_id` at `at_ms` on `track` (or adopt the clip the import placed
/// on an empty timeline, which starts at the same moment). Returns the clip id.
/// Every file of one recovered take is placed at the same `at_ms`, each on its
/// own track, so sounds recorded together play together.
async fn place_at(
    state: &AppState,
    actor: &Actor,
    asset_id: &str,
    track: &str,
    kind: &str,
    at_ms: u64,
    src_range_ms: Option<[u64; 2]>,
) -> Result<Option<String>, CutError> {
    let (placed, track_exists) = {
        let guard = state.project.read().await;
        let Some(store) = guard.as_ref() else {
            return Err(CutError::new(
                error_codes::NO_PROJECT,
                "no project is open",
                "the project closed while the recording was being recovered",
            ));
        };
        let placed = store
            .project
            .tracks
            .iter()
            .flat_map(|track| track.clips.iter())
            .find_map(|clip| match clip {
                cut_core::Clip::Media(media) if media.asset == asset_id => Some(media.id.clone()),
                _ => None,
            });
        (
            placed,
            store
                .project
                .tracks
                .iter()
                .any(|existing| existing.id == track),
        )
    };
    if placed.is_some() {
        return Ok(placed);
    }
    if !track_exists {
        let added = Box::pin(dispatch(
            state,
            "edit.add_track",
            json!({"kind": kind, "id": track, "rationale": "auto: screen_record.recover track"}),
            actor.clone(),
        ))
        .await;
        if let Some(error) = added.error {
            return Err(error);
        }
    }
    let mut insert = json!({
        "asset": asset_id,
        "track": track,
        "at_ms": at_ms,
        "ripple": false,
        "rationale": "auto: screen_record.recover",
    });
    if let Some(range) = src_range_ms {
        insert["src_range_ms"] = json!(range);
    }
    let inserted = Box::pin(dispatch(state, "edit.insert", insert, actor.clone())).await;
    if let Some(error) = inserted.error {
        return Err(error);
    }
    Ok(inserted
        .result
        .as_ref()
        .and_then(|value| value["clip_id"].as_str())
        .map(String::from))
}

/// The probed length of an imported asset.
async fn asset_duration_ms(state: &AppState, asset_id: &str) -> Option<u64> {
    let guard = state.project.read().await;
    guard
        .as_ref()?
        .project
        .assets
        .get(asset_id)?
        .probe
        .as_ref()?
        .get("duration_ms")?
        .as_u64()
}

/// The sound recorded after the saved picture of an interrupted take: the
/// joined take ends with its picture, so this sound would be lost. Each file
/// gives `(file, track, timeline offset, source range)` to place right after
/// the picture, from the same files, at the moment it was recorded.
fn sound_tails(
    capture_dir: &Path,
    media: &Media,
    picture_ms: u64,
) -> Vec<(PathBuf, &'static str, u64, [u64; 2])> {
    // System sound starts this long after the picture's first frame.
    let system_offset = super::system_audio::read_timing(capture_dir)
        .ok()
        .flatten()
        .and_then(|timing| timing.first_packet_offset_ms)
        .unwrap_or(0);
    [
        (media.mic.as_ref(), 0, RECOVERED_AUDIO_TRACK),
        (media.system.as_ref(), system_offset, RECOVERED_SYSTEM_TRACK),
    ]
    .into_iter()
    .filter_map(|(path, starts_at, track)| {
        let path = path?;
        let length = super::recovered_extent::sound_ms(path)?;
        let from = picture_ms.saturating_sub(starts_at);
        (length >= from.saturating_add(super::recovered_extent::SOUND_SPOOL_SLACK_MS)).then(|| {
            (
                path.clone(),
                track,
                picture_ms.max(starts_at),
                [from, length],
            )
        })
    })
    .collect()
}

/// Record that this capture's take was added (best effort; a failure only
/// means the capture may be offered again).
fn mark_added(capture_dir: &Path) {
    let Ok(run) = RunDir::create(capture_dir) else {
        return;
    };
    let staged = run.file(ADDED_MARKER);
    let body = serde_json::json!({ "added_unix_ms": super::active_capture::unix_ms_now() });
    if std::fs::write(&staged, body.to_string()).is_ok() {
        let _ = publish(&staged, &capture_dir.join(ADDED_MARKER));
    }
}

pub(crate) async fn recover_handler(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        capture_id: String,
    }
    let args: Args = parse_args(args)?;
    validate_capture_id(&args.capture_id)?;
    let (_project, _edl, dir, _at) = snapshot(state).await?;
    super::recovery::background::refuse_while_preparing(&dir)?;
    if super::capture_registry::is_capture_active(&args.capture_id) {
        return Err(refusal(
            error_codes::CONFLICT,
            "this recording is still running",
            "a running recording is saved by Stop, not recovered".into(),
            "stop it with screen_record.stop",
        ));
    }
    let Some(capture_dir) = super::existing_capture_dir(&dir, &args.capture_id)? else {
        return Err(refusal(
            error_codes::NOT_FOUND,
            "this recording is not in the open project",
            format!("no recording folder {} in {}", args.capture_id, dir.display()),
            "open the recording's project (project_path in screen_record.pending) with project.open, then recover again",
        ));
    };
    let capture_dir = super::strip_verbatim_prefix(&capture_dir);
    let folder = capture_dir.display().to_string();
    ensure_sealed(&capture_dir, &args.capture_id)?;
    let media = collect(&capture_dir)?;
    let take: PathBuf;
    let mut sound_only: Vec<(PathBuf, &str)> = Vec::new();
    match &media.video {
        Some(video) => {
            if media.mic.is_none() && media.system.is_none() {
                take = video.clone();
            } else {
                let joined = capture_dir.join(TAKE_FILE);
                let offset = super::system_audio::read_timing(&capture_dir)
                    .ok()
                    .flatten()
                    .and_then(|timing| timing.first_packet_offset_ms);
                // ffmpeg writes only inside a private folder this run created;
                // the result is then renamed to its name in the recording folder.
                let run = RunDir::create(&capture_dir).map_err(|error| {
                    refusal(
                        error_codes::IO,
                        "the recording folder cannot be written",
                        error.to_string(),
                        "check that the project folder is writable, then recover again",
                    )
                })?;
                let staged = run.file(TAKE_FILE);
                let (video, mic, system) = (video.clone(), media.mic.clone(), media.system.clone());
                let out = staged.clone();
                tokio::task::spawn_blocking(move || {
                    super::mux_raw_sources(&video, mic.as_deref(), system.as_deref(), offset, &out)
                })
                .await
                .map_err(|error| {
                    CutError::new(
                        error_codes::IO,
                        "recovery worker stopped",
                        error.to_string(),
                    )
                })??;
                publish(&staged, &joined).map_err(|refused| {
                    refusal(
                        error_codes::CONFLICT,
                        "recovery refused: the recovered take could not be saved by its name",
                        refused.0,
                        "delete recovered-take.mp4 from the recording folder, then recover again",
                    )
                })?;
                take = joined;
            }
        }
        None => {
            sound_only.extend(
                media
                    .mic
                    .iter()
                    .map(|path| (path.clone(), RECOVERED_AUDIO_TRACK)),
            );
            sound_only.extend(
                media
                    .system
                    .iter()
                    .map(|path| (path.clone(), RECOVERED_SYSTEM_TRACK)),
            );
            let Some((first, _)) = sound_only.first().cloned() else {
                return Err(refusal(
                    error_codes::NOT_FOUND,
                    "nothing can be recovered from this recording",
                    "it stopped before the first part of the video was saved (parts are saved every 15 seconds) and no sound was kept".into(),
                    "dismiss the notice with screen_record.pending_dismiss; the leftover files stay in files_folder",
                ));
            };
            take = first;
        }
    }

    let checkpoint = Box::pin(dispatch(
        state,
        "project.checkpoint",
        json!({"name": "before-screen-record-recover", "rationale": "auto: before screen_record.recover"}),
        actor.clone(),
    ))
    .await;
    if let Some(error) = checkpoint.error {
        return Err(error);
    }
    let mut clips = Vec::new();
    let start_ms = timeline_end(state).await;
    let mut sound_placed = false;
    if media.video.is_some() {
        let asset = import_and_wait(state, &actor, &take).await?;
        clips.push(place_at(state, &actor, &asset, "v1", "video", start_ms, None).await?);
        if let Some(picture) = asset_duration_ms(state, &asset).await {
            let tails = sound_tails(&capture_dir, &media, picture);
            for (path, track, at, range) in tails {
                let sound = import_and_wait(state, &actor, &path).await?;
                clips.push(
                    place_at(
                        state,
                        &actor,
                        &sound,
                        track,
                        "audio",
                        start_ms + at,
                        Some(range),
                    )
                    .await?,
                );
                sound_placed = true;
            }
        }
    } else {
        for (path, track) in &sound_only {
            let asset = import_and_wait(state, &actor, path).await?;
            clips.push(place_at(state, &actor, &asset, track, "audio", start_ms, None).await?);
        }
    }
    super::pending_recordings::remove(&args.capture_id);
    mark_added(&capture_dir);
    let video_recovered = media.video.is_some();
    // What was kept against how long it was recorded, in the same words on
    // every system (see recovered_extent).
    let extent = super::recovered_extent::measure(
        &capture_dir,
        video_recovered,
        &[media.mic.as_deref(), media.system.as_deref()],
    );
    let extent_sentence = extent.sentence_after_placing(sound_placed);
    let mut message = if video_recovered {
        format!("{extent_sentence} The recovered recording was added to the end of the timeline.")
    } else {
        // Sound only: "all that was recorded" would read as if nothing was
        // lost, while none of the picture was saved.
        format!(
            "Only the sound of this recording could be recovered ({} of sound): none of its picture was saved, because the recording stopped before the first part of the picture was closed (every {}). It was added to the end of the timeline.",
            super::recovered_extent::duration_words(extent.recovered_ms),
            super::recovered_extent::duration_words(super::recovery::CHECKPOINT_INTERVAL_MS),
        )
    }
    .replace("  ", " ")
    .trim()
    .to_string();
    if system_sound_requested(&capture_dir) && media.system.is_none() {
        // Row 185: say it instead of letting a silent track look complete.
        message.push_str(
            " System sound was recorded but could not be recovered: none of it had been saved to disk when the recording was interrupted.",
        );
    }
    Ok(VerbResult::ok(json!({
        "capture_id": args.capture_id,
        "clip_ids": clips.into_iter().flatten().collect::<Vec<_>>(),
        "take_path": take.display().to_string(),
        "video_recovered": video_recovered,
        "microphone_recovered": media.mic.is_some(),
        "system_audio_recovered": media.system.is_some(),
        "recovered_ms": extent.recovered_ms,
        "recorded_ms_at_least": extent.recorded_ms_at_least,
        "lost_tail_lower_bound_ms": extent.lost_tail_lower_bound_ms,
        "recorded_longer_unknown": extent.longer_unknown,
        "sound_after_picture": sound_placed,
        "files_folder": folder,
        "skipped_files": media.skipped,
        "message": message,
    })))
}

/// Whether this recording asked for system sound (its start marker says so).
fn system_sound_requested(capture_dir: &Path) -> bool {
    let marker = capture_dir.join(".capture.json");
    if !record_recovery::is_plain_regular_file(&marker).unwrap_or(false) {
        return false;
    }
    match cut_core::store::read_bounded_cache(&marker, 1024 * 1024) {
        cut_core::store::BoundedCacheRead::Bytes(bytes) => serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|marker| marker.get("system_audio").and_then(Value::as_bool))
            .unwrap_or(false),
        _ => false,
    }
}

#[cfg(test)]
#[path = "recover_take_tests.rs"]
mod tests;

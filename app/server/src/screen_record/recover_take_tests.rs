//! Recovery of interrupted recordings, with the native capture layer absent:
//! each test builds the files an interrupted capture leaves behind.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);

#[test]
fn stitch_limit_refusal_names_the_reason_and_a_recovery_step() {
    let error = recovery_refusal(ManifestError::StitchLimit("the join list is too large"));
    assert!(error.message.contains("safety limit"));
    assert!(error.cause.contains("join list is too large"));
    assert!(error
        .suggested_action
        .as_deref()
        .unwrap_or_default()
        .contains("keep the original capture folder"));
}

async fn project(name: &str) -> (tempfile::TempDir, PathBuf, AppState) {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join(format!("{name}.cutproj"));
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": name, "dir": dir}),
        Actor::system(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    (temp, dir, state)
}

/// A capture folder exactly as a killed recorder leaves it: a journal with an
/// open segment and no receipt, owned by this process but no longer active.
fn interrupted_capture(project_dir: &Path) -> (String, PathBuf) {
    let id = format!(
        "cap_{}_{}_{}",
        std::process::id(),
        super::super::active_capture::unix_ms_now(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let capture = super::super::create_capture_dir(project_dir, &id).unwrap();
    let capture = super::super::strip_verbatim_prefix(&capture);
    record_recovery::ManifestOwner::begin(
        &capture,
        record_recovery::CaptureStart::new(&id, 15_000),
    )
    .unwrap();
    (id, capture)
}

/// An unfinished 16-bit stereo 48 kHz microphone file holding `seconds` of a
/// tone, with the zero sizes a streaming writer leaves before it finishes.
fn unfinished_microphone(capture: &Path, seconds: u32) {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF\0\0\0\0WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&48_000u32.to_le_bytes());
    bytes.extend_from_slice(&192_000u32.to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data\0\0\0\0");
    for frame in 0..48_000 * seconds {
        let value =
            ((frame as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 8_000.0) as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    std::fs::write(capture.join(".mic-wav-5f5f.part"), bytes).unwrap();
}

/// The same unfinished format as `unfinished_microphone`, as the system-sound
/// writer leaves it.
fn unfinished_system_sound(capture: &Path, seconds: u32) {
    unfinished_microphone(capture, seconds);
    std::fs::rename(
        capture.join(".mic-wav-5f5f.part"),
        capture.join(".system-wav-6e6e.part"),
    )
    .unwrap();
}

/// Timeline start of a clip: the summed length of the clips before it on its track.
fn clip_start_ms(state_project: &cut_core::Project, clip_id: &str) -> Option<(String, u64)> {
    for track in &state_project.tracks {
        let mut at = 0u64;
        for clip in &track.clips {
            if clip.id() == Some(clip_id) {
                return Some((track.id.clone(), at));
            }
            at += clip.timeline_duration_ms();
        }
    }
    None
}

#[tokio::test]
async fn sound_of_a_recording_killed_before_its_first_video_part_is_recovered() {
    let (_temp, dir, state) = project("killed-early").await;
    let (id, capture) = interrupted_capture(&dir);
    unfinished_microphone(&capture, 2);
    super::super::pending_recordings::note_started(&id, &dir);

    let reply = recover_handler(&state, json!({"capture_id": id}), Actor::system())
        .await
        .unwrap();
    let result = reply.result.unwrap();
    assert_eq!(result["video_recovered"], json!(false));
    assert_eq!(result["microphone_recovered"], json!(true));
    assert_eq!(result["clip_ids"].as_array().unwrap().len(), 1);
    assert!(result["message"]
        .as_str()
        .unwrap()
        .starts_with("Only the sound of this recording could be recovered (2 s of sound): none of its picture was saved"), "{}", result["message"]);
    // The journal is sealed, the playable copy exists, the original remains.
    let status = record_recovery::recovery_status(&capture);
    assert!(status.receipt.is_some(), "the capture is sealed");
    let copy = hound::WavReader::open(capture.join("mic.recovered.wav")).unwrap();
    assert_eq!(copy.duration(), 96_000);
    assert!(capture.join(".mic-wav-5f5f.part").is_file());
    // The returned clip is on the timeline.
    let clip_id = result["clip_ids"][0].as_str().unwrap().to_string();
    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    assert!(store
        .project
        .tracks
        .iter()
        .flat_map(|track| track.clips.iter())
        .any(|clip| matches!(clip, cut_core::Clip::Media(media) if media.id == clip_id)));
    drop(guard);
    // Resolved: the next start no longer offers it.
    let pending = super::super::pending_recordings::pending_handler(json!({}))
        .await
        .unwrap()
        .result
        .unwrap();
    assert!(!pending["recordings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["capture_id"] == json!(id)));
}

/// The notice shown before the person presses recover carries the same
/// truth the recover reply does: what can be kept, against what was recorded.
#[tokio::test]
async fn the_pending_notice_says_what_can_be_recovered_and_matches_the_reply() {
    let (_temp, dir, state) = project("notice-extent").await;
    let (id, capture) = interrupted_capture(&dir);
    unfinished_microphone(&capture, 2);
    super::super::pending_recordings::note_started(&id, &dir);

    let pending = super::super::pending_recordings::pending_handler(json!({}))
        .await
        .unwrap()
        .result
        .unwrap();
    let row = pending["recordings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["capture_id"] == json!(id))
        .expect("the interrupted recording is offered")
        .clone();
    assert_eq!(row["situation"], "interrupted");
    assert!(
        row["message"]
            .as_str()
            .unwrap()
            .ends_with("About 2 s can be recovered, all that was recorded."),
        "{}",
        row["message"]
    );
    assert_eq!(row["recovered_ms"], json!(2_000));
    assert_eq!(row["recorded_ms_at_least"], json!(2_000));
    assert_eq!(row["recorded_longer_unknown"], json!(false));

    let reply = recover_handler(&state, json!({"capture_id": id}), Actor::system())
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(reply["recovered_ms"], row["recovered_ms"]);
    assert_eq!(reply["recorded_ms_at_least"], row["recorded_ms_at_least"]);
    assert_eq!(
        reply["recorded_longer_unknown"],
        row["recorded_longer_unknown"]
    );
}

#[tokio::test]
async fn a_recording_with_nothing_saved_says_so_and_names_no_clip() {
    let (_temp, dir, state) = project("nothing-saved").await;
    let (id, capture) = interrupted_capture(&dir);
    let error = recover_handler(&state, json!({"capture_id": id}), Actor::system())
        .await
        .unwrap_err();
    assert_eq!(error.code, error_codes::NOT_FOUND);
    assert_eq!(
        error.message,
        "nothing can be recovered from this recording"
    );
    assert!(error.cause.contains("every 15 seconds"));
    // The capture is sealed instead of staying unresolved.
    assert!(record_recovery::recovery_status(&capture).receipt.is_some());
}

#[tokio::test]
async fn a_recording_still_running_is_not_recovered() {
    let _lock = super::super::capture_registry::capture_test_lock()
        .lock()
        .await;
    let (_temp, dir, state) = project("still-running").await;
    let (id, _capture) = interrupted_capture(&dir);
    let reservation =
        super::super::capture_registry::reserve_project_transition_test_capture(&dir, id.clone());
    let error = recover_handler(&state, json!({"capture_id": id}), Actor::system())
        .await
        .unwrap_err();
    assert_eq!(error.code, error_codes::CONFLICT);
    assert_eq!(
        error.suggested_action.as_deref(),
        Some("stop it with screen_record.stop")
    );
    drop(reservation);
}

#[tokio::test]
async fn a_recording_of_another_project_names_the_project_to_open() {
    let (_temp, _dir, state) = project("other").await;
    let error = recover_handler(&state, json!({"capture_id": "cap_1_2_3"}), Actor::system())
        .await
        .unwrap_err();
    assert_eq!(error.code, error_codes::NOT_FOUND);
    assert!(error.suggested_action.unwrap().contains("project.open"));
}

/// A capture sealed as complete with a one-second `source.mp4` made by the
/// configured ffmpeg, as a recording saved at close leaves it.
fn finished_capture_with_video(project_dir: &Path) -> (String, PathBuf) {
    let (id, capture) = interrupted_capture(project_dir);
    let status = std::process::Command::new(cut_media::toolpath::ffmpeg())
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=1:size=64x64:rate=10",
        ])
        .args(["-pix_fmt", "yuv420p"])
        .arg(capture.join("source.mp4"))
        .status()
        .expect("ffmpeg runs");
    assert!(status.success());
    let mut owner = record_recovery::ManifestOwner::open(&capture).unwrap();
    owner
        .publish_receipt(record_recovery::RecoveryReceipt {
            state: RecoveryState::Complete,
            recovered_segments: 0,
            lost_tail_ms: Some(0),
            lost_tail_lower_bound_ms: 0,
            lost_tail_upper_bound_ms: Some(0),
            audio_first_packet_offset_ms: None,
            source: Some("source.mp4".into()),
            note: "test".into(),
        })
        .unwrap();
    (id, capture)
}

fn sentinel() -> (tempfile::TempDir, PathBuf, Vec<u8>) {
    let outside = tempfile::tempdir().unwrap();
    let path = outside.path().join("outside.bin");
    let bytes = b"outside the project".to_vec();
    std::fs::write(&path, &bytes).unwrap();
    (outside, path, bytes)
}

#[cfg(unix)]
#[tokio::test]
async fn a_link_planted_at_the_take_name_is_never_written_through() {
    let (_temp, dir, state) = project("take-link").await;
    let (id, capture) = finished_capture_with_video(&dir);
    unfinished_microphone(&capture, 1);
    let (_outside, outside, bytes) = sentinel();
    std::os::unix::fs::symlink(&outside, capture.join(TAKE_FILE)).unwrap();
    let error = recover_handler(&state, json!({"capture_id": id}), Actor::system())
        .await
        .unwrap_err();
    assert_eq!(error.code, error_codes::CONFLICT);
    assert!(error.cause.contains("left untouched"), "{}", error.cause);
    assert_eq!(std::fs::read(&outside).unwrap(), bytes);
    // Replace the planted link with a hard link: refused the same way.
    std::fs::remove_file(capture.join(TAKE_FILE)).unwrap();
    std::fs::hard_link(&outside, capture.join(TAKE_FILE)).unwrap();
    let error = recover_handler(&state, json!({"capture_id": id}), Actor::system())
        .await
        .unwrap_err();
    assert_eq!(error.code, error_codes::CONFLICT);
    assert_eq!(std::fs::read(&outside).unwrap(), bytes);
}

#[cfg(unix)]
#[tokio::test]
async fn a_saved_video_that_is_a_link_is_refused_without_reading_it() {
    let (_temp, dir, state) = project("video-link").await;
    let (id, capture) = finished_capture_with_video(&dir);
    let (_outside, outside, bytes) = sentinel();
    std::fs::remove_file(capture.join("source.mp4")).unwrap();
    std::os::unix::fs::symlink(&outside, capture.join("source.mp4")).unwrap();
    let error = recover_handler(&state, json!({"capture_id": id}), Actor::system())
        .await
        .unwrap_err();
    assert_eq!(error.code, error_codes::CONFLICT);
    assert!(error.message.contains("link or not an ordinary file"));
    assert_eq!(std::fs::read(&outside).unwrap(), bytes);
}

#[cfg(unix)]
#[tokio::test]
async fn a_sound_part_that_is_a_link_is_skipped_and_reported() {
    let (_temp, dir, state) = project("part-link").await;
    let (id, capture) = finished_capture_with_video(&dir);
    let (_outside, outside, bytes) = sentinel();
    std::os::unix::fs::symlink(&outside, capture.join("mic.wav")).unwrap();
    let reply = recover_handler(&state, json!({"capture_id": id}), Actor::system())
        .await
        .unwrap();
    let result = reply.result.unwrap();
    assert_eq!(result["video_recovered"], json!(true));
    assert_eq!(result["microphone_recovered"], json!(false));
    assert!(result["skipped_files"][0]
        .as_str()
        .unwrap()
        .contains("mic.wav is a link or not an ordinary file"));
    assert_eq!(std::fs::read(&outside).unwrap(), bytes);
}

#[tokio::test]
async fn a_finished_take_with_sound_is_joined_and_placed() {
    let (_temp, dir, state) = project("joined").await;
    let (id, capture) = finished_capture_with_video(&dir);
    unfinished_microphone(&capture, 1);
    let reply = recover_handler(&state, json!({"capture_id": id}), Actor::system())
        .await
        .unwrap();
    let result = reply.result.unwrap();
    assert_eq!(result["video_recovered"], json!(true));
    assert_eq!(result["microphone_recovered"], json!(true));
    assert!(capture.join(TAKE_FILE).is_file());
    assert_eq!(result["clip_ids"].as_array().unwrap().len(), 1);
    assert!(
        !super::super::pending_recordings::has_recoverable_media(&capture),
        "a recording added to the timeline is not offered again"
    );
    // The private staging folder is gone.
    assert!(std::fs::read_dir(&capture)
        .unwrap()
        .flatten()
        .all(|entry| !entry.file_name().to_string_lossy().starts_with(".recover-")));
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "needs the Windows symbolic-link privilege or Developer Mode and ffmpeg; run by the recovery-link-windows rig"]
async fn windows_symlink_recovery_take_is_never_written_through() {
    let (_temp, dir, state) = project("take-link-windows").await;
    let (id, capture) = finished_capture_with_video(&dir);
    unfinished_microphone(&capture, 1);
    let (_outside, outside, bytes) = sentinel();
    std::os::windows::fs::symlink_file(&outside, capture.join(TAKE_FILE))
        .expect("symlink privilege");
    let error = recover_handler(&state, json!({"capture_id": id}), Actor::system())
        .await
        .unwrap_err();
    assert_eq!(error.code, error_codes::CONFLICT);
    assert_eq!(std::fs::read(&outside).unwrap(), bytes);
}

#[cfg(unix)]
#[tokio::test]
async fn a_link_planted_at_the_sound_staging_name_is_never_written_through() {
    let (_temp, dir, state) = project("staging-link").await;
    let (id, capture) = interrupted_capture(&dir);
    unfinished_microphone(&capture, 1);
    let (_outside, outside, bytes) = sentinel();
    std::os::unix::fs::symlink(&outside, capture.join("mic.recovered.wav.repairing")).unwrap();
    let reply = recover_handler(&state, json!({"capture_id": id}), Actor::system()).await;
    assert!(reply.is_ok(), "{:?}", reply.err());
    assert_eq!(std::fs::read(&outside).unwrap(), bytes);
}

#[tokio::test]
async fn microphone_and_system_sound_without_video_are_placed_together() {
    let (_temp, dir, state) = project("both-sounds").await;
    let (id, capture) = interrupted_capture(&dir);
    unfinished_system_sound(&capture, 3);
    unfinished_microphone(&capture, 2);
    let reply = recover_handler(&state, json!({"capture_id": id}), Actor::system())
        .await
        .unwrap();
    let result = reply.result.unwrap();
    assert_eq!(result["video_recovered"], json!(false));
    assert_eq!(result["microphone_recovered"], json!(true));
    assert_eq!(result["system_audio_recovered"], json!(true));
    let clip_ids: Vec<String> = result["clip_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_string())
        .collect();
    assert_eq!(clip_ids.len(), 2);
    let guard = state.project.read().await;
    let project = &guard.as_ref().unwrap().project;
    let (mic_track, mic_at) = clip_start_ms(project, &clip_ids[0]).unwrap();
    let (system_track, system_at) = clip_start_ms(project, &clip_ids[1]).unwrap();
    assert_ne!(mic_track, system_track, "each sound has its own track");
    assert_eq!(
        mic_at, system_at,
        "sounds recorded together start together ({mic_track}@{mic_at} vs {system_track}@{system_at})"
    );
}

/// Row 185: a recording that asked for system sound but has none on disk says
/// so in the recovery message; one that did not ask says nothing about it.
#[tokio::test]
async fn system_sound_that_could_not_be_recovered_is_named() {
    let (_temp, dir, state) = project("no-system-sound").await;
    let (id, capture) = interrupted_capture(&dir);
    unfinished_microphone(&capture, 2);
    std::fs::write(
        capture.join(".capture.json"),
        br#"{"pid":1,"audio":true,"system_audio":true}"#,
    )
    .unwrap();
    let reply = recover_handler(&state, json!({"capture_id": id}), Actor::system())
        .await
        .unwrap();
    let result = reply.result.unwrap();
    assert_eq!(result["system_audio_recovered"], json!(false));
    assert!(
        result["message"]
            .as_str()
            .unwrap()
            .contains("System sound was recorded but could not be recovered"),
        "{result}"
    );

    let (_temp, dir, state) = project("mic-only").await;
    let (id, capture) = interrupted_capture(&dir);
    unfinished_microphone(&capture, 2);
    let reply = recover_handler(&state, json!({"capture_id": id}), Actor::system())
        .await
        .unwrap();
    assert!(!reply.result.unwrap()["message"]
        .as_str()
        .unwrap()
        .contains("System sound"));
}

/// A capture killed about 5 s in: one finished 2 s video checkpoint, the next
/// one opened at 2.4 s and never finished, and 5 s of microphone sound.
fn killed_after_one_checkpoint(project_dir: &Path) -> (String, PathBuf) {
    let (id, capture) = interrupted_capture(project_dir);
    let mut owner = record_recovery::ManifestOwner::open(&capture).unwrap();
    let staging = owner.begin_segment(0, 0).unwrap();
    let status = std::process::Command::new(cut_media::toolpath::ffmpeg())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc=duration=2:size=64x64:rate=10")
        .args(["-pix_fmt", "yuv420p", "-f", "mp4"])
        .arg(&staging)
        .status()
        .expect("ffmpeg runs");
    assert!(status.success());
    owner
        .publish(
            0,
            &staging,
            record_recovery::CheckpointFacts {
                start_ms: 0,
                end_ms: 2_000,
                event_offset_ms: 0,
                audio_offset_ms: None,
            },
            record_recovery::MediaFacts {
                duration_ms: 2_000,
                decoded_video_frames: 20,
                has_audio: false,
                width: Some(64),
                height: Some(64),
                codec_name: Some("h264".into()),
                avg_frame_rate: None,
                r_frame_rate: None,
            },
        )
        .unwrap();
    owner.begin_segment(1, 2_400).unwrap();
    drop(owner);
    unfinished_microphone(&capture, 5);
    (id, capture)
}

/// P5: a recovered take says what was recovered against what was recorded,
/// and the lower bound of the lost tail is what the files prove, not zero.
#[tokio::test]
async fn a_recovered_take_says_how_much_of_the_recording_it_kept() {
    let (_temp, dir, state) = project("kept-of-recorded").await;
    let (id, _capture) = killed_after_one_checkpoint(&dir);
    super::super::pending_recordings::note_started(&id, &dir);
    let reply = recover_handler(&state, json!({"capture_id": id}), Actor::system())
        .await
        .unwrap();
    let result = reply.result.unwrap();
    assert_eq!(result["video_recovered"], json!(true));
    assert_eq!(result["recovered_ms"], json!(2_000));
    assert_eq!(result["recorded_ms_at_least"], json!(5_000));
    assert_eq!(result["lost_tail_lower_bound_ms"], json!(3_000));
    assert_eq!(result["recorded_longer_unknown"], json!(false));
    assert_eq!(result["sound_after_picture"], json!(true));
    assert_eq!(
        result["message"],
        json!(
            "Recovered 2 s of about 5 s. The last 3 s were not saved before the recording stopped. \
             Their sound was recovered and placed after the picture. \
             The recovered recording was added to the end of the timeline."
        )
    );
}

/// A recorder killed after one saved picture part keeps sound to the end: the
/// sound recorded after the saved picture is placed after it, not cut.
#[tokio::test]
async fn a_killed_take_names_the_missing_picture_and_keeps_its_sound() {
    let (_temp, dir, state) = project("killed-later").await;
    let (id, capture) = interrupted_capture(&dir);
    let mut owner = record_recovery::ManifestOwner::open(&capture).unwrap();
    let staging = owner.begin_segment(0, 0).unwrap();
    let status = std::process::Command::new(cut_media::toolpath::ffmpeg())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc=duration=2:size=64x64:rate=10")
        .args(["-pix_fmt", "yuv420p", "-f", "mp4"])
        .arg(&staging)
        .status()
        .expect("ffmpeg runs");
    assert!(status.success());
    let media = record_recovery::verify_media(
        &cut_media::toolpath::ffmpeg().to_string_lossy(),
        &cut_media::toolpath::ffprobe().to_string_lossy(),
        &staging,
    )
    .unwrap();
    owner
        .publish(
            0,
            &staging,
            record_recovery::CheckpointFacts {
                start_ms: 0,
                end_ms: 2_000,
                event_offset_ms: 0,
                audio_offset_ms: None,
            },
            media,
        )
        .unwrap();
    // The next picture part was still open when the recorder was killed.
    let open = owner.begin_segment(1, 2_000).unwrap();
    std::fs::write(&open, b"unfinished picture part").unwrap();
    drop(owner);
    unfinished_microphone(&capture, 5);

    let reply = recover_handler(&state, json!({"capture_id": id}), Actor::system())
        .await
        .unwrap();
    let result = reply.result.unwrap();
    assert_eq!(result["video_recovered"], json!(true));
    let picture = result["recovered_ms"].as_u64().unwrap();
    assert!(picture.abs_diff(2_000) <= 100, "{result}");
    assert_eq!(result["recorded_ms_at_least"], json!(5_000));
    assert_eq!(result["lost_tail_lower_bound_ms"], json!(5_000 - picture));
    assert_eq!(result["sound_after_picture"], json!(true));
    let message = result["message"].as_str().unwrap();
    assert!(
        message.starts_with("Recovered 2 s of about 5 s. The last 3 s were not saved before the recording stopped. Their sound was recovered and placed after the picture."),
        "{message}"
    );
    // Picture with its sound, then the sound recorded after the picture.
    let clips = result["clip_ids"].as_array().unwrap();
    assert_eq!(clips.len(), 2, "{result}");
    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    let (track, at) = clip_start_ms(&store.project, clips[1].as_str().unwrap()).unwrap();
    assert_eq!(track, RECOVERED_AUDIO_TRACK);
    assert_eq!(at, picture);
    let tail = store
        .project
        .tracks
        .iter()
        .flat_map(|track| track.clips.iter())
        .find_map(|clip| match clip {
            cut_core::Clip::Media(media) if media.id == clips[1].as_str().unwrap() => {
                Some(media.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!((tail.src_in_ms, tail.src_out_ms), (picture, 5_000));
}

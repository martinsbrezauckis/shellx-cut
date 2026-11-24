//! Explicit, validated opt-in conversion from 0.6.116-0.6.118 baked recordings
//! into editable 0.6.119+ takes (`screen_record.make_editable`).

use crate::{events::Event, state::AppState};
use cut_core::{error_codes, recording::RecordingTake, Actor, CutError, TrackKind, VerbResult};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MAX_RECORDING_MS: u64 = 7 * 24 * 3_600_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MakeEditableArgs {
    clip: String,
    rationale: Option<String>,
}

struct ParsedPolishedAsset {
    src_hash: String,
    plan_tag: String,
}

fn is_lower_hex(s: &str, expected_len: usize) -> bool {
    s.len() == expected_len
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn asset_cache_tail(path: &str) -> Option<(&str, &str, &str)> {
    let mut parts = path.split(['/', '\\']).filter(|s| !s.is_empty());
    let mut ring = [""; 3];
    let mut count = 0usize;
    for part in parts.by_ref() {
        ring[0] = ring[1];
        ring[1] = ring[2];
        ring[2] = part;
        count += 1;
    }
    if count >= 3 && ring[0] == "cache" && ring[1] == "screen_record" {
        Some((ring[0], ring[1], ring[2]))
    } else {
        None
    }
}

fn is_raw_polished_filename(path: &str) -> bool {
    let Some((_, _, name)) = asset_cache_tail(path) else {
        return false;
    };
    (name.starts_with("sha256_") || name.starts_with("sha256s_"))
        && (name.ends_with("_raw_av.mp4") || name.ends_with("_raw_v.mp4"))
}

fn parse_polished_filename(name: &str) -> Option<ParsedPolishedAsset> {
    if name.ends_with("_raw_av.mp4") || name.ends_with("_raw_v.mp4") {
        return None;
    }
    let stem = name
        .strip_suffix("_av.mp4")
        .or_else(|| name.strip_suffix("_v.mp4"))?;
    let (src_part, plan_tag) = stem.rsplit_once('_')?;
    if !is_lower_hex(plan_tag, 16) {
        return None;
    }
    let (prefix, hex) = if let Some(hex) = src_part.strip_prefix("sha256_") {
        ("sha256", hex)
    } else {
        ("sha256s", src_part.strip_prefix("sha256s_")?)
    };
    if !is_lower_hex(hex, 64) {
        return None;
    }
    let src_hash = format!("{prefix}:{hex}");
    Some(ParsedPolishedAsset {
        src_hash,
        plan_tag: plan_tag.to_string(),
    })
}

fn parse_polished_asset_location(path: &str) -> Option<ParsedPolishedAsset> {
    let (_, _, filename) = asset_cache_tail(path)?;
    parse_polished_filename(filename)
}

pub(crate) fn looks_like_old_polished_asset(path: &str) -> bool {
    parse_polished_asset_location(path).is_some()
}

fn plain_refusal(
    code: &str,
    message: impl Into<String>,
    cause: impl Into<String>,
    action: &str,
) -> CutError {
    CutError::new(code, message.into(), cause.into()).with_suggested_action(action)
}

fn relative_display(project_dir: &Path, path: &Path) -> String {
    let clean_proj = super::strip_verbatim_prefix(project_dir);
    let clean_path = super::strip_verbatim_prefix(path);
    if let Ok(rel) = clean_path.strip_prefix(&clean_proj) {
        let s = rel.to_string_lossy().replace('\\', "/");
        if !s.is_empty() {
            return s;
        }
    }
    let proj_norm = clean_proj
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string();
    let path_norm = clean_path.to_string_lossy().replace('\\', "/");
    if let Some(rest) = path_norm
        .strip_prefix(&proj_norm)
        .and_then(|s| s.strip_prefix('/'))
    {
        if !rest.is_empty() {
            return rest.to_string();
        }
    }
    if let Some((_, _, name)) = asset_cache_tail(&path_norm) {
        if !name.is_empty() && !name.contains(['/', '\\', ':']) {
            return format!("cache/screen_record/{name}");
        }
    }
    path_norm
        .rsplit('/')
        .find(|s| !s.is_empty() && !s.ends_with(':'))
        .unwrap_or("recording")
        .to_string()
}

fn token_is_absolute_or_external_path(token: &str) -> bool {
    let t = token.trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | '`' | ',' | ';'));
    let bytes = t.as_bytes();
    t.starts_with('/')
        || t.starts_with("~/")
        || t.starts_with('\\')
        || t.contains('\\')
        || t.contains("=/")
        || t.contains(":/")
        || (bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic())
}

fn contains_any_path_like_token(text: &str) -> bool {
    text.split_whitespace().any(|token| {
        let t = token.trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | '`' | ',' | ';'));
        token_is_absolute_or_external_path(token) || t.contains('/')
    })
}

fn scrub_project_roots(
    text: &str,
    project_dir: &Path,
    canonical_project_dir: Option<&Path>,
) -> String {
    let mut out = text.to_string();
    let mut roots: Vec<String> = Vec::new();
    for root in std::iter::once(project_dir).chain(canonical_project_dir) {
        for candidate in [root.to_path_buf(), super::strip_verbatim_prefix(root)] {
            let raw = candidate.to_string_lossy().into_owned();
            let fwd = raw.replace('\\', "/");
            for variant in [raw, fwd] {
                let trimmed = variant.trim_end_matches(['/', '\\']).to_string();
                if !trimmed.is_empty() && !roots.contains(&trimmed) {
                    roots.push(trimmed);
                }
            }
        }
    }
    // Replace longest root spellings first so canonical/verbatim prefixes match before shorter ones.
    roots.sort_by_key(|b| std::cmp::Reverse(b.len()));
    for root in roots {
        let with_slash = format!("{root}/");
        let with_backslash = format!("{root}\\");
        out = out.replace(&with_slash, "");
        out = out.replace(&with_backslash, "");
        out = out.replace(&root, "project folder");
    }
    out.replace('\\', "/")
}

fn sanitize_error(
    project_dir: &Path,
    canonical_project_dir: Option<&Path>,
    fallback_cause: &str,
    error: CutError,
) -> CutError {
    tracing::warn!(
        code = %error.code,
        message = %error.message,
        cause = %error.cause,
        suggested_action = ?error.suggested_action,
        "screen_record.make_editable refused"
    );
    let scrubbed_msg = scrub_project_roots(&error.message, project_dir, canonical_project_dir);
    let message = if scrubbed_msg
        .split_whitespace()
        .any(token_is_absolute_or_external_path)
    {
        "Recording file could not be accessed inside cache/screen_record".to_string()
    } else {
        scrubbed_msg
    };
    let cause = if error.cause.trim().is_empty() || contains_any_path_like_token(&error.cause) {
        fallback_cause.to_string()
    } else {
        error.cause
    };
    let suggested_action = error.suggested_action.map(|action| {
        let scrubbed = scrub_project_roots(&action, project_dir, canonical_project_dir);
        if scrubbed
            .split_whitespace()
            .any(token_is_absolute_or_external_path)
        {
            "Check the recording files inside cache/screen_record, or keep using the existing video as-is.".to_string()
        } else {
            scrubbed
        }
    });
    CutError {
        code: error.code,
        message,
        clip_id: error.clip_id,
        at_ms: error.at_ms,
        cause,
        suggested_action,
        requirement: error.requirement,
    }
}

fn collect_clip_ids(project: &cut_core::Project, asset_id: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut scan_tracks = |tracks: &[cut_core::Track]| {
        for track in tracks {
            for clip in &track.clips {
                if let cut_core::Clip::Media(m) = clip {
                    if m.asset == asset_id && !ids.iter().any(|id| id == &m.id) {
                        ids.push(m.id.clone());
                    }
                }
            }
        }
    };
    scan_tracks(&project.tracks);
    for nest in &project.nests {
        scan_tracks(&nest.tracks);
    }
    for seq in &project.sequences {
        scan_tracks(&seq.tracks);
        for nest in &seq.nests {
            scan_tracks(&nest.tracks);
        }
    }
    ids
}

fn plain_components_equal(left: &Path, right: &Path) -> bool {
    // The `\\?\` form `canonicalize` gives on Windows is the same place as the
    // plain `C:\...` form a project stores, so both are compared without it.
    let left = super::strip_verbatim_prefix(left);
    let right = super::strip_verbatim_prefix(right);
    let left_s = left.to_string_lossy().replace('\\', "/");
    let right_s = right.to_string_lossy().replace('\\', "/");
    Path::new(&left_s)
        .components()
        .eq(Path::new(&right_s).components())
}

/// The folder an asset path sits in once the artifact's relative video path
/// (`camera/camera_01.mp4`) is taken off its end.
fn capture_folder_of_asset(asset: &Path, video: &str) -> Option<PathBuf> {
    let depth = Path::new(video).components().count();
    asset
        .ancestors()
        .nth(depth)
        .map(Path::to_path_buf)
        .filter(|folder| !folder.as_os_str().is_empty())
}

fn same_path_components(left: &Path, right: &Path) -> bool {
    if plain_components_equal(left, right) {
        return true;
    }
    // Short 8.3 names, different letter case on Windows and links: two spellings
    // of one existing file have the same canonical form.
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => plain_components_equal(&left, &right),
        _ => false,
    }
}

fn is_hashed_plan_name(name: &str) -> bool {
    name.strip_prefix("plan-")
        .and_then(|s| s.strip_suffix(".json"))
        .is_some_and(|hex| is_lower_hex(hex, 64))
}

struct ResolvedCaptureInputs {
    capture_dir: PathBuf,
    events_path: PathBuf,
    plan_path: PathBuf,
}

fn find_matching_capture_and_plan(
    project_dir: &Path,
    cache: &Path,
    asset_path: &str,
    parsed: &ParsedPolishedAsset,
) -> Result<ResolvedCaptureInputs, CutError> {
    let raw_asset = Path::new(asset_path);
    let candidate_baked = if raw_asset.is_absolute() {
        raw_asset.to_path_buf()
    } else {
        project_dir.join(raw_asset)
    };
    if matches!(
        std::fs::symlink_metadata(&candidate_baked),
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound
    ) {
        let rel_name = relative_display(project_dir, &candidate_baked);
        tracing::warn!(
            path = %candidate_baked.display(),
            relative = %rel_name,
            "screen_record.make_editable: missing polished video file"
        );
        return Err(plain_refusal(
            error_codes::NOT_FOUND,
            format!("The recording video file '{rel_name}' could not be found"),
            "the expected polished video file is missing from the recording cache",
            "Restore or relink the recording video file inside cache/screen_record, then try Make editable again.",
        ));
    }
    super::capture_files::plain_existing_file_under_dir(
        cache,
        &candidate_baked,
        "polished recording video",
        "restore or relink the recording video inside this project's cache/screen_record folder, then try again",
    )
    .map_err(|err| {
        sanitize_error(
            project_dir,
            None,
            "the polished recording video must stay a plain file inside the recording cache",
            err,
        )
    })?;

    let entries = std::fs::read_dir(cache).map_err(|error| {
        tracing::warn!(cache = %cache.display(), %error, "screen_record.make_editable: could not inspect cache/screen_record");
        CutError::new(
            error_codes::IO,
            "Could not inspect cache/screen_record",
            "reading the recording cache folder failed",
        )
    })?;

    let mut matching_captures = Vec::new();
    let mut matching_plans = Vec::new();
    let mut saw_hashed_plan = false;
    let mut saw_legacy_events_plan = false;
    let mut saw_unsafe_source_link = false;

    for entry in entries {
        let entry = entry.map_err(|error| {
            tracing::warn!(cache = %cache.display(), %error, "screen_record.make_editable: could not read recording cache entry");
            CutError::new(
                error_codes::IO,
                "Could not read recording cache entry",
                "inspecting a recording cache entry failed",
            )
        })?;
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            if entry.file_name() == "events.plan.json" {
                saw_legacy_events_plan = true;
            }
            continue;
        }
        let entry_path = entry.path();
        if ft.is_dir() {
            let src = entry_path.join("source.mp4");
            match std::fs::symlink_metadata(&src) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    saw_unsafe_source_link = true;
                }
                Ok(_) => {
                    let verified = super::capture_files::plain_existing_file_under_dir(
                        &entry_path,
                        &src,
                        "recording source",
                        "keep source.mp4 as a plain file inside its capture folder",
                    )
                    .map_err(|err| {
                        sanitize_error(
                            project_dir,
                            None,
                            "source.mp4 must stay a plain file inside its capture folder",
                            err,
                        )
                    })?;
                    if let Ok(hash) = cut_core::hash_file(&verified) {
                        if hash == parsed.src_hash {
                            matching_captures.push(entry_path);
                        }
                    }
                }
                Err(_) => {}
            }
        } else if ft.is_file() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if name == "events.plan.json" {
                saw_legacy_events_plan = true;
                continue;
            }
            if is_hashed_plan_name(&name) {
                saw_hashed_plan = true;
                let tag = super::polish::plan_cache_tag(&entry_path).map_err(|err| {
                    sanitize_error(
                        project_dir,
                        None,
                        "the recording edit plan file could not be read",
                        err,
                    )
                })?;
                if tag == parsed.plan_tag {
                    matching_plans.push(entry_path);
                }
            }
        }
    }

    if matching_captures.len() > 1 {
        return Err(plain_refusal(
            error_codes::CONFLICT,
            "Multiple capture folders match this recording",
            "more than one capture folder has the same source video hash",
            "Remove duplicate capture folders from cache/screen_record before making this recording editable.",
        ));
    }
    let Some(capture_dir) = matching_captures.pop() else {
        let cause = if saw_unsafe_source_link {
            "the capture folder's source.mp4 is a link outside the project"
        } else {
            "the original source.mp4 capture folder is missing from the recording cache"
        };
        return Err(plain_refusal(
            error_codes::NOT_FOUND,
            "Recording files could not be found",
            cause,
            "Restore the original capture folder into cache/screen_record, or keep using the existing video as-is.",
        ));
    };

    let events_candidate = capture_dir.join("events.json");
    if matches!(
        std::fs::symlink_metadata(&events_candidate),
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound
    ) {
        return Err(plain_refusal(
            error_codes::NOT_FOUND,
            "Recording files could not be found",
            "events.json is missing from the recording's capture folder",
            "Restore events.json from backup, or keep using the existing video as-is.",
        ));
    }
    let events_path = super::capture_files::plain_existing_file_under_dir(
        &capture_dir,
        &events_candidate,
        "recording event journal",
        "keep events.json as a plain file inside the capture folder, then retry",
    )
    .map_err(|err| {
        sanitize_error(
            project_dir,
            None,
            "events.json must remain a plain regular file inside the capture folder",
            err,
        )
    })?;

    if matching_plans.len() > 1 {
        return Err(plain_refusal(
            error_codes::CONFLICT,
            "Multiple edit plans match this recording",
            "more than one plan-*.json snapshot matches this recording's plan tag",
            "Remove duplicate plan files from cache/screen_record and retry.",
        ));
    }
    let Some(plan_path) = matching_plans.pop() else {
        if saw_legacy_events_plan || !saw_hashed_plan {
            return Err(plain_refusal(
                error_codes::INVALID_ARGS,
                "This recording was made with an older version and cannot be made editable",
                "recordings made before 0.6.116 did not keep a per-take plan snapshot (plan-*.json)",
                "Keep using this video clip as-is, or record a new take to edit its zooms and camera.",
            ));
        }
        return Err(plain_refusal(
            error_codes::NOT_FOUND,
            "Recording plan file could not be found",
            "the matching plan snapshot is missing from the recording cache",
            "Restore the missing plan file into cache/screen_record, or keep using the existing video as-is.",
        ));
    };

    Ok(ResolvedCaptureInputs {
        capture_dir,
        events_path,
        plan_path,
    })
}

struct ResolvedLegacyCamera {
    capture_folder: String,
    artifact: record_core::CameraArtifact,
    clip_id: String,
    asset_id: String,
    look: cut_core::recording::CameraLook,
    pending_plan: PathBuf,
}

fn resolve_legacy_camera(
    project_dir: &Path,
    cache: &Path,
    project: &cut_core::Project,
    capture_dir: &Path,
    plan_path: &Path,
    plan: &record_core::EditPlan,
) -> Result<(Option<ResolvedLegacyCamera>, Option<&'static str>), CutError> {
    let artifact_opt = super::capture_artifacts::camera_artifact_for_capture(capture_dir).map_err(|error| {
        let mut out = error;
        if out.suggested_action.is_none() {
            out = out.with_suggested_action(
                "Restore the intact camera recording files inside the capture folder, or keep using the existing video as-is.",
            );
        }
        out
    })?;
    let Some(validated) = artifact_opt else {
        return Ok((None, None));
    };

    let expected_hash = format!("sha256:{}", validated.artifact.media.sha256);
    let expected_abs = capture_dir.join(&validated.artifact.video);
    let capture_rel = capture_dir
        .strip_prefix(project_dir)
        .map_err(|_| super::take_edit::bad("capture folder is outside the project"))?;
    let expected_rel = capture_rel.join(&validated.artifact.video);

    let mut matched: Option<(String, String, String)> = None;
    let first_non_empty_video = project
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video && !t.clips.is_empty())
        .map(|t| t.id.as_str());

    for track in project.tracks.iter().filter(|t| t.kind == TrackKind::Video) {
        if first_non_empty_video == Some(track.id.as_str()) {
            continue;
        }
        for clip in &track.clips {
            let cut_core::Clip::Media(media) = clip else {
                continue;
            };
            let Some(asset) = project.assets.get(&media.asset) else {
                continue;
            };
            if asset.hash != expected_hash {
                continue;
            }
            let asset_p = Path::new(&asset.path);
            let folder_choice = if same_path_components(asset_p, &expected_abs) {
                // Keep the capture folder in the spelling the camera asset uses
                // (the take is later checked against that asset's path).
                Some(
                    capture_folder_of_asset(asset_p, &validated.artifact.video)
                        .unwrap_or_else(|| capture_dir.to_path_buf())
                        .to_string_lossy()
                        .into_owned(),
                )
            } else if same_path_components(asset_p, &expected_rel) {
                Some(capture_rel.to_string_lossy().into_owned())
            } else {
                None
            };
            if let Some(folder) = folder_choice {
                if !project
                    .recordings
                    .iter()
                    .any(|t| t.camera_asset.as_deref() == Some(media.asset.as_str()))
                {
                    matched = Some((folder, media.id.clone(), media.asset.clone()));
                    break;
                }
            }
        }
        if matched.is_some() {
            break;
        }
    }

    let Some((capture_folder, clip_id, asset_id)) = matched else {
        return Ok((
            None,
            Some("no camera clip on the timeline uses this recording's camera video"),
        ));
    };
    let Some(look) =
        super::camera_look::recorded(plan, validated.artifact.clock.first_frame_offset_ms)?
    else {
        return Ok((
            None,
            Some("the recording's plan holds no camera settings to restore"),
        ));
    };
    let pending_plan = super::camera_look::screen_plan(cache, plan_path, plan)?;

    Ok((
        Some(ResolvedLegacyCamera {
            capture_folder,
            artifact: validated.artifact,
            clip_id,
            asset_id,
            look,
            pending_plan,
        }),
        None,
    ))
}

pub(crate) async fn handle(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let parsed_args: MakeEditableArgs = serde_json::from_value(args)?;
    let clip_id = parsed_args.clip.trim().to_owned();
    if clip_id.is_empty() {
        return Err(plain_refusal(
            error_codes::INVALID_ARGS,
            "Clip id is required",
            "pass the recording clip id to make editable",
            "Call project.state and pass a recording clip id to screen_record.make_editable.",
        ));
    }

    let (project_dir, asset_id, asset_path, asset_hash, project_snapshot) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(crate::dispatch::no_project)?;
        let Some((track_id, index)) = store.project.find_clip(&clip_id) else {
            return Err(plain_refusal(
                error_codes::NOT_FOUND,
                format!("Clip '{clip_id}' was not found"),
                "no clip with that id exists in the open project",
                "Call project.state to inspect the project's video clips and pass a valid clip id.",
            ));
        };
        let track = store.project.track(track_id).unwrap();
        let cut_core::Clip::Media(media) = &track.clips[index] else {
            return Err(plain_refusal(
                error_codes::INVALID_ARGS,
                "Only video recording clips can be made editable",
                format!("clip '{clip_id}' is not a media clip"),
                "Select the screen recording video clip and try again.",
            ));
        };
        if let Some(existing) = cut_core::recording::take_for_clip(&store.project, &clip_id) {
            return Err(plain_refusal(
                error_codes::INVALID_ARGS,
                "This recording is already editable",
                format!(
                    "clip '{clip_id}' already belongs to editable take '{}'",
                    existing.id
                ),
                "Use the Inspector Zoom or Camera controls, or call screen_record.zoom_list.",
            ));
        }
        if store.project.recordings.iter().any(|t| {
            t.camera_clip.as_deref() == Some(clip_id.as_str())
                || t.camera_asset.as_deref() == Some(media.asset.as_str())
        }) {
            return Err(plain_refusal(
                error_codes::INVALID_ARGS,
                "This recording is already editable",
                format!("clip '{clip_id}' already belongs to an editable recording camera track"),
                "Select the Camera clip in the Inspector to adjust camera position, shape, or reset.",
            ));
        }
        if track.kind == TrackKind::Audio {
            let suggestion = store
                .project
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Video)
                .flat_map(|t| t.clips.iter())
                .find_map(|c| match c {
                    cut_core::Clip::Media(v) if v.asset == media.asset => Some(v.id.clone()),
                    _ => None,
                })
                .map(|video_clip| {
                    format!("Call screen_record.make_editable with clip:'{video_clip}' instead of audio clip '{clip_id}'.")
                })
                .unwrap_or_else(|| "Select a video recording clip and try again.".into());
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "Select the video recording clip to make it editable",
                format!("clip '{clip_id}' is an audio track clip"),
            )
            .with_suggested_action(suggestion));
        }
        if track.kind != TrackKind::Video {
            return Err(plain_refusal(
                error_codes::INVALID_ARGS,
                "Only video recording clips can be made editable",
                format!("clip '{clip_id}' is not on a video track"),
                "Select the screen recording video clip and try again.",
            ));
        }
        let asset = store.project.assets.get(&media.asset).ok_or_else(|| {
            plain_refusal(
                error_codes::NOT_FOUND,
                "Recording asset is missing",
                format!(
                    "asset '{}' referenced by clip '{clip_id}' is missing",
                    media.asset
                ),
                "Restore or relink the recording media before retrying.",
            )
        })?;
        (
            store.dir.clone(),
            media.asset.clone(),
            asset.path.clone(),
            asset.hash.clone(),
            store.project.clone(),
        )
    };

    if is_raw_polished_filename(&asset_path) {
        return Err(plain_refusal(
            error_codes::INVALID_ARGS,
            "Raw recordings do not have editable zooms",
            "this clip is a raw screen recording without a zoom/edit plan",
            "Keep using this clip as ordinary video, or record a polished take to edit zooms.",
        ));
    }
    let Some(parsed_asset) = parse_polished_asset_location(&asset_path) else {
        let display_name = relative_display(&project_dir, Path::new(&asset_path));
        tracing::warn!(
            clip_id = %clip_id,
            asset_path = %asset_path,
            "screen_record.make_editable: clip asset is not an earlier polished screen recording"
        );
        return Err(plain_refusal(
            error_codes::INVALID_ARGS,
            format!("This clip ('{display_name}') is not an earlier polished screen recording"),
            "the clip asset is not a polished recording in the recording cache",
            "Only earlier polished screen recordings (0.6.116-0.6.118) can be made editable here.",
        ));
    };

    let Some(cache) = super::containment::existing_cache_dir(&project_dir).map_err(|err| {
        sanitize_error(
            &project_dir,
            None,
            "the project's recording cache directory could not be opened safely",
            err,
        )
    })?
    else {
        return Err(plain_refusal(
            error_codes::NOT_FOUND,
            "Recording files could not be found in this project",
            "the project's recording cache directory is missing",
            "Restore the project's cache/screen_record folder from backup, or keep using the existing video as-is.",
        ));
    };

    let canonical_project_dir = project_dir.canonicalize().map_err(|error| {
        tracing::warn!(
            project_dir = %project_dir.display(),
            %error,
            "screen_record.make_editable: could not resolve project folder"
        );
        CutError::new(
            error_codes::IO,
            "Could not resolve project folder",
            "the open project directory could not be accessed",
        )
    })?;
    let resolved =
        find_matching_capture_and_plan(&canonical_project_dir, &cache, &asset_path, &parsed_asset)
            .map_err(|err| {
                sanitize_error(
                    &project_dir,
                    Some(&canonical_project_dir),
                    "the recording files could not be verified inside the recording cache",
                    err,
                )
            })?;

    let mut plan = super::polish::load_plan(&resolved.plan_path).map_err(|e| {
        let e = if e.suggested_action.is_some() {
            e
        } else {
            e.with_suggested_action(
                "Keep using the existing video as-is, or restore the recording's plan file.",
            )
        };
        sanitize_error(
            &project_dir,
            Some(&canonical_project_dir),
            "the recording edit plan file could not be read",
            e,
        )
    })?;
    let events = super::polish::load_events(&resolved.events_path)
        .map_err(|e| {
            let e = if e.suggested_action.is_some() {
                e
            } else {
                e.with_suggested_action("Restore an intact events.json file in the capture folder, or keep using the existing video as-is.")
            };
            sanitize_error(
                &project_dir,
                Some(&canonical_project_dir),
                "the recording event journal file could not be read",
                e,
            )
        })?;
    if events.duration_ms == 0
        || events.duration_ms > MAX_RECORDING_MS
        || events.screen_w == 0
        || events.screen_h == 0
    {
        return Err(plain_refusal(
            error_codes::INVALID_ARGS,
            "Recording event journal has invalid dimensions or duration",
            format!(
                "events.json reported {}x{} duration {}ms",
                events.screen_w, events.screen_h, events.duration_ms
            ),
            "Keep using the existing video as-is, or restore an undamaged events.json.",
        ));
    }
    if events.duration_ms != plan.duration_ms {
        return Err(plain_refusal(
            error_codes::INVALID_ARGS,
            "Recording files do not match each other",
            format!(
                "events.json duration ({}ms) does not match edit plan duration ({}ms)",
                events.duration_ms, plan.duration_ms
            ),
            "Restore the matching events.json and plan file, or keep using the existing video as-is.",
        ));
    }

    plan.validate()
        .map_err(|error| {
            super::take_edit::bad(&error.to_string()).with_suggested_action(
                "Keep using the existing video as-is, or restore an undamaged plan file.",
            )
        })
        .map_err(|err| {
            sanitize_error(
                &project_dir,
                Some(&canonical_project_dir),
                "the recording edit plan is invalid",
                err,
            )
        })?;
    super::camera_look::validate_camera_timeline(&plan).map_err(|err| {
        sanitize_error(
            &project_dir,
            Some(&canonical_project_dir),
            "the recording camera timeline is invalid",
            err,
        )
    })?;
    super::plan_inputs::admit(&canonical_project_dir, &project_snapshot, &mut plan).map_err(
        |err| {
            sanitize_error(
                &project_dir,
                Some(&canonical_project_dir),
                "the recording edit plan references an outside or invalid media file",
                err,
            )
        },
    )?;
    let raw_segments =
        record_core::zoom_segments::keys_to_segments(&plan.zoom).map_err(super::record_err)?;
    let segments: Vec<cut_core::recording::ZoomSegment> =
        serde_json::from_value(serde_json::to_value(&raw_segments)?)?;
    cut_core::recording::validate_segments(&segments)?;

    let (cam, camera_note) = resolve_legacy_camera(
        &canonical_project_dir,
        &cache,
        &project_snapshot,
        &resolved.capture_dir,
        &resolved.plan_path,
        &plan,
    )
    .map_err(|err| {
        sanitize_error(
            &project_dir,
            Some(&canonical_project_dir),
            "the recording camera files could not be verified",
            err,
        )
    })?;

    let capture_relative = resolved
        .capture_dir
        .strip_prefix(&canonical_project_dir)
        .map_err(|_| super::take_edit::bad("capture folder is outside the project"))?;
    let take_id = format!(
        "take-{:x}",
        Sha256::digest(capture_relative.to_string_lossy().as_bytes())
    );
    let source_path = resolved.capture_dir.join("source.mp4");
    let applied_plan = resolved.plan_path.clone();
    let pending_plan = cam
        .as_ref()
        .map(|c| c.pending_plan.clone())
        .unwrap_or_else(|| applied_plan.clone());
    let stale = applied_plan != pending_plan;

    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(crate::dispatch::no_project)?;
    let Some((track_id, idx)) = store.project.find_clip(&clip_id) else {
        return Err(plain_refusal(
            error_codes::CONFLICT,
            "Recording clip changed while preparing editable metadata",
            format!("clip '{clip_id}' is no longer present"),
            "Call project.state and retry Make editable.",
        ));
    };
    let track = store.project.track(track_id).unwrap();
    if track.kind != TrackKind::Video {
        return Err(plain_refusal(
            error_codes::CONFLICT,
            "Recording clip moved off its video track while preparing",
            format!("clip '{clip_id}' changed track kind"),
            "Select the video recording clip and retry.",
        ));
    }
    let cut_core::Clip::Media(current_media) = &track.clips[idx] else {
        return Err(plain_refusal(
            error_codes::CONFLICT,
            "Recording clip changed while preparing",
            format!("clip '{clip_id}' is no longer media"),
            "Select the video recording clip and retry.",
        ));
    };
    if cut_core::recording::take_for_clip(&store.project, &clip_id).is_some() {
        return Err(plain_refusal(
            error_codes::INVALID_ARGS,
            "This recording is already editable",
            format!("clip '{clip_id}' was already converted by another action"),
            "Use the Inspector Zoom or Camera controls, or call screen_record.zoom_list.",
        ));
    }
    let Some(current_asset) = store.project.assets.get(&current_media.asset) else {
        return Err(plain_refusal(
            error_codes::CONFLICT,
            "Recording asset changed while preparing",
            "recording asset is missing",
            "Call project.state and retry.",
        ));
    };
    if current_media.asset != asset_id
        || current_asset.path != asset_path
        || current_asset.hash != asset_hash
    {
        return Err(plain_refusal(
            error_codes::CONFLICT,
            "Recording changed while preparing editable metadata",
            "the clip's source video changed concurrently",
            "Retry Make editable on the updated clip.",
        ));
    }

    let clip_ids = collect_clip_ids(&store.project, &asset_id);
    let capture_folder_str = cam
        .as_ref()
        .map(|c| c.capture_folder.clone())
        .unwrap_or_else(|| resolved.capture_dir.to_string_lossy().into_owned());
    let camera_clip = cam.as_ref().map(|c| c.clip_id.clone());
    let camera_asset = cam.as_ref().map(|c| c.asset_id.clone());
    let camera_artifact = cam
        .as_ref()
        .map(|c| serde_json::to_value(&c.artifact))
        .transpose()?;
    let take = RecordingTake {
        id: take_id.clone(),
        capture_folder: capture_folder_str,
        source: source_path.to_string_lossy().into_owned(),
        events: resolved.events_path.to_string_lossy().into_owned(),
        pending_plan: pending_plan.to_string_lossy().into_owned(),
        applied_plan: applied_plan.to_string_lossy().into_owned(),
        clip_ids,
        asset: asset_id,
        camera_artifact,
        camera_clip: camera_clip.clone(),
        camera_asset: camera_asset.clone(),
        camera_look: cam.map(|c| c.look),
        segments: segments.clone(),
    };

    let op = store
        .apply(
            "screen_record.make_editable",
            json!({ "clip": clip_id, "recording": take }),
            actor,
            parsed_args.rationale,
        )
        .map_err(|err| {
            sanitize_error(
                &project_dir,
                Some(&canonical_project_dir),
                "could not save the editable recording operation to the project",
                err,
            )
        })?;
    state.events.publish(Event::OpApplied { op: op.clone() });

    let mut body = json!({
        "clip": clip_id,
        "take": take_id,
        "stale": stale,
        "segments": segments,
        "camera_clip": camera_clip,
        "camera_asset": camera_asset,
    });
    if let Some(reason) = camera_note {
        body["camera_note"] = json!(format!(
            "The camera could not be made editable: {reason}. The zooms are editable."
        ));
    }
    Ok(VerbResult::ok_with_ops(body, vec![op.op_id]))
}

#[cfg(test)]
mod tests {
    use super::{capture_folder_of_asset, looks_like_old_polished_asset, same_path_components};
    use std::path::Path;

    #[test]
    fn verbatim_and_plain_forms_of_one_path_are_the_same_path() {
        assert!(same_path_components(
            Path::new(r"C:\Users\dev\demo.cutproj\cache\cap_1\camera\camera_01.mp4"),
            Path::new(r"\\?\C:\Users\dev\demo.cutproj\cache\cap_1\camera\camera_01.mp4"),
        ));
        assert!(!same_path_components(
            Path::new(r"C:\Users\dev\demo.cutproj\cache\cap_1\camera\camera_01.mp4"),
            Path::new(r"\\?\C:\Users\dev\demo.cutproj\cache\cap_1\camera\camera_02.mp4"),
        ));
    }

    #[test]
    fn capture_folder_keeps_the_spelling_of_the_asset_path() {
        assert_eq!(
            capture_folder_of_asset(
                Path::new("/work/Demo.cutproj/cache/cap_1/camera/camera_01.mp4"),
                "camera/camera_01.mp4"
            ),
            Some(Path::new("/work/Demo.cutproj/cache/cap_1").to_path_buf())
        );
        assert_eq!(
            capture_folder_of_asset(Path::new("camera_01.mp4"), "camera_01.mp4"),
            None
        );
    }

    #[test]
    fn an_existing_file_matches_its_canonical_form_and_not_another_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("camera_01.mp4");
        let other = dir.path().join("camera_02.mp4");
        std::fs::write(&file, b"a").unwrap();
        std::fs::write(&other, b"b").unwrap();
        let canonical = file.canonicalize().unwrap();
        assert!(same_path_components(&file, &canonical));
        assert!(same_path_components(&canonical, &file));
        assert!(!same_path_components(&other, &canonical));
    }

    #[test]
    fn recognizes_windows_macos_linux_and_relative_polished_paths() {
        let hash = "a".repeat(64);
        let tag = "0123456789abcdef";
        assert!(looks_like_old_polished_asset(&format!(
            r"\\?\C:\Users\dev\demo.cutproj\cache\screen_record\sha256_{hash}_{tag}_v.mp4"
        )));
        assert!(looks_like_old_polished_asset(&format!(
            "/Users/person/demo.cutproj/cache/screen_record/sha256_{hash}_{tag}_av.mp4"
        )));
        assert!(looks_like_old_polished_asset(&format!(
            "/home/person/previous.cutproj/cache/screen_record/sha256s_{hash}_{tag}_v.mp4"
        )));
        assert!(looks_like_old_polished_asset(&format!(
            "cache/screen_record/sha256_{hash}_{tag}_v.mp4"
        )));
        assert!(!looks_like_old_polished_asset(&format!(
            "/work/demo.cutproj/cache/screen_record/sha256_{hash}_raw_v.mp4"
        )));
        assert!(!looks_like_old_polished_asset(
            "/work/demo.cutproj/clip.mp4"
        ));
    }
}

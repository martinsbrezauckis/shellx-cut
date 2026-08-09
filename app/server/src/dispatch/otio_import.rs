//! Bounded, previewable, atomic OpenTimelineIO import.

use super::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

const MAX_OTIO_BYTES: u64 = 16 * 1024 * 1024;
const MAX_OTIO_TRACKS: usize = 128;
const MAX_OTIO_ITEMS: usize = 50_000;

#[derive(Clone)]
struct LoadedOtio {
    path: PathBuf,
    source_hash: String,
    name: String,
    tracks: Vec<cut_export::otio::OtioTrack>,
    source_format: Option<Value>,
}

#[derive(Clone)]
struct PreparedMedia {
    path: PathBuf,
    hash: String,
    probe: cut_media::MediaProbe,
}

fn require_time(node: &Value, pointer: &str, allow_zero: bool) -> Result<(), CutError> {
    let value = node
        .pointer(&format!("{pointer}/value"))
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0);
    let rate = node
        .pointer(&format!("{pointer}/rate"))
        .and_then(Value::as_f64)
        .filter(|rate| rate.is_finite() && *rate > 0.0 && *rate <= 1000.0);
    if value.is_none() || rate.is_none() || (!allow_zero && value == Some(0.0)) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "OTIO contains an invalid time value",
            format!("{pointer} requires a finite non-negative value and rate in (0,1000]"),
        ));
    }
    Ok(())
}

fn reject_unsupported_item_state(
    node: &Value,
    location: &str,
    allow_source_range: bool,
) -> Result<(), CutError> {
    if let Some(enabled) = node.get("enabled") {
        match enabled.as_bool() {
            Some(true) => {}
            Some(false) => {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "OTIO contains a disabled timeline item",
                    format!(
                        "{location}.enabled=false cannot be represented without changing timing"
                    ),
                ))
            }
            None => {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "OTIO enabled flag is not boolean",
                    location.to_string(),
                ))
            }
        }
    }
    for field in ["effects", "markers"] {
        let Some(value) = node.get(field) else {
            continue;
        };
        let Some(values) = value.as_array() else {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("OTIO {field} field is not an array"),
                location.to_string(),
            ));
        };
        if !values.is_empty() {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("OTIO {field} are not supported by timeline import"),
                format!("{location} contains {} {field} entrie(s)", values.len()),
            ));
        }
    }
    if !allow_source_range
        && node
            .get("source_range")
            .is_some_and(|value| !value.is_null())
    {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "OTIO composition source ranges are not supported",
            format!("{location}.source_range would crop the imported composition"),
        ));
    }
    Ok(())
}

fn validate_media_reference(item: &Value, location: &str) -> Result<(), CutError> {
    let reference = if item.get("OTIO_SCHEMA").and_then(Value::as_str) == Some("Clip.2") {
        let key = item
            .get("active_media_reference_key")
            .and_then(Value::as_str)
            .unwrap_or("DEFAULT_MEDIA");
        item.get("media_references")
            .and_then(Value::as_object)
            .and_then(|references| references.get(key))
    } else {
        item.get("media_reference")
    };
    let Some(reference) = reference else {
        return Ok(());
    };
    match reference.get("OTIO_SCHEMA").and_then(Value::as_str) {
        Some("ExternalReference.1" | "MissingReference.1") | None => Ok(()),
        Some(schema) => Err(CutError::new(
            error_codes::INVALID_ARGS,
            "OTIO clip uses an unsupported media reference",
            format!("{location} active media reference is '{schema}'"),
        )),
    }
}

fn validate_structure(raw: &Value) -> Result<(), CutError> {
    if raw.get("OTIO_SCHEMA").and_then(Value::as_str) != Some("Timeline.1") {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "OTIO root is not Timeline.1",
            "expected OTIO_SCHEMA=Timeline.1",
        ));
    }
    let stack = raw.get("tracks").ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "OTIO timeline has no track stack",
            "expected tracks with OTIO_SCHEMA=Stack.1",
        )
    })?;
    if stack.get("OTIO_SCHEMA").and_then(Value::as_str) != Some("Stack.1") {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "OTIO tracks node is not Stack.1",
            "flatten nested stacks before importing the timeline",
        ));
    }
    reject_unsupported_item_state(stack, "tracks", false)?;
    let tracks = stack
        .get("children")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "OTIO track stack has no children array",
                "export a flat video/audio OTIO timeline",
            )
        })?;
    if tracks.is_empty() || tracks.len() > MAX_OTIO_TRACKS {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "OTIO track count is outside the supported range",
            format!("got {}; expected 1..={MAX_OTIO_TRACKS}", tracks.len()),
        ));
    }
    let mut total_items = 0usize;
    for (track_index, track) in tracks.iter().enumerate() {
        if track.get("OTIO_SCHEMA").and_then(Value::as_str) != Some("Track.1") {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "OTIO contains a non-track child",
                format!("tracks.children[{track_index}] is not Track.1"),
            ));
        }
        if !matches!(
            track.get("kind").and_then(Value::as_str),
            Some("Video" | "Audio")
        ) {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "OTIO track kind is unsupported",
                format!("track {track_index} must be Video or Audio"),
            ));
        }
        reject_unsupported_item_state(track, &format!("tracks.children[{track_index}]"), false)?;
        let items = track
            .get("children")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    "OTIO track has no children array",
                    format!("track {track_index}"),
                )
            })?;
        total_items = total_items.checked_add(items.len()).ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "OTIO item count overflowed",
                "timeline is too large to import safely",
            )
        })?;
        if total_items > MAX_OTIO_ITEMS {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "OTIO timeline has too many items",
                format!("got more than {MAX_OTIO_ITEMS} clips/gaps"),
            ));
        }
        for (item_index, item) in items.iter().enumerate() {
            let schema = item
                .get("OTIO_SCHEMA")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !matches!(schema, "Clip.1" | "Clip.2" | "Gap.1") {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "OTIO contains an unsupported timeline item",
                    format!(
                        "track {track_index} item {item_index} is '{schema}'; flatten transitions, stacks, and effects before import"
                    ),
                ));
            }
            let location = format!("tracks.children[{track_index}].children[{item_index}]");
            reject_unsupported_item_state(item, &location, true)?;
            if matches!(schema, "Clip.1" | "Clip.2") {
                validate_media_reference(item, &location)?;
            }
            require_time(item, "/source_range/start_time", true)?;
            require_time(item, "/source_range/duration", false)?;
        }
    }
    if total_items == 0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "OTIO timeline contains no clips or gaps",
            "add at least one timeline item before export",
        ));
    }
    Ok(())
}

fn load_file(requested: &str) -> Result<LoadedOtio, CutError> {
    let requested = PathBuf::from(requested);
    if requested
        .extension()
        .and_then(|extension| extension.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("otio"))
    {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "timeline import path must end in .otio",
            requested.display().to_string(),
        ));
    }
    let path = requested.canonicalize().map_err(|error| {
        CutError::new(
            error_codes::NOT_FOUND,
            "OTIO timeline file was not found",
            error.to_string(),
        )
    })?;
    let metadata = path.metadata()?;
    if !metadata.is_file() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "OTIO timeline path is not a regular file",
            path.display().to_string(),
        ));
    }
    if metadata.len() > MAX_OTIO_BYTES {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "OTIO timeline file is too large",
            format!(
                "{} bytes exceeds the {MAX_OTIO_BYTES} byte limit",
                metadata.len()
            ),
        ));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&path)?
        .take(MAX_OTIO_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_OTIO_BYTES {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "OTIO timeline file is too large",
            "bounded read exceeded 16 MiB",
        ));
    }
    let raw: Value = serde_json::from_slice(&bytes).map_err(|error| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "OTIO timeline JSON is invalid",
            error.to_string(),
        )
    })?;
    validate_structure(&raw)?;
    let text = std::str::from_utf8(&bytes).map_err(|error| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "OTIO timeline is not UTF-8 JSON",
            error.to_string(),
        )
    })?;
    let tracks = cut_export::otio::parse_otio(text).map_err(super::export_formats::export_error)?;
    let source_format = match (
        raw.pointer("/metadata/shellx_cut/width")
            .and_then(Value::as_u64),
        raw.pointer("/metadata/shellx_cut/height")
            .and_then(Value::as_u64),
        raw.pointer("/metadata/shellx_cut/fps")
            .and_then(Value::as_f64),
    ) {
        (Some(width), Some(height), Some(fps))
            if (16..=16_384).contains(&width)
                && (16..=16_384).contains(&height)
                && fps.is_finite()
                && fps > 0.0
                && fps <= 240.0 =>
        {
            Some(json!({"width":width,"height":height,"fps":fps}))
        }
        _ => None,
    };
    Ok(LoadedOtio {
        path,
        source_hash: format!("sha256:{:x}", Sha256::digest(&bytes)),
        name: raw
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        tracks,
        source_format,
    })
}

fn media_path(url: &str) -> Result<PathBuf, CutError> {
    let mut encoded = if let Some(rest) = url.strip_prefix("file://") {
        if let Some(local) = rest.strip_prefix("localhost/") {
            format!("/{local}")
        } else if rest.starts_with('/') {
            rest.to_string()
        } else {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "OTIO media URI has a remote file authority",
                url.to_string(),
            ));
        }
    } else if url.contains("://") {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "OTIO media reference uses an unsupported URI scheme",
            url.to_string(),
        ));
    } else {
        url.to_string()
    };
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len() {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "OTIO media URI has an incomplete percent escape",
                url.to_string(),
            ));
        }
        let high = (bytes[index + 1] as char).to_digit(16);
        let low = (bytes[index + 2] as char).to_digit(16);
        let (Some(high), Some(low)) = (high, low) else {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "OTIO media URI has an invalid percent escape",
                url.to_string(),
            ));
        };
        decoded.push((high * 16 + low) as u8);
        index += 3;
    }
    encoded = String::from_utf8(decoded).map_err(|error| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "OTIO media URI is not valid UTF-8",
            error.to_string(),
        )
    })?;
    encoded = if let Some(rest) = encoded.strip_prefix(r"/\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = encoded.strip_prefix(r"/\\?\") {
        rest.to_string()
    } else if let Some(rest) = encoded.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = encoded.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        encoded
    };
    #[cfg(windows)]
    if encoded.starts_with('/')
        && encoded.as_bytes().get(2) == Some(&b':')
        && encoded
            .as_bytes()
            .get(1)
            .is_some_and(u8::is_ascii_alphabetic)
    {
        encoded.remove(0);
    }
    Ok(PathBuf::from(encoded))
}

fn targets(
    tracks: &[cut_export::otio::OtioTrack],
    otio_path: &Path,
) -> Result<BTreeMap<String, Option<PathBuf>>, CutError> {
    let mut targets = BTreeMap::new();
    let source_dir = otio_path.parent().unwrap_or_else(|| Path::new("."));
    for track in tracks {
        for clip in &track.clips {
            if clip.is_gap || clip.target_url.is_empty() || targets.contains_key(&clip.target_url) {
                continue;
            }
            let path = media_path(&clip.target_url)?;
            let path = if path.is_relative() {
                source_dir.join(path)
            } else {
                path
            };
            let available = path.is_file().then(|| path.canonicalize()).transpose()?;
            targets.insert(clip.target_url.clone(), available);
        }
    }
    Ok(targets)
}

fn preview_result(loaded: &LoadedOtio, targets: &BTreeMap<String, Option<PathBuf>>) -> Value {
    let mut clips = 0usize;
    let mut gaps = 0usize;
    let mut missing_clips = 0usize;
    let tracks: Vec<Value> = loaded
        .tracks
        .iter()
        .map(|track| {
            let mut track_clips = 0usize;
            let mut track_gaps = 0usize;
            let duration_ms = track.clips.iter().fold(0u64, |total, clip| {
                if clip.is_gap {
                    gaps += 1;
                    track_gaps += 1;
                } else {
                    clips += 1;
                    track_clips += 1;
                    if targets.get(&clip.target_url).is_none_or(Option::is_none) {
                        missing_clips += 1;
                    }
                }
                total.saturating_add(clip.dur_ms)
            });
            json!({
                "name":track.name,
                "kind":track.kind,
                "clips":track_clips,
                "gaps":track_gaps,
                "duration_ms":duration_ms,
            })
        })
        .collect();
    let missing_sources: Vec<String> = targets
        .iter()
        .filter(|(_, path)| path.is_none())
        .map(|(url, _)| {
            media_path(url)
                .ok()
                .and_then(|path| {
                    path.file_name()
                        .map(|name| name.to_string_lossy().to_string())
                })
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "unresolved media".into())
        })
        .collect();
    json!({
        "status":"preview",
        "path":loaded.path,
        "source_hash":loaded.source_hash,
        "name":loaded.name,
        "tracks":tracks,
        "track_count":loaded.tracks.len(),
        "clips":clips,
        "gaps":gaps,
        "media_references":targets.len(),
        "media_available":targets.values().filter(|path| path.is_some()).count(),
        "media_missing":targets.values().filter(|path| path.is_none()).count(),
        "missing_clips":missing_clips,
        "missing_sources":missing_sources,
        "source_format":loaded.source_format,
        "format_policy":"preserve_project",
    })
}

fn track_id(name: &str, kind: &str, index: usize, used: &mut BTreeSet<String>) -> String {
    let mut base: String = name
        .trim()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
        .take(48)
        .collect();
    if base.is_empty() {
        base = format!("otio_{kind}{}", index + 1);
    }
    let mut candidate = base.clone();
    let mut suffix = 2usize;
    while used.contains(&candidate) {
        candidate = format!("{base}_{suffix}");
        suffix += 1;
    }
    used.insert(candidate.clone());
    candidate
}

/// OTIO stores time as rational frame values while Cut stores integer
/// milliseconds. A Cut clip ending exactly at a non-frame-aligned media EOF can
/// therefore round up by less than one source frame on export and come back a
/// few milliseconds past the probe duration. Clamp only that serialization
/// drift; unknown-rate or genuinely out-of-range clips remain hard failures.
fn clamp_source_end_for_otio_rounding(
    source_in: u64,
    requested_end: u64,
    media_duration: u64,
    source_fps: Option<f64>,
) -> Option<u64> {
    if requested_end <= media_duration {
        return Some(requested_end);
    }
    let fps = source_fps.filter(|fps| fps.is_finite() && *fps > 0.0 && *fps <= 240.0)?;
    let frame_ms = (1000.0 / fps).ceil() as u64;
    (source_in < media_duration && requested_end.saturating_sub(media_duration) <= frame_ms.max(1))
        .then_some(media_duration)
}

pub(super) async fn import_otio(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        path: String,
        mode: Option<String>,
        expected_hash: Option<String>,
        rationale: Option<String>,
    }
    let args: Args = parse_args(args)?;
    let mode = args.mode.as_deref().unwrap_or("replace");
    if !matches!(mode, "preview" | "replace") {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "import.otio mode must be preview or replace",
            mode.to_string(),
        ));
    }
    let requested_path = args.path.clone();
    let loaded = run_blocking("import.otio.read", move || load_file(&requested_path)).await?;
    let targets = targets(&loaded.tracks, &loaded.path)?;
    let preview = preview_result(&loaded, &targets);
    if mode == "preview" {
        return Ok(VerbResult::ok(preview));
    }
    if args
        .expected_hash
        .as_deref()
        .is_some_and(|expected| expected != loaded.source_hash)
    {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "OTIO file changed after preview",
            format!(
                "expected {}, actual {}",
                args.expected_hash.as_deref().unwrap_or_default(),
                loaded.source_hash
            ),
        )
        .with_suggested_action("preview the file again before replacing the timeline"));
    }
    let expected_project_dir = {
        let guard = state.project.read().await;
        guard.as_ref().ok_or_else(no_project)?.dir.clone()
    };
    let available_paths: Vec<PathBuf> = targets
        .values()
        .filter_map(Clone::clone)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let prepared = run_blocking("import.otio.media_preflight", move || {
        available_paths
            .into_iter()
            .map(|path| {
                let hash = cut_core::hash_file(&path)?;
                let probe = cut_media::probe(&path)?;
                Ok(PreparedMedia { path, hash, probe })
            })
            .collect::<Result<Vec<_>, CutError>>()
    })
    .await?;

    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    if store.dir != expected_project_dir {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "the project changed during OTIO media preflight",
            format!(
                "import belongs to {}; open project is {}",
                expected_project_dir.display(),
                store.dir.display()
            ),
        ));
    }
    let mut asset_for_path = BTreeMap::<PathBuf, String>::new();
    let mut new_media = Vec::new();
    for media in prepared {
        let existing = store.project.assets.iter().find_map(|(id, asset)| {
            (asset.probe.is_some()
                && asset.hash == media.hash
                && Path::new(&asset.path)
                    .canonicalize()
                    .ok()
                    .is_some_and(|path| path == media.path))
            .then(|| id.clone())
        });
        if let Some(id) = existing {
            asset_for_path.insert(media.path.clone(), id);
        } else {
            new_media.push(media);
        }
    }
    let ids = store.next_asset_ids(new_media.len())?;
    let mut new_assets = BTreeMap::new();
    let mut enrichment = Vec::new();
    for (media, id) in new_media.into_iter().zip(ids) {
        new_assets.insert(
            id.clone(),
            cut_core::Asset {
                path: media.path.display().to_string(),
                hash: media.hash.clone(),
                probe: Some(serde_json::to_value(&media.probe)?),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
        asset_for_path.insert(media.path.clone(), id.clone());
        enrichment.push((id, media.path, media.hash));
    }

    let mut used_track_ids = BTreeSet::new();
    let mut imported_tracks = Vec::new();
    let mut clips_inserted = 0usize;
    let mut gaps = 0usize;
    let mut missing_clips = 0usize;
    let mut rounded_source_clips = 0usize;
    let mut next_clip = 1usize;
    let source_fps = loaded
        .source_format
        .as_ref()
        .and_then(|format| format.get("fps"))
        .and_then(Value::as_f64);
    for (track_index, track) in loaded.tracks.iter().enumerate() {
        let kind = if track.kind == "audio" {
            "audio"
        } else {
            "video"
        };
        let track_id = track_id(&track.name, kind, track_index, &mut used_track_ids);
        let mut items = Vec::new();
        for clip in &track.clips {
            if clip.is_gap {
                gaps += 1;
                items.push(json!({"kind":"gap","duration_ms":clip.dur_ms}));
                continue;
            }
            let Some(path) = targets.get(&clip.target_url).and_then(Clone::clone) else {
                gaps += 1;
                missing_clips += 1;
                items.push(json!({"kind":"gap","duration_ms":clip.dur_ms}));
                continue;
            };
            let asset_id = asset_for_path.get(&path).ok_or_else(|| {
                CutError::new(
                    error_codes::CONFLICT,
                    "OTIO media preflight lost an available source",
                    path.display().to_string(),
                )
            })?;
            let asset = new_assets
                .get(asset_id)
                .or_else(|| store.project.assets.get(asset_id))
                .ok_or_else(|| {
                    CutError::new(
                        error_codes::CONFLICT,
                        "OTIO media asset disappeared before commit",
                        asset_id.clone(),
                    )
                })?;
            let probe = asset.probe.as_ref().ok_or_else(|| {
                CutError::new(
                    error_codes::CONFLICT,
                    "OTIO media has no probe after preflight",
                    path.display().to_string(),
                )
            })?;
            let probe_kind = probe.get("kind").and_then(Value::as_str).unwrap_or("");
            let has_audio = probe
                .get("has_audio")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if (kind == "video" && probe_kind == "audio") || (kind == "audio" && !has_audio) {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "OTIO track kind does not match referenced media",
                    format!(
                        "track '{track_id}' ({kind}) references {} ({probe_kind})",
                        path.display()
                    ),
                ));
            }
            let requested_source_end =
                clip.src_in_ms.checked_add(clip.dur_ms).ok_or_else(|| {
                    CutError::new(
                        error_codes::INVALID_ARGS,
                        "OTIO clip source range overflowed",
                        format!("{} + {} ms", clip.src_in_ms, clip.dur_ms),
                    )
                })?;
            let source_end = match probe.get("duration_ms").and_then(Value::as_u64) {
                Some(duration) => clamp_source_end_for_otio_rounding(
                    clip.src_in_ms,
                    requested_source_end,
                    duration,
                    source_fps,
                )
                .ok_or_else(|| {
                    CutError::new(
                        error_codes::INVALID_ARGS,
                        "OTIO clip source range exceeds the media duration",
                        format!(
                            "{} requests {}..{} ms",
                            path.display(),
                            clip.src_in_ms,
                            requested_source_end
                        ),
                    )
                })?,
                None => requested_source_end,
            };
            if source_end != requested_source_end {
                rounded_source_clips += 1;
            }
            items.push(json!({
                "id":format!("otio_c{next_clip}"),
                "asset":asset_id,
                "src_in_ms":clip.src_in_ms,
                "src_out_ms":source_end,
            }));
            next_clip += 1;
            clips_inserted += 1;
        }
        imported_tracks.push(serde_json::from_value::<cut_core::Track>(json!({
            "id":track_id,
            "kind":kind,
            "clips":items,
        }))?);
    }
    let new_asset_count = new_assets.len();
    let reused_asset_count = asset_for_path.len().saturating_sub(new_asset_count);
    let op = guard_call("import.otio", || {
        store.replace_timeline_from_otio(
            imported_tracks,
            new_assets,
            loaded.source_hash.clone(),
            actor,
            args.rationale,
        )
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op });
    drop(guard);
    let jobs: Vec<String> = enrichment
        .into_iter()
        .map(|(asset_id, path, hash)| {
            super::media::spawn_import_chain(state.clone(), asset_id, path, hash, None, true, false)
        })
        .collect();
    let mut result = VerbResult::ok_with_ops(
        json!({
            "status":"imported",
            "source_hash":loaded.source_hash,
            "tracks_created":loaded.tracks.len(),
            "clips_inserted":clips_inserted,
            "gaps":gaps,
            "missing_clips":missing_clips,
            "time_clamped_clips":rounded_source_clips,
            "assets_imported":new_asset_count,
            "assets_reused":reused_asset_count,
            "jobs":jobs,
            "format_policy":"preserve_project",
            "source_format":loaded.source_format,
        }),
        vec![op_id],
    );
    let mut warnings = Vec::new();
    if rounded_source_clips > 0 {
        warnings.push(cut_core::VerbWarning {
            code: "otio_time_clamped".into(),
            message: format!(
                "{rounded_source_clips} clip source range(s) were clamped to media EOF after sub-frame OTIO time rounding"
            ),
            detail: Default::default(),
        });
    }
    if missing_clips > 0 {
        warnings.push(cut_core::VerbWarning {
            code: "otio_media_missing".into(),
            message: format!(
                "{missing_clips} clip(s) reference unavailable media and were preserved as timed gaps"
            ),
            detail: Default::default(),
        });
    }
    if !warnings.is_empty() {
        result = result.with_warnings(warnings);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_uri_rejects_remote_authority_and_bad_escapes() {
        assert!(media_path("file://remote/share/a.mov").is_err());
        assert!(media_path("https://example.test/a.mov").is_err());
        assert!(media_path("bad%2.mov").is_err());
        assert_eq!(media_path("A%20B.mov").unwrap(), PathBuf::from("A B.mov"));
        assert_eq!(
            media_path("file:///%5C%5C%3F%5CC:%5CUsers%5CEditor%5Cclip.mov").unwrap(),
            PathBuf::from(r"C:\Users\Editor\clip.mov"),
        );
    }

    #[test]
    fn source_end_clamps_only_sub_frame_otio_rounding() {
        assert_eq!(
            clamp_source_end_for_otio_rounding(0, 58_167, 58_162, Some(30.0)),
            Some(58_162)
        );
        assert_eq!(
            clamp_source_end_for_otio_rounding(1_000, 2_000, 2_000, Some(30.0)),
            Some(2_000)
        );
        assert_eq!(
            clamp_source_end_for_otio_rounding(0, 58_202, 58_162, Some(30.0)),
            None
        );
        assert_eq!(
            clamp_source_end_for_otio_rounding(0, 58_167, 58_162, None),
            None
        );
        assert_eq!(
            clamp_source_end_for_otio_rounding(58_162, 58_167, 58_162, Some(30.0)),
            None
        );
    }

    #[test]
    fn structure_rejects_unknown_items_instead_of_silently_dropping_them() {
        let raw = json!({
            "OTIO_SCHEMA":"Timeline.1",
            "tracks":{"OTIO_SCHEMA":"Stack.1","children":[{
                "OTIO_SCHEMA":"Track.1","kind":"Video","children":[{
                    "OTIO_SCHEMA":"Transition.1",
                    "source_range":{
                        "start_time":{"value":0,"rate":30},
                        "duration":{"value":10,"rate":30}
                    }
                }]
            }]}
        });
        let error = validate_structure(&raw).unwrap_err();
        assert_eq!(error.code, error_codes::INVALID_ARGS);
        assert!(error.message.contains("unsupported"));
    }

    #[test]
    fn structure_rejects_semantics_the_importer_cannot_preserve() {
        let base = json!({
            "OTIO_SCHEMA":"Timeline.1",
            "tracks":{"OTIO_SCHEMA":"Stack.1","children":[{
                "OTIO_SCHEMA":"Track.1","kind":"Video","children":[{
                    "OTIO_SCHEMA":"Clip.2",
                    "source_range":{
                        "start_time":{"value":0,"rate":30},
                        "duration":{"value":10,"rate":30}
                    },
                    "media_references":{
                        "DEFAULT_MEDIA":{"OTIO_SCHEMA":"MissingReference.1"}
                    }
                }]}
            ]}
        });

        for (parent, field, value) in [
            ("/tracks/children/0", "enabled", json!(false)),
            (
                "/tracks/children/0",
                "effects",
                json!([{"OTIO_SCHEMA":"Effect.1"}]),
            ),
            (
                "/tracks/children/0/children/0",
                "markers",
                json!([{"OTIO_SCHEMA":"Marker.2"}]),
            ),
            ("/tracks", "source_range", json!({"unsupported":"crop"})),
            (
                "/tracks/children/0/children/0/media_references/DEFAULT_MEDIA",
                "OTIO_SCHEMA",
                json!("GeneratorReference.1"),
            ),
        ] {
            let mut raw = base.clone();
            raw.pointer_mut(parent)
                .and_then(Value::as_object_mut)
                .unwrap()
                .insert(field.into(), value);
            assert!(
                validate_structure(&raw).is_err(),
                "unsupported semantic field at {parent}/{field} must be refused"
            );
        }
    }
}

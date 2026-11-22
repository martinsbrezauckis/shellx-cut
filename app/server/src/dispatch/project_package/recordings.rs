// Recording editing inputs use the same source admission and copy path as media.
const MAX_RECORDING_PLAN_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
struct RecordingNotCarried {
    take: String,
    reason: String,
}

fn recording_tree_is_plain(root: &Path) -> bool {
    fs::symlink_metadata(root).is_ok_and(|m| is_plain_dir(&m))
        && fs::read_dir(root).is_ok_and(|entries| {
            entries.into_iter().all(|entry| {
                entry.is_ok_and(|entry| {
                    fs::symlink_metadata(entry.path()).is_ok_and(|m| {
                        is_plain_regular(&m)
                            || (is_plain_dir(&m) && recording_tree_is_plain(&entry.path()))
                    })
                })
            })
        })
}

fn recording_member_path(folder: &str, relative: &Path) -> Result<String, CutError> {
    let raw = relative.to_str().ok_or_else(|| source_changed(relative))?;
    if raw.contains(['\\', ':'])
        || raw
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(source_changed(relative));
    }
    let components = relative
        .components()
        .map(|component| match component {
            Component::Normal(name) => {
                let text = name.to_str().ok_or_else(|| source_changed(relative))?;
                if text.is_empty() || text.contains(['/', '\\', ':']) {
                    return Err(source_changed(relative));
                }
                Ok(text)
            }
            _ => Err(source_changed(relative)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if components.is_empty() {
        return Err(source_changed(relative));
    }
    let path = format!("cache/screen_record/{folder}/{}", components.join("/"));
    if !is_manifest_member_path(&path) {
        return Err(source_changed(relative));
    }
    Ok(path)
}

fn named_recording_file(
    root: &Path,
    take: &str,
    value: &str,
    member: String,
    label: &str,
) -> Result<PackageSourceFile, String> {
    if !is_manifest_member_path(&member) {
        return Err(format!("{label} has an unsafe name"));
    }
    let path = source_path(root, value);
    let metadata =
        fs::symlink_metadata(&path).map_err(|_| format!("{label} is missing or cannot be read"))?;
    if !is_plain_regular(&metadata) {
        return Err(format!("{label} is not a plain file"));
    }
    let candidate = path
        .canonicalize()
        .map_err(|_| format!("{label} is missing or cannot be read"))?;
    let project_root = root
        .canonicalize()
        .map_err(|_| "project folder cannot be read".to_string())?;
    if !candidate.starts_with(&project_root) {
        return Err(format!("{label} is outside this project"));
    }
    let (canonical, stamp, bytes, sha256) =
        inspect_source(&path).map_err(|_| format!("{label} is missing or cannot be read"))?;
    if !canonical.starts_with(&project_root) {
        return Err(format!("{label} is outside this project"));
    }
    Ok(PackageSourceFile {
        asset_id: format!("recording:{take}"),
        source: canonical,
        stamp,
        bytes,
        sha256,
        package_path: member,
        rewritten: None,
    })
}

fn add_take_file(
    root: &Path,
    take: &str,
    value: &str,
    member: String,
    label: &str,
    files: &mut Vec<PackageSourceFile>,
) -> Result<PathBuf, String> {
    let file = named_recording_file(root, take, value, member, label)?;
    let path = file.source.clone();
    if !files.iter().any(|existing| existing.source == path) {
        files.push(file);
    }
    Ok(path)
}

fn read_recording_plan(file: &PackageSourceFile) -> Result<record_core::EditPlan, String> {
    if file.stamp.len > MAX_RECORDING_PLAN_BYTES {
        return Err("plan is too large to read".into());
    }
    let input = open_plain_regular(&file.source).map_err(|_| "plan cannot be read".to_string())?;
    if file_stamp(&input.metadata().map_err(|_| "plan cannot be read")?) != file.stamp {
        return Err("plan changed while making the copy".into());
    }
    let mut bytes = Vec::new();
    input
        .take(MAX_RECORDING_PLAN_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "plan cannot be read".to_string())?;
    if bytes.len() as u64 > MAX_RECORDING_PLAN_BYTES {
        return Err("plan is too large to read".into());
    }
    if format!("{:x}", Sha256::digest(&bytes)) != file.sha256 {
        return Err("plan changed while making the copy".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "plan cannot be read".into())
}

fn add_one_take(
    source: &PackageSourceSnapshot,
    take: &cut_core::recording::RecordingTake,
) -> Result<Vec<PackageSourceFile>, String> {
    add_one_take_after_plan_admission(source, take, || {})
}

fn add_one_take_after_plan_admission(
    source: &PackageSourceSnapshot,
    take: &cut_core::recording::RecordingTake,
    after_admission: impl FnOnce(),
) -> Result<Vec<PackageSourceFile>, String> {
    let root = &source.project_dir;
    let folder = format!("take-{:x}", Sha256::digest(take.id.as_bytes()));
    let member = |name: &str| {
        recording_member_path(&folder, Path::new(name))
            .map_err(|_| format!("{name} has an unsafe name"))
    };
    let mut files = Vec::new();
    let source_file = add_take_file(
        root,
        &take.id,
        &take.source,
        member("source.mp4")?,
        "source video",
        &mut files,
    )?;
    let capture = source_file
        .parent()
        .ok_or_else(|| "source video has no folder".to_string())?;
    if source_path(root, &take.capture_folder)
        .canonicalize()
        .map_err(|_| "capture folder is missing".to_string())?
        != capture
    {
        return Err("source video is outside its capture folder".into());
    }
    add_take_file(
        root,
        &take.id,
        &take.events,
        member("events.json")?,
        "events",
        &mut files,
    )?;
    let pending = add_take_file(
        root,
        &take.id,
        &take.pending_plan,
        member("plan-pending.json")?,
        "pending plan",
        &mut files,
    )?;
    let applied = add_take_file(
        root,
        &take.id,
        &take.applied_plan,
        member("plan-applied.json")?,
        "applied plan",
        &mut files,
    )?;
    after_admission();
    let mut plans = Vec::new();
    for path in [&pending, &applied] {
        if plans.iter().any(|(seen, _)| seen == path) {
            continue;
        }
        let file = files
            .iter()
            .find(|file| &file.source == path)
            .ok_or_else(|| "plan is missing".to_string())?;
        let plan = read_recording_plan(file)?;
        let plan = crate::screen_record::camera_look::plan_to_bake(take, plan);
        if take.camera_look.is_some() {
            plan.validate()
                .map_err(|_| "separate screen plan is invalid".to_string())?;
        }
        plans.push((path.clone(), plan));
    }
    let mic = capture.join("mic.wav");
    match fs::symlink_metadata(&mic) {
        Ok(_) => {
            add_take_file(
                root,
                &take.id,
                &mic.to_string_lossy(),
                member("mic.wav")?,
                "microphone audio",
                &mut files,
            )?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("microphone audio cannot be inspected".into()),
    }
    if let Some(video) = take
        .camera_artifact
        .as_ref()
        .and_then(|artifact| artifact["video"].as_str())
    {
        let portable = video.starts_with("media/sha256/");
        let camera_path = if portable {
            source_path(root, video)
        } else {
            capture.join(video)
        };
        let mut camera_file = named_recording_file(
            root,
            &take.id,
            &camera_path.to_string_lossy(),
            if portable {
                video.to_owned()
            } else {
                member(video)?
            },
            "camera video",
        )?;
        let camera = camera_file.source.clone();
        if let Some(camera_id) = &take.camera_asset {
            let asset = source
                .project
                .assets
                .get(camera_id)
                .ok_or_else(|| "camera asset is missing".to_string())?;
            let asset_path = source_path(root, &asset.path)
                .canonicalize()
                .map_err(|_| "camera asset is missing".to_string())?;
            if asset_path != camera {
                return Err("camera asset does not match the named camera video".into());
            }
            camera_file.package_path = format!(
                "media/sha256/{}.{}",
                camera_file.sha256,
                safe_extension(&camera)
            );
        }
        if !files.iter().any(|existing| existing.source == camera) {
            files.push(camera_file);
        }
    } else if take.camera_asset.is_some() {
        return Err("camera video is not named".into());
    }
    for (_, plan) in &plans {
        if let Some(webcam) = &plan.webcam {
            let camera_name = format!(
                "camera/plan-camera-{:x}.mp4",
                Sha256::digest(webcam.source.as_bytes())
            );
            add_take_file(
                root,
                &take.id,
                &webcam.source,
                member(&camera_name)?,
                "camera video",
                &mut files,
            )?;
        }
    }
    let mut destinations = BTreeMap::new();
    for file in &files {
        if destinations
            .insert(file.package_path.clone(), file.source.clone())
            .is_some()
        {
            return Err("named files would share one path in the copy".into());
        }
    }
    let mapping = recording_path_map(&files);
    for file in &mut files {
        let Some((_, mut plan)) = plans.iter().find(|(path, _)| *path == file.source).cloned()
        else {
            continue;
        };
        if let Some(webcam) = plan.webcam.as_mut() {
            rebase_recording_path(&mut webcam.source, root, &mapping);
        }
        if let record_core::Background::Image { path } = &mut plan.background {
            rebase_recording_path(path, root, &mapping);
        }
        if let Some(captions) = plan.captions.as_mut() {
            rebase_recording_path(&mut captions.words_path, root, &mapping);
        }
        let bytes =
            serde_json::to_vec_pretty(&plan).map_err(|_| "plan cannot be rewritten".to_string())?;
        if bytes.len() as u64 > MAX_RECORDING_PLAN_BYTES {
            return Err("rewritten plan is too large".into());
        }
        file.rewritten = Some((file.sha256.clone(), bytes.clone()));
        file.bytes = bytes.len() as u64;
        file.sha256 = format!("{:x}", Sha256::digest(&bytes));
    }
    Ok(files)
}

fn add_recording_files(
    source: &PackageSourceSnapshot,
    files: &mut Vec<PackageSourceFile>,
) -> Vec<RecordingNotCarried> {
    let mut omitted = Vec::new();
    for take in &source.project.recordings {
        match add_one_take(source, take) {
            Ok(named) => files.extend(named),
            Err(reason) => omitted.push(RecordingNotCarried {
                take: take.id.clone(),
                reason,
            }),
        }
    }
    omitted
}

fn recording_path_map(files: &[PackageSourceFile]) -> BTreeMap<PathBuf, String> {
    let mut mapping = BTreeMap::new();
    for file in files {
        mapping
            .entry(file.source.clone())
            .or_insert_with(|| file.package_path.clone());
    }
    mapping
}

fn rebase_recording_path(path: &mut String, root: &Path, mapping: &BTreeMap<PathBuf, String>) {
    let resolved = source_path(root, path);
    if let Some(new) = resolved.canonicalize().ok().and_then(|p| mapping.get(&p)) {
        *path = new.clone();
    } else if let Some((old, new)) = mapping
        .iter()
        .find(|(old, _)| old.parent() == Some(resolved.as_path()))
    {
        if let (Some(_), Some(parent)) = (old.parent(), Path::new(new).parent()) {
            *path = parent.to_string_lossy().into_owned();
        }
    }
}

fn rebase_recordings(
    project: &mut Project,
    root: &Path,
    files: &[PackageSourceFile],
    omitted: &[RecordingNotCarried],
) -> Result<(), CutError> {
    project
        .recordings
        .retain(|take| !omitted.iter().any(|entry| entry.take == take.id));
    for take in &mut project.recordings {
        let id = format!("recording:{}", take.id);
        let original_capture = take.capture_folder.clone();
        let mapping = recording_path_map(
            &files
                .iter()
                .filter(|file| file.asset_id == id)
                .cloned()
                .collect::<Vec<_>>(),
        );
        for path in [
            &mut take.source,
            &mut take.events,
            &mut take.pending_plan,
            &mut take.applied_plan,
        ] {
            let canonical = source_path(root, path).canonicalize()?;
            *path = mapping
                .get(&canonical)
                .ok_or_else(|| source_changed(&canonical))?
                .clone();
        }
        take.capture_folder = Path::new(&take.source)
            .parent()
            .ok_or_else(|| source_changed(Path::new(&take.source)))?
            .to_string_lossy()
            .into_owned();
        if take.camera_asset.is_some() {
            let video = take
                .camera_artifact
                .as_mut()
                .and_then(|artifact| artifact.get_mut("video"))
                .ok_or_else(|| source_changed(Path::new(&take.capture_folder)))?;
            let original = video
                .as_str()
                .ok_or_else(|| source_changed(Path::new(&take.capture_folder)))?;
            let old_path = if original.starts_with("media/sha256/") {
                source_path(root, original)
            } else {
                source_path(root, &original_capture).join(original)
            };
            let canonical = old_path.canonicalize()?;
            *video = serde_json::Value::String(
                mapping
                    .get(&canonical)
                    .ok_or_else(|| source_changed(&canonical))?
                    .clone(),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod security_tests {
    use super::*;

    #[test]
    fn recording_member_rejects_backslash_and_dot_components() {
        assert!(recording_member_path("safe", Path::new("..\\outside")).is_err());
        assert!(recording_member_path("safe", Path::new("./source.mp4")).is_err());
        assert!(recording_member_path("safe", Path::new("camera/camera.mp4")).is_ok());
    }
}

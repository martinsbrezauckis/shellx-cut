async fn prepare(
    state: &AppState,
    destination: String,
    name: String,
    b5_receipt: Option<Value>,
) -> Result<PreparedPackage, CutError> {
    validate_package_name(&name)?;
    let destination = validate_destination(PathBuf::from(destination))?;
    let target = destination.join(format!("{name}.cutproj"));
    let snapshot = snapshot_source(state).await?;
    let b5_receipt_sha256 = validate_b5_receipt(b5_receipt.as_ref(), &snapshot)?;
    let for_blocking = snapshot.clone();
    run_blocking("project.package_plan", move || {
        build_prepared(for_blocking, destination, target, name, b5_receipt_sha256)
    })
    .await
}

async fn snapshot_source(state: &AppState) -> Result<PackageSourceSnapshot, CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let project_revision = store.log.current_revision()?.ok_or_else(|| {
        CutError::new(
            error_codes::CONFLICT,
            "portable package requires a durable source revision",
            "save or reopen the project before packaging",
        )
    })?;
    let ops = store.log.read_all()?;
    let current_b5_receipt = current_relink_receipt(&ops)?;
    if ops.iter().any(|op| {
        matches!(
            op.verb.as_str(),
            "motion.apply_import" | "motion.link.refresh" | "motion.link.relink"
        )
    }) {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "portable package refuses Motion-linked provenance",
            "this first B6 slice will not silently detach or rewrite external Motion package links",
        ));
    }
    Ok(PackageSourceSnapshot {
        project: store.project.clone(),
        project_dir: store.dir.clone(),
        project_identity: path_free_project_identity(store)?,
        project_revision,
        ops,
        current_b5_receipt,
    })
}

fn build_prepared(
    source: PackageSourceSnapshot,
    destination: PathBuf,
    target: PathBuf,
    name: String,
    b5_receipt_sha256: Option<String>,
) -> Result<PreparedPackage, CutError> {
    let referenced = referenced_asset_ids(&source.project)?;
    let mut raw = Vec::with_capacity(referenced.len());
    for asset_id in referenced {
        let asset = source.project.assets.get(&asset_id).ok_or_else(|| {
            CutError::new(
                error_codes::CONFLICT,
                "timeline references an unavailable project asset",
                format!("asset '{asset_id}' is not in the project asset map"),
            )
        })?;
        let source_path = source_path(&source.project_dir, &asset.path);
        let (canonical, stamp, bytes, sha256) = inspect_source(&source_path)?;
        raw.push((asset_id, canonical, stamp, bytes, sha256));
    }
    let mut extension_by_hash = BTreeMap::<String, String>::new();
    for (_, path, _, _, sha256) in &raw {
        extension_by_hash
            .entry(sha256.clone())
            .or_insert_with(|| safe_extension(path));
    }
    let files = raw
        .into_iter()
        .map(
            |(asset_id, source_path, stamp, bytes, sha256)| PackageSourceFile {
                asset_id,
                source: source_path,
                stamp,
                bytes,
                package_path: format!(
                    "media/sha256/{}.{}",
                    sha256,
                    extension_by_hash
                        .get(&sha256)
                        .expect("every exact hash has a selected extension")
                ),
                sha256,
            },
        )
        .collect::<Vec<_>>();
    let mut first_by_path = BTreeMap::<String, &PackageSourceFile>::new();
    for file in &files {
        first_by_path
            .entry(file.package_path.clone())
            .or_insert(file);
    }
    let total_source_bytes = files.iter().map(|file| file.bytes).sum();
    let total_package_bytes = first_by_path.values().map(|file| file.bytes).sum();
    let assets = files
        .iter()
        .map(|file| {
            Ok(PackagePlanAsset {
                asset: file.asset_id.clone(),
                source_path: file.source.to_str().ok_or_else(|| CutError::new(
                    error_codes::INVALID_ARGS,
                    "portable package source path cannot be shown exactly",
                    format!("asset '{}' has a source filename outside Unicode; rename the file before making a portable copy", file.asset_id),
                ))?.to_owned(),
                bytes: file.bytes,
                sha256: format!("sha256:{}", file.sha256),
                package_path: file.package_path.clone(),
            })
        })
        .collect::<Result<Vec<_>, CutError>>()?;
    let plan = PackagePlan {
        schema: PACKAGE_PLAN_SCHEMA,
        project_identity: source.project_identity.clone(),
        project_revision: source.project_revision.clone(),
        destination: destination.to_string_lossy().into_owned(),
        name: name.clone(),
        target: target.to_string_lossy().into_owned(),
        target_status: package_target_status(&target)?,
        b5_receipt_sha256,
        assets,
        total_source_bytes,
        total_package_bytes,
        source_file_count: files.len(),
        unique_media_count: first_by_path.len(),
        policy: PackagePlanPolicy {
            only_referenced_media: true,
            derived_cache: "excluded",
            offline_media: "refuse",
            collision: "refuse",
            publication: "atomic_no_replace_native",
        },
    };
    Ok(PreparedPackage {
        plan,
        source,
        destination,
        target,
        name,
        files,
    })
}

fn referenced_asset_ids(project: &Project) -> Result<BTreeSet<String>, CutError> {
    let mut result = BTreeSet::new();
    for track in project.all_sequence_tracks() {
        for clip in &track.clips {
            if let Clip::Media(media) = clip {
                result.insert(media.asset.clone());
            }
        }
    }
    Ok(result)
}

fn validate_destination(destination: PathBuf) -> Result<PathBuf, CutError> {
    let metadata = fs::symlink_metadata(&destination).map_err(|error| {
        CutError::new(
            error_codes::NOT_FOUND,
            "portable package destination is unavailable",
            error.to_string(),
        )
    })?;
    if !is_plain_dir(&metadata) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "portable package destination must be a real local directory",
            "symlink and reparse-point destinations are refused",
        ));
    }
    destination.canonicalize().map_err(Into::into)
}

/// Report a destination leaf as occupied whenever it exists in the directory
/// entry table. `Path::exists` follows links and would wrongly describe a
/// dangling symlink as free even though native no-replace publication refuses
/// that leaf. Inspection failures are not availability claims.
fn package_target_status(target: &Path) -> Result<&'static str, CutError> {
    match fs::symlink_metadata(target) {
        Ok(_) => Ok("occupied"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok("available"),
        Err(error) => Err(CutError::new(
            error_codes::IO,
            "portable package destination could not be inspected",
            error.to_string(),
        )
        .with_suggested_action(
            "resolve access to the selected destination folder, then preview the copy again",
        )),
    }
}

fn validate_package_name(name: &str) -> Result<(), CutError> {
    let valid = !name.is_empty()
        && name.len() <= 80
        && !name.ends_with('.')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '-' | '_'));
    if valid {
        Ok(())
    } else {
        Err(CutError::new(
            error_codes::INVALID_ARGS,
            "portable package name is not cross-platform safe",
            "use 1-80 ASCII letters, digits, spaces, hyphens, or underscores; do not end with a dot",
        ))
    }
}

fn source_path(project_dir: &Path, value: &str) -> PathBuf {
    let value = PathBuf::from(value);
    if value.is_absolute() {
        value
    } else {
        project_dir.join(value)
    }
}

fn inspect_source(path: &Path) -> Result<(PathBuf, FileStamp, u64, String), CutError> {
    let canonical = path.canonicalize().map_err(|error| {
        CutError::new(
            error_codes::NOT_FOUND,
            "referenced media is offline",
            format!("{}: {error}", path.display()),
        )
    })?;
    let mut file = open_plain_regular(&canonical)?;
    let before = file_stamp(&file.metadata()?);
    let (bytes, sha256) = stream_sha256(&mut file, None)?;
    let after = file_stamp(&file.metadata()?);
    if before != after {
        return Err(source_changed(&canonical));
    }
    Ok((canonical, after, bytes, sha256))
}

fn verify_source(
    file: &PackageSourceFile,
    cancel: Option<&crate::jobs::JobCancellation>,
) -> Result<(), CutError> {
    let mut input = open_plain_regular(&file.source)?;
    let before = file_stamp(&input.metadata()?);
    if before != file.stamp {
        return Err(source_changed(&file.source));
    }
    let (bytes, sha256) = stream_sha256(&mut input, cancel)?;
    let after = file_stamp(&input.metadata()?);
    if before != after || bytes != file.bytes || sha256 != file.sha256 {
        return Err(source_changed(&file.source));
    }
    Ok(())
}

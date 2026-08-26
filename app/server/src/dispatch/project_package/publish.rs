struct PublishedPackage {
    destination: String,
    manifest_sha256: String,
    source_revision: String,
    file_count: usize,
    bytes: u64,
    warnings: Vec<String>,
}
fn publish_package(
    prepared: PreparedPackage,
    cancel: &crate::jobs::JobCancellation,
    mut progress: impl FnMut(f32, String),
    before_publish: impl FnOnce(&Path, &Path) -> Result<(), CutError>,
) -> Result<PublishedPackage, CutError> {
    ensure_plain_dir(&prepared.destination)?;
    if prepared.target.exists() {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "portable package destination already exists",
            prepared.target.display().to_string(),
        ));
    }
    let stage = create_stage(&prepared.destination, &prepared.name)?;
    let result = (|| {
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let package_dir = stage.path.join(format!("{}.cutproj", prepared.name));
        let mut package_store = ProjectStore::create(&stage.path, &prepared.name, None)?;
        let media_dir = package_dir.join("media").join("sha256");
        fs::create_dir_all(&media_dir)?;
        let mut groups = BTreeMap::<String, Vec<&PackageSourceFile>>::new();
        for file in &prepared.files {
            groups
                .entry(file.package_path.clone())
                .or_default()
                .push(file);
        }
        let total = groups.len().max(1) as f32;
        for (index, (relative, files)) in groups.iter().enumerate() {
            if cancel.is_cancelled() {
                return Err(cancelled());
            }
            let first = files.first().expect("non-empty hash group");
            let final_path = package_dir.join(relative);
            let part = media_dir.join(format!(".package-{index:08}.part"));
            copy_source(first, &part, cancel)?;
            for duplicate in files.iter().skip(1) {
                verify_source(duplicate, Some(cancel))?;
            }
            record_recovery::publish_new_synced(&part, &final_path)?;
            progress(
                0.05 + ((index + 1) as f32 / total) * 0.70,
                format!("copied {}/{} media files", index + 1, groups.len()),
            );
        }

        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        progress(0.80, "writing portable project baseline".into());
        let mut projected =
            projected_project(&prepared.source.project, &prepared.files, &prepared.name)?;
        // `ProjectStore::create` uses the requested package name, so retain it
        // in the materialized baseline as well.
        projected.name = prepared.name.clone();
        package_store.record_portable_snapshot(
            projected,
            "package.manifest.json",
            Actor::system(),
        )?;
        drop(package_store);
        // Empty derived-output roots are not cache content, but omitting them
        // keeps the published tree honest. ProjectStore::open recreates them.
        for dir in ["receipts", "proxies", "filmstrip"] {
            fs::remove_dir(package_dir.join(dir))?;
        }

        progress(0.88, "verifying portable package manifest".into());
        let manifest = package_manifest(&prepared, &package_dir)?;
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        let manifest_sha256 = format!("sha256:{:x}", Sha256::digest(&manifest_bytes));
        write_new_synced(&package_dir.join("package.manifest.json"), &manifest_bytes)?;
        verify_manifest_members(&package_dir, &manifest)?;
        sync_dir(&package_dir)?;
        sync_dir(&stage.path)?;
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        progress(0.96, "publishing portable package".into());
        before_publish(&package_dir, &prepared.target)?;
        let mut warnings = Vec::new();
        if let Err(error) = sync_dir(&prepared.destination) {
            warnings.push(format!(
                "published destination could not be directory-synced: {error}"
            ));
        }
        if let Err(error) = fs::remove_dir(&stage.path) {
            warnings.push(format!(
                "published package left private stage directory behind: {error}"
            ));
        }
        Ok(PublishedPackage {
            destination: prepared.target.to_string_lossy().into_owned(),
            manifest_sha256,
            source_revision: prepared.source.project_revision.clone(),
            file_count: manifest.totals.file_count,
            bytes: manifest.totals.bytes,
            warnings,
        })
    })();
    if result.is_err() {
        cleanup_stage(&stage.path, &prepared.name);
    }
    result
}
fn source_revision_then_publish(
    state: &AppState,
    expected_revision: &str,
    cancel: &crate::jobs::JobCancellation,
    stage: &Path,
    target: &Path,
) -> Result<(), CutError> {
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    let guard = state.project.blocking_read();
    let store = guard.as_ref().ok_or_else(|| {
        CutError::new(
            error_codes::CONFLICT,
            "portable package source project closed before publication",
            "the private stage was not published",
        )
    })?;
    let actual = store.log.current_revision()?;
    if actual.as_deref() == Some(expected_revision) {
        // Keep the read guard through no-replace publication. A source edit
        // cannot slip between the revision check and the destination commit.
        publish_new_directory(stage, target)
    } else {
        Err(CutError::new(
            error_codes::CONFLICT,
            "portable package source changed before publication",
            "the final destination was not published; run project.package_plan again",
        ))
    }
}

fn projected_project(
    source: &Project,
    files: &[PackageSourceFile],
    name: &str,
) -> Result<Project, CutError> {
    let by_asset = files
        .iter()
        .map(|file| (file.asset_id.as_str(), file))
        .collect::<BTreeMap<_, _>>();
    let mut project = source.clone();
    project.name = name.to_string();
    project
        .assets
        .retain(|asset_id, _| by_asset.contains_key(asset_id.as_str()));
    for (asset_id, asset) in &mut project.assets {
        let file = by_asset
            .get(asset_id.as_str())
            .expect("retained asset has file plan");
        asset.path = file.package_path.clone();
        asset.hash = format!("sha256:{}", file.sha256);
        asset.probe = None;
        asset.transcript = None;
        asset.perception = None;
        asset.proxy = None;
        asset.filmstrip = None;
    }
    Ok(project)
}
fn package_manifest(
    prepared: &PreparedPackage,
    package_dir: &Path,
) -> Result<PackageManifest, CutError> {
    let mut groups = BTreeMap::<String, Vec<&PackageSourceFile>>::new();
    for file in &prepared.files {
        groups
            .entry(file.package_path.clone())
            .or_default()
            .push(file);
    }
    let mut members = Vec::new();
    for (path, files) in groups {
        let first = files.first().expect("non-empty package hash group");
        members.push(PackageManifestMember {
            path,
            role: "media",
            bytes: first.bytes,
            sha256: format!("sha256:{}", first.sha256),
            asset_ids: files
                .into_iter()
                .map(|file| file.asset_id.clone())
                .collect(),
        });
    }
    for (path, role) in [
        ("ops.jsonl", "project_log"),
        ("project.json", "project_cache"),
    ] {
        let file = package_dir.join(path);
        let (bytes, sha256) = hash_path(&file)?;
        members.push(PackageManifestMember {
            path: path.into(),
            role,
            bytes,
            sha256: format!("sha256:{sha256}"),
            asset_ids: Vec::new(),
        });
    }
    members.sort_by(|left, right| left.path.cmp(&right.path));
    let bytes = members.iter().map(|member| member.bytes).sum();
    let media_file_count = members
        .iter()
        .filter(|member| member.role == "media")
        .count();
    Ok(PackageManifest {
        schema: PACKAGE_MANIFEST_SCHEMA,
        source: PackageManifestSource {
            project_identity: prepared.source.project_identity.clone(),
            project_revision: prepared.source.project_revision.clone(),
            b5_receipt_sha256: prepared.plan.b5_receipt_sha256.clone(),
        },
        policy: PackageManifestPolicy {
            only_referenced_media: true,
            derived_cache: "excluded",
            offline_media: "refuse",
            collision: "refuse",
        },
        project: PackageManifestProject {
            project_json: "project.json",
            ops_jsonl: "ops.jsonl",
            baseline_schema: PORTABLE_SNAPSHOT_SCHEMA,
        },
        totals: PackageManifestTotals {
            file_count: members.len(),
            media_file_count,
            bytes,
        },
        members,
    })
}
fn verify_manifest_members(
    package_dir: &Path,
    manifest: &PackageManifest,
    cancel: &crate::jobs::JobCancellation,
) -> Result<(), CutError> {
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    let mut expected = manifest
        .members
        .iter()
        .map(|member| member.path.clone())
        .collect::<BTreeSet<_>>();
    // The manifest deliberately does not hash itself. Its exact SHA-256 is
    // returned by the terminal job result, while this set gate prevents an
    // unlisted stage member from hitching a ride into the published package.
    expected.insert("package.manifest.json".into());
    let actual = package_regular_members(package_dir)?;
    if actual != expected {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "portable package member set does not match its manifest",
            format!(
                "expected {} members, found {}",
                expected.len(),
                actual.len()
            ),
        ));
    }
    for member in &manifest.members {
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        if !is_manifest_member_path(&member.path) {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "portable package manifest contains an unsafe member path",
                member.path.clone(),
            ));
        }
        let mut file = open_plain_regular(&package_dir.join(&member.path))?;
        let (bytes, sha256) = stream_sha256(&mut file, Some(cancel))?;
        if bytes != member.bytes || format!("sha256:{sha256}") != member.sha256 {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "portable package member failed checksum verification",
                member.path.clone(),
            ));
        }
    }
    Ok(())
}

fn package_regular_members(package_dir: &Path) -> Result<BTreeSet<String>, CutError> {
    let mut members = BTreeSet::new();
    collect_regular_members(package_dir, Path::new(""), &mut members)?;
    Ok(members)
}

fn collect_regular_members(
    root: &Path,
    relative: &Path,
    members: &mut BTreeSet<String>,
) -> Result<(), CutError> {
    ensure_plain_dir(root)?;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        if !matches!(
            Path::new(&name).components().next(),
            Some(Component::Normal(_))
        ) || Path::new(&name).components().count() != 1
        {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "portable package contains an unsafe filesystem member",
                entry.path().display().to_string(),
            ));
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        let child_relative = relative.join(name);
        if is_plain_dir(&metadata) {
            collect_regular_members(&entry.path(), &child_relative, members)?;
        } else if is_plain_regular(&metadata) {
            let member = child_relative.to_str().ok_or_else(|| {
                CutError::new(
                    error_codes::CONFLICT,
                    "portable package member name is not UTF-8",
                    entry.path().display().to_string(),
                )
            })?;
            if !members.insert(member.replace('\\', "/")) {
                return Err(CutError::new(
                    error_codes::CONFLICT,
                    "portable package has duplicate member paths",
                    member.to_string(),
                ));
            }
        } else {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "portable package contains a non-regular member",
                entry.path().display().to_string(),
            ));
        }
    }
    Ok(())
}

fn hash_path(path: &Path) -> Result<(u64, String), CutError> {
    let mut file = open_plain_regular(path)?;
    stream_sha256(&mut file, None)
}

fn is_manifest_member_path(value: &str) -> bool {
    let path = Path::new(value);
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), CutError> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    sync_dir(path.parent().unwrap_or_else(|| Path::new(".")))
}

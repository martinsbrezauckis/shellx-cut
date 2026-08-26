fn copy_source(
    file: &PackageSourceFile,
    part: &Path,
    cancel: &crate::jobs::JobCancellation,
) -> Result<(), CutError> {
    let mut input = open_plain_regular(&file.source)?;
    let before = file_stamp(&input.metadata()?);
    if before != file.stamp {
        return Err(source_changed(&file.source));
    }
    let mut output = OpenOptions::new().write(true).create_new(true).open(part)?;
    let (bytes, sha256) = stream_copy_sha256(&mut input, &mut output, cancel)?;
    output.sync_all()?;
    drop(output);
    let after = file_stamp(&input.metadata()?);
    if before != after || bytes != file.bytes || sha256 != file.sha256 {
        let _ = fs::remove_file(part);
        return Err(source_changed(&file.source));
    }
    Ok(())
}
fn stream_sha256(
    input: &mut File,
    cancel: Option<&crate::jobs::JobCancellation>,
) -> Result<(u64, String), CutError> {
    let mut digest = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        if cancel.is_some_and(crate::jobs::JobCancellation::is_cancelled) {
            return Err(cancelled());
        }
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes = bytes.saturating_add(read as u64);
        digest.update(&buffer[..read]);
    }
    Ok((bytes, format!("{:x}", digest.finalize())))
}
fn stream_copy_sha256(
    input: &mut File,
    output: &mut File,
    cancel: &crate::jobs::JobCancellation,
) -> Result<(u64, String), CutError> {
    let mut digest = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        bytes = bytes.saturating_add(read as u64);
        digest.update(&buffer[..read]);
    }
    Ok((bytes, format!("{:x}", digest.finalize())))
}

fn open_plain_regular(path: &Path) -> Result<File, CutError> {
    let metadata = fs::symlink_metadata(path)?;
    if !is_plain_regular(&metadata) {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "portable package source is not a local regular file",
            path.display().to_string(),
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    if !is_plain_regular(&file.metadata()?) {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "portable package source changed type while opening",
            path.display().to_string(),
        ));
    }
    Ok(file)
}

fn file_stamp(metadata: &fs::Metadata) -> FileStamp {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        FileStamp {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
    #[cfg(not(unix))]
    {
        FileStamp {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        }
    }
}

fn safe_extension(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| {
            !extension.is_empty()
                && extension.len() <= 16
                && extension.chars().all(|ch| ch.is_ascii_alphanumeric())
        })
        .map(|extension| extension.to_ascii_lowercase())
        .unwrap_or_else(|| "bin".into())
}

fn source_changed(path: &Path) -> CutError {
    CutError::new(
        error_codes::CONFLICT,
        "referenced media changed while portable package was running",
        path.display().to_string(),
    )
    .with_suggested_action("run project.package_plan again; no final package was published")
}

fn cancelled() -> CutError {
    CutError::new(
        "job_cancelled",
        "portable package was cancelled",
        "the final destination was not published",
    )
}

fn hash_json(value: &impl Serialize) -> Result<String, CutError> {
    let value = serde_json::to_value(value)?;
    let bytes = serde_json::to_vec(&canonical_json(value))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn canonical_json(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut pairs = object.into_iter().collect::<Vec<_>>();
            pairs.sort_by(|left, right| left.0.cmp(&right.0));
            Value::Object(
                pairs
                    .into_iter()
                    .map(|(key, value)| (key, canonical_json(value)))
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonical_json).collect()),
        other => other,
    }
}

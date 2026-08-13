use super::{run_with_atomic_output, run_with_validated_atomic_output};
use cut_core::{error_codes, CutError};

#[cfg(unix)]
fn legacy_temporary_output_path(out: &std::path::Path) -> std::path::PathBuf {
    let parent = out.parent().unwrap_or_else(|| std::path::Path::new("."));
    let stem = out
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("ffmpeg-output");
    let ext = out.extension().and_then(|s| s.to_str()).unwrap_or("tmp");
    let sequence = super::OUTPUT_SEQ.load(std::sync::atomic::Ordering::Relaxed);
    parent.join(format!(
        ".{stem}.{}.{}.tmp.{ext}",
        std::process::id(),
        sequence
    ))
}

fn args_for(out: &std::path::Path) -> Vec<String> {
    vec!["-i".into(), "input.mov".into(), out.display().to_string()]
}

#[cfg(unix)]
#[test]
fn precreated_legacy_temp_symlink_never_overwrites_its_sentinel() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("final.mp4");
    let sentinel = dir.path().join("sentinel.txt");
    std::fs::write(&sentinel, b"keep me").unwrap();

    let predicted = legacy_temporary_output_path(&out);
    std::os::unix::fs::symlink(&sentinel, &predicted).unwrap();

    run_with_atomic_output(&args_for(&out), &out, |tmp_args| {
        std::fs::write(tmp_args.last().unwrap(), b"render bytes").unwrap();
        Ok(())
    })
    .unwrap();

    assert_eq!(
        std::fs::read(&sentinel).unwrap(),
        b"keep me",
        "the ffmpeg output path must never follow a precreated temp symlink"
    );
    assert_eq!(std::fs::read(&out).unwrap(), b"render bytes");
    assert!(
        !std::fs::symlink_metadata(&out)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the final output must be a regular file, not the attacker link"
    );
}

#[cfg(unix)]
#[test]
fn group_or_world_writable_output_parent_is_refused_before_reservation() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let output_dir = root.path().join("shared-output");
    std::fs::create_dir(&output_dir).unwrap();
    std::fs::set_permissions(&output_dir, std::fs::Permissions::from_mode(0o777)).unwrap();
    let out = output_dir.join("final.mp4");

    let error = run_with_atomic_output(&args_for(&out), &out, |_tmp_args| Ok(())).unwrap_err();
    assert_eq!(error.code, error_codes::IO);
    assert!(error.message.contains("writable by group or other"));
}

#[cfg(unix)]
#[test]
fn owned_project_output_parent_is_hardened_before_reservation() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("edit.cutproj");
    let output_dir = project.join("exports");
    std::fs::create_dir_all(&output_dir).unwrap();
    std::fs::set_permissions(&output_dir, std::fs::Permissions::from_mode(0o775)).unwrap();
    let out = output_dir.join("final.mp4");

    run_with_atomic_output(&args_for(&out), &out, |tmp_args| {
        std::fs::write(tmp_args.last().unwrap(), b"render bytes").unwrap();
        Ok(())
    })
    .unwrap();

    assert_eq!(std::fs::read(&out).unwrap(), b"render bytes");
    assert_eq!(
        std::fs::symlink_metadata(&output_dir).unwrap().mode() & 0o022,
        0,
        "Cut-owned project output directories must be protected before ffmpeg starts",
    );
}

#[test]
fn failed_writer_removes_its_temp_and_a_stale_zero_byte_final_output() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("final.mp4");
    std::fs::write(&out, []).unwrap();

    let error = run_with_atomic_output(&args_for(&out), &out, |tmp_args| {
        let tmp = std::path::Path::new(tmp_args.last().unwrap());
        assert!(
            !out.exists(),
            "stale empty final output must be cleared first"
        );
        std::fs::write(tmp, []).unwrap();
        Err::<(), CutError>(CutError::new(
            error_codes::FFMPEG,
            "encode failed",
            "test failure",
        ))
    })
    .unwrap_err();

    assert_eq!(error.code, error_codes::FFMPEG);
    assert!(
        !out.exists(),
        "a failed render cannot leave an empty final file"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn failed_writer_keeps_an_existing_nonempty_final_output() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("final.mp4");
    std::fs::write(&out, b"known-good").unwrap();

    let _ = run_with_atomic_output(&args_for(&out), &out, |tmp_args| {
        std::fs::write(tmp_args.last().unwrap(), b"partial").unwrap();
        Err::<(), CutError>(CutError::new(
            error_codes::FFMPEG,
            "encode failed",
            "test failure",
        ))
    });

    assert_eq!(std::fs::read(&out).unwrap(), b"known-good");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn successful_writer_publishes_its_temp_at_the_final_path() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("final.mp4");

    let result = run_with_atomic_output(&args_for(&out), &out, |tmp_args| {
        let tmp = std::path::Path::new(tmp_args.last().unwrap());
        let metadata = std::fs::symlink_metadata(tmp).unwrap();
        assert!(metadata.file_type().is_file());
        assert_eq!(metadata.len(), 0, "the writer receives its reservation");
        assert_eq!(tmp.parent(), out.parent(), "temp stays beside final output");
        std::fs::write(tmp, b"complete").unwrap();
        Ok("finished")
    })
    .unwrap();

    assert_eq!(result, "finished");
    assert_eq!(std::fs::read(&out).unwrap(), b"complete");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn second_successful_writer_replaces_the_prior_final_output() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("final.mp4");
    std::fs::write(&out, b"old-render").unwrap();

    run_with_atomic_output(&args_for(&out), &out, |tmp_args| {
        std::fs::write(tmp_args.last().unwrap(), b"new-render").unwrap();
        Ok(())
    })
    .unwrap();

    assert_eq!(std::fs::read(&out).unwrap(), b"new-render");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn publication_failure_at_an_existing_destination_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("final.mp4");
    std::fs::create_dir(&out).unwrap();

    let error = run_with_atomic_output(&args_for(&out), &out, |tmp_args| {
        std::fs::write(tmp_args.last().unwrap(), b"new-render").unwrap();
        Ok(())
    })
    .unwrap_err();

    assert_eq!(error.code, error_codes::IO);
    assert!(out.is_dir());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn zero_byte_success_is_rejected_before_it_can_be_published() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("final.mp4");

    let error = run_with_atomic_output(&args_for(&out), &out, |tmp_args| {
        std::fs::write(tmp_args.last().unwrap(), []).unwrap();
        Ok(())
    })
    .unwrap_err();

    assert_eq!(error.code, error_codes::FFMPEG);
    assert!(!out.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn failed_validation_removes_temp_without_publishing_it() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("final.mp4");

    let error = run_with_validated_atomic_output(
        &args_for(&out),
        &out,
        |tmp_args| {
            std::fs::write(tmp_args.last().unwrap(), b"not a media container").unwrap();
            Ok(())
        },
        |_| {
            Err::<(), CutError>(CutError::new(
                error_codes::FFMPEG,
                "output validation failed",
                "simulated ffprobe rejection",
            ))
        },
    )
    .unwrap_err();

    assert_eq!(error.code, error_codes::FFMPEG);
    assert!(
        !out.exists(),
        "an invalid completed temp must not be published"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[cfg(windows)]
#[test]
fn reparse_output_parent_is_refused_before_reservation() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    let reparse = root.path().join("reparse");
    std::fs::create_dir(&target).unwrap();
    if std::os::windows::fs::symlink_dir(&target, &reparse).is_err() {
        eprintln!("skipping: Windows symlink creation privilege is unavailable");
        return;
    }

    let out = reparse.join("final.mp4");
    let error = run_with_atomic_output(&args_for(&out), &out, |_tmp_args| Ok(())).unwrap_err();
    assert_eq!(error.code, error_codes::IO);
    assert!(error.message.contains("plain directory"));
}

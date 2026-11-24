// The folder rule is about permission bits, so its test and this import are Unix-only.
#[cfg(unix)]
use super::check_output_dir_policy;
use super::{private_test_tempdir, run_with_atomic_output, run_with_validated_atomic_output};
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
    let dir = private_test_tempdir();
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
fn an_output_parent_everyone_can_write_is_refused_unless_sticky_and_the_group_may_write() {
    use std::os::unix::fs::PermissionsExt;

    let root = private_test_tempdir();
    let output_dir = root.path().join("shared-output");
    std::fs::create_dir(&output_dir).unwrap();
    let out = output_dir.join("final.mp4");
    let mode = |bits| {
        std::fs::set_permissions(&output_dir, std::fs::Permissions::from_mode(bits)).unwrap()
    };

    mode(0o777);
    let error = run_with_atomic_output(&args_for(&out), &out, |_tmp_args| Ok(())).unwrap_err();
    assert_eq!(error.code, error_codes::IO);
    assert!(error
        .message
        .contains("can be written by anyone on this computer"));
    assert!(check_output_dir_policy(&output_dir).is_err());

    // The sticky bit (a /tmp-like folder) protects users' files from each other.
    mode(0o1777);
    assert!(check_output_dir_policy(&output_dir).is_ok());
    // A folder only the group can write to is allowed, and a render publishes there.
    mode(0o775);
    assert!(check_output_dir_policy(&output_dir).is_ok());
    run_with_atomic_output(&args_for(&out), &out, |tmp_args| {
        let tmp = std::path::Path::new(tmp_args.last().unwrap());
        assert_eq!(
            std::fs::symlink_metadata(tmp).unwrap().permissions().mode() & 0o777,
            0o600,
            "the staged file is owner-only"
        );
        std::fs::write(tmp, b"render bytes").unwrap();
        Ok(())
    })
    .unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"render bytes");
}

/// In a folder the group can write to, a file a group member swaps in for the
/// staged one after it was reserved is never published.
#[cfg(unix)]
#[test]
fn a_staged_file_swapped_by_another_user_is_not_published() {
    use std::os::unix::fs::PermissionsExt;

    let root = private_test_tempdir();
    let output_dir = root.path().join("team-output");
    std::fs::create_dir(&output_dir).unwrap();
    std::fs::set_permissions(&output_dir, std::fs::Permissions::from_mode(0o775)).unwrap();
    let out = output_dir.join("final.mp4");

    let error = run_with_atomic_output(&args_for(&out), &out, |tmp_args| {
        let tmp = std::path::Path::new(tmp_args.last().unwrap());
        // The swap: remove the reservation and put another regular file there.
        std::fs::remove_file(tmp).unwrap();
        std::fs::write(tmp, b"attacker bytes").unwrap();
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.code, error_codes::FFMPEG);
    assert_eq!(
        error.message,
        "the staged output was replaced while the render ran"
    );
    assert!(!out.exists(), "a swapped file is never published");
}

#[cfg(unix)]
#[test]
fn owned_project_output_parent_is_hardened_before_reservation() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let root = private_test_tempdir();
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
    let dir = private_test_tempdir();
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
    let dir = private_test_tempdir();
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
    let dir = private_test_tempdir();
    let out = dir.path().join("final.mp4");

    let result = run_with_atomic_output(&args_for(&out), &out, |tmp_args| {
        let tmp = std::path::Path::new(tmp_args.last().unwrap());
        let metadata = std::fs::symlink_metadata(tmp).unwrap();
        assert!(metadata.file_type().is_file());
        assert_eq!(metadata.len(), 0, "the writer receives its reservation");
        assert!(
            !tmp.starts_with(out.parent().unwrap()),
            "temp stays out of the export folder"
        );
        assert!(
            tmp.starts_with(super::export_staging_root().unwrap()),
            "temp stays inside Cut's private export staging root"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let stage_mode = std::fs::metadata(tmp.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(stage_mode, 0o700, "staging directory must be mode 0700");
        }
        std::fs::write(tmp, b"complete").unwrap();
        Ok("finished")
    })
    .unwrap();

    assert_eq!(result, "finished");
    assert_eq!(std::fs::read(&out).unwrap(), b"complete");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn cross_volume_fallback_copies_fsyncs_and_renames_without_following_target_link() {
    let dir = private_test_tempdir();
    let out = dir.path().join("crossvol.mp4");
    let victim = dir.path().join("victim.mp4");
    std::fs::write(&victim, b"keep-victim").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&victim, &out).unwrap();
    #[cfg(windows)]
    if std::os::windows::fs::symlink_file(&victim, &out).is_err() {
        // Without developer-mode symlink rights on Windows, verify normal target replacement.
        std::fs::write(&out, b"stale").unwrap();
    }

    let dest_dir = super::PinnedExportDir::open(dir.path()).unwrap();
    let stage = super::PrivateStageDir::new().unwrap();
    let staged = stage.reserve_file_for(&out).unwrap();
    std::fs::write(&staged.path, b"fresh-cross-volume-payload").unwrap();
    let (file, meta) =
        super::publish::verify_staged_file(&staged.path, Some(&staged.identity), false).unwrap();
    super::publish::publish_verified_stage(
        Some(&stage),
        Some(std::ffi::OsStr::new(&staged.leaf_name)),
        &staged.path,
        file,
        meta.len(),
        &dest_dir,
        std::ffi::OsStr::new("crossvol.mp4"),
        super::publish::PublishMode::ForceCrossVolume,
    )
    .unwrap();

    assert_eq!(std::fs::read(&victim).unwrap(), b"keep-victim");
    assert_eq!(std::fs::read(&out).unwrap(), b"fresh-cross-volume-payload");
    assert!(
        !std::fs::symlink_metadata(&out)
            .unwrap()
            .file_type()
            .is_symlink(),
        "published target must be a regular file, not the replaced symlink"
    );
    assert!(!staged.path.exists(), "staged copy removed after publish");
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(
        entries.len(),
        2,
        "no temp leftovers beside target: {entries:?}"
    );
}

#[test]
fn second_successful_writer_replaces_the_prior_final_output() {
    let dir = private_test_tempdir();
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
    let dir = private_test_tempdir();
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
    let dir = private_test_tempdir();
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
    let dir = private_test_tempdir();
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
    let root = private_test_tempdir();
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

// `/proc/self` is a link that root owns, two folders down: it stands for a
// link another user put above an export folder, which is refused, while a link
// the current user made there is followed to the real folder.
#[cfg(target_os = "linux")]
#[test]
fn a_link_owned_by_another_user_above_the_folder_is_refused_and_an_own_link_is_followed() {
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skipping: as root every link is the current user's own");
        return;
    }
    let error = match super::PinnedExportDir::open(std::path::Path::new("/proc/self/fd")) {
        Ok(_) => panic!("a link owned by another user above the folder must be refused"),
        Err(error) => error,
    };
    assert_eq!(error.code, error_codes::IO);
    assert!(error.message.contains("another user"), "{}", error.message);
    assert!(error.suggested_action.is_some());

    let root = private_test_tempdir();
    let real = root.path().join("real");
    std::fs::create_dir_all(real.join("exports")).unwrap();
    let own_link = root.path().join("own-link");
    std::os::unix::fs::symlink(&real, &own_link).unwrap();
    let out = own_link.join("exports").join("final.mp4");
    run_with_atomic_output(&args_for(&out), &out, |tmp_args| {
        std::fs::write(tmp_args.last().unwrap(), b"rendered")?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        std::fs::read(real.join("exports").join("final.mp4")).unwrap(),
        b"rendered"
    );
    assert_eq!(std::fs::read_dir(real.join("exports")).unwrap().count(), 1);
}

struct ChildGuard(Option<std::process::Child>);

impl ChildGuard {
    fn spawn_sleep() -> Self {
        let child = if cfg!(windows) {
            std::process::Command::new("ping")
                .args(["-n", "60", "127.0.0.1"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap()
        } else {
            std::process::Command::new("sleep")
                .arg("60")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap()
        };
        Self(Some(child))
    }

    fn pid(&self) -> u32 {
        self.0.as_ref().unwrap().id()
    }

    fn kill_and_wait(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.kill_and_wait();
    }
}

#[test]
fn stale_staging_folder_of_a_live_process_is_kept_until_it_exits() {
    let root = private_test_tempdir();
    let mut child = ChildGuard::spawn_sleep();
    let live_stage = root.path().join(format!("stage-{}-abc", child.pid()));
    std::fs::create_dir_all(&live_stage).unwrap();
    std::fs::write(live_stage.join("in_flight.mp4"), b"writing").unwrap();

    super::staging::sweep_stale_export_staging_in(root.path(), std::time::Duration::ZERO);
    assert!(
        live_stage.is_dir(),
        "staging folder of a live engine pid must be kept even past max_age"
    );

    child.kill_and_wait();
    super::staging::sweep_stale_export_staging_in(root.path(), std::time::Duration::ZERO);
    assert!(
        !live_stage.exists(),
        "staging folder must be swept after its process exits"
    );
}

#[test]
fn killed_exports_staging_goes_at_once_but_pidless_entries_keep_the_age_rule() {
    let root = private_test_tempdir();
    let mut child = ChildGuard::spawn_sleep();
    let dead_pid = child.pid();
    child.kill_and_wait();
    let twelve_h = std::time::Duration::from_secs(12 * 3600);
    let dead = root.path().join(format!("stage-{dead_pid}-abc"));
    let own = root
        .path()
        .join(format!("stage-{}-own", std::process::id()));
    let pidless = root.path().join("stage-x-abc");
    for d in [&dead, &own, &pidless] {
        std::fs::create_dir_all(d).unwrap();
        std::fs::write(d.join("partial.mp4"), b"x").unwrap();
    }
    super::staging::sweep_stale_export_staging_in(root.path(), twelve_h);
    assert!(!dead.exists(), "dead pid's fresh staging must go at once");
    assert!(own.is_dir(), "own staging must stay");
    assert!(pidless.is_dir(), "entry without a pid keeps the 12 h rule");
}

#[test]
fn process_is_running_distinguishes_live_and_exited_children() {
    assert!(!super::process_is_running(0));
    assert!(!super::process_is_running(u32::MAX));
    assert!(super::process_is_running(std::process::id()));
    let mut child = ChildGuard::spawn_sleep();
    let pid = child.pid();
    assert!(super::process_is_running(pid));
    child.kill_and_wait();
    assert!(!super::process_is_running(pid));
}

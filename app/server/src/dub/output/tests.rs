use super::*;

#[test]
fn deterministic_outputs_replace_plain_files_and_failed_stages_preserve_them() {
    let project = tempfile::tempdir().unwrap();
    for lang in ["lv", "pt-br", "latin american spanish", "latviešu"] {
        let outputs = Outputs::new(project.path(), "a1", lang).unwrap();
        assert_eq!(
            outputs.wav(),
            project.path().join(format!("dub/a1.{lang}.wav"))
        );
        std::fs::write(outputs.wav(), b"old WAV").unwrap();
        let stage_path = {
            let stage = outputs.stage_wav().unwrap();
            std::fs::write(stage.path(), b"partial").unwrap();
            stage.path().to_owned()
        };
        assert!(!stage_path.exists());
        assert_eq!(std::fs::read(outputs.wav()).unwrap(), b"old WAV");
        let empty = outputs.stage_wav().unwrap();
        std::fs::write(empty.path(), []).unwrap();
        assert!(empty.publish().is_err());
        assert_eq!(std::fs::read(outputs.wav()).unwrap(), b"old WAV");
        let stage = outputs.stage_wav().unwrap();
        std::fs::write(stage.path(), b"new WAV").unwrap();
        stage.publish().unwrap();
        assert_eq!(std::fs::read(outputs.wav()).unwrap(), b"new WAV");
        outputs.write_receipt(b"old receipt").unwrap();
        outputs.write_receipt(b"new receipt").unwrap();
        assert_eq!(std::fs::read(&outputs.receipt).unwrap(), b"new receipt");
    }
    assert!(std::fs::read_dir(project.path().join("dub"))
        .unwrap()
        .all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".cut-dub-")));
}

#[test]
fn path_forming_languages_are_refused_before_directory_creation() {
    for lang in ["../lv", "lv/other", "lv\\other", "lv:stream", "..", "lv\n"] {
        let project = tempfile::tempdir().unwrap();
        assert!(
            Outputs::new(project.path(), "a1", lang).is_err(),
            "{lang:?}"
        );
        assert_eq!(std::fs::read_dir(project.path()).unwrap().count(), 0);
    }
}

#[test]
fn hardlinked_and_directory_final_leaves_never_mutate_the_external_inode() {
    for relative in ["dub/a1.lv.wav", "receipts/a1.lv.dub.json"] {
        let project = tempfile::tempdir().unwrap();
        let outputs = Outputs::new(project.path(), "a1", "lv").unwrap();
        let outside = project.path().join("outside");
        std::fs::write(&outside, b"sentinel").unwrap();
        let leaf = project.path().join(relative);
        std::fs::hard_link(&outside, &leaf).unwrap();
        assert!(Outputs::new(project.path(), "a1", "lv").is_err());
        if relative.starts_with("dub/") {
            assert!(outputs.stage_wav().is_err());
        } else {
            assert!(outputs.write_receipt(b"new").is_err());
        }
        assert_eq!(std::fs::read(&outside).unwrap(), b"sentinel");
        std::fs::remove_file(&leaf).unwrap();
        std::fs::create_dir(&leaf).unwrap();
        assert!(Outputs::new(project.path(), "a1", "lv").is_err());
    }
}

#[cfg(unix)]
#[test]
fn linked_roots_and_leaves_including_dangling_links_are_refused() {
    for relative in [
        "dub",
        "receipts",
        "dub/a1.lv.wav",
        "receipts/a1.lv.dub.json",
    ] {
        for dangling in [false, true] {
            let project = tempfile::tempdir().unwrap();
            let outside_root = tempfile::tempdir().unwrap();
            let outside = outside_root.path().join("target");
            if !dangling {
                if relative.contains('/') {
                    std::fs::write(&outside, b"sentinel").unwrap();
                } else {
                    std::fs::create_dir(&outside).unwrap();
                }
            }
            let leaf = project.path().join(relative);
            std::fs::create_dir_all(leaf.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&outside, leaf).unwrap();
            assert!(Outputs::new(project.path(), "a1", "lv").is_err());
            if dangling {
                assert!(!outside.exists());
            } else if relative.contains('/') {
                assert_eq!(std::fs::read(&outside).unwrap(), b"sentinel");
            } else {
                assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn publication_rechecks_leaf_and_preserves_existing_directory_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("dub")).unwrap();
    std::fs::set_permissions(
        project.path().join("dub"),
        std::fs::Permissions::from_mode(0o770),
    )
    .unwrap();
    let outputs = Outputs::new(project.path(), "a1", "lv").unwrap();
    let stage = outputs.stage_wav().unwrap();
    let stage_path = stage.path().to_owned();
    std::fs::write(stage.path(), b"new WAV").unwrap();
    let outside = project.path().join("outside");
    std::fs::write(&outside, b"sentinel").unwrap();
    std::os::unix::fs::symlink(&outside, outputs.wav()).unwrap();
    assert!(stage.publish().is_err());
    assert!(!stage_path.exists());
    assert_eq!(std::fs::read(outside).unwrap(), b"sentinel");
    assert_eq!(
        std::fs::metadata(project.path().join("dub"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o770
    );
}

#[cfg(unix)]
#[tokio::test]
async fn owned_worker_failure_and_deadline_leave_previous_wav_and_clean_stage() {
    for deadline in [false, true] {
        let project = tempfile::tempdir().unwrap();
        let outputs = Outputs::new(project.path(), "a1", "lv").unwrap();
        std::fs::write(outputs.wav(), b"previous WAV").unwrap();
        let script = project.path().join("stub.sh");
        std::fs::write(&script, if deadline { "sleep 2\n" } else { "exit 1\n" }).unwrap();
        let runtime = crate::dub::Runtime {
            // Bash accepts the existing sidecar -B flag; dash does not.
            python: "/bin/bash".into(),
            script,
            native_context: false,
        };
        let stage_path = {
            let stage = outputs.stage_wav().unwrap();
            let path = stage.path().to_owned();
            let result = crate::dub::synthesize_track(
                &runtime,
                "unused",
                "fixture",
                Some("lv"),
                None,
                24000,
                stage.path(),
                &[],
                std::time::Duration::from_millis(50),
            )
            .await;
            assert!(result.is_err());
            if deadline {
                assert!(result.unwrap_err().message.contains("timed out"));
            }
            path
        };
        assert!(!stage_path.exists());
        assert_eq!(std::fs::read(outputs.wav()).unwrap(), b"previous WAV");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn managed_cancellation_cleans_stage_and_preserves_previous_wav() {
    let project = tempfile::tempdir().unwrap();
    let outputs = Outputs::new(project.path(), "a1", "lv").unwrap();
    std::fs::write(outputs.wav(), b"previous WAV").unwrap();
    let script = project.path().join("cancel-stub.sh");
    std::fs::write(&script, "printf ready > \"$0.ready\"\nsleep 5\n").unwrap();
    let marker = PathBuf::from(format!("{}.ready", script.display()));
    let jobs = crate::jobs::JobManager::new(crate::events::EventBus::new());
    let job = jobs.create("dub-output-test");
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let project_path = project.path().to_owned();
    jobs.spawn(&job.job_id, async move {
        let outputs = Outputs::new(&project_path, "a1", "lv").unwrap();
        let runtime = crate::dub::Runtime {
            python: "/bin/bash".into(),
            script,
            native_context: false,
        };
        let (stage_path, result) = {
            let stage = outputs.stage_wav().unwrap();
            let stage_path = stage.path().to_owned();
            let result = crate::dub::synthesize_track(
                &runtime,
                "unused",
                "fixture",
                Some("lv"),
                None,
                24000,
                stage.path(),
                &[],
                std::time::Duration::from_secs(10),
            )
            .await;
            (stage_path, result)
        };
        let _ = sender.send((stage_path, result));
    });
    for _ in 0..100 {
        if marker.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(marker.exists(), "owned stub did not start");
    assert!(jobs.abort(&job.job_id).await.unwrap());
    let (stage_path, result) = receiver.await.unwrap();
    assert_eq!(result.unwrap_err().code, "job_cancelled");
    assert!(!stage_path.exists());
    assert_eq!(std::fs::read(outputs.wav()).unwrap(), b"previous WAV");
}

#[cfg(target_os = "linux")]
mod linux {
    use std::fs;

    use crate::camera_finalization::{
        finalize_after_close_for_test, finalize_after_close_with_sync_for_test,
        finalize_staged_camera_media, CameraNativeCloser,
    };
    use crate::camera_finalization_test_support::{
        facts, nameless_stage, owner, staged, FixtureCloser, FixtureProbe,
    };

    #[test]
    fn finalized_closed_media_has_no_staging_alias_and_publishes_one_opaque_seal() {
        let temp = tempfile::tempdir().unwrap();
        let staged = staged(temp.path());
        let owner = owner(temp.path());
        let probe = FixtureProbe::new(facts());
        let mut closer = FixtureCloser::with_stage(temp.path());

        let seal = finalize_staged_camera_media(&mut closer, &probe, &owner, &staged).unwrap();

        assert!(closer.closed);
        assert_eq!(probe.calls.get(), 6);
        assert_eq!(seal.bytes(), b"closed camera media".len() as u64);
        assert_eq!(seal.artifact_id(), "camera_01");
        assert_eq!(seal.video(), "camera/camera.mp4");
        assert_eq!(seal.media().duration_ms, 1_000);
        assert!(fs::read_dir(temp.path().join(".camera-staging"))
            .unwrap()
            .next()
            .is_none());
        assert_eq!(
            fs::read(temp.path().join("camera/camera.mp4")).unwrap(),
            b"closed camera media"
        );
    }

    #[test]
    fn missing_native_close_fails_before_any_probe_or_publication() {
        let temp = tempfile::tempdir().unwrap();
        let staged = staged(temp.path());
        let owner = owner(temp.path());
        let probe = FixtureProbe::new(facts());
        let mut closer = FixtureCloser::failing();

        let error = finalize_staged_camera_media(&mut closer, &probe, &owner, &staged).unwrap_err();

        assert_eq!(error.message, "native camera writer did not close");
        assert!(closer.closed);
        assert_eq!(probe.calls.get(), 0);
        assert!(!temp.path().join("camera/camera.mp4").exists());
    }

    #[test]
    fn false_declared_media_facts_fail_closed_without_publication() {
        let mut wrong_dimensions = crate::camera_finalization_test_support::declaration();
        wrong_dimensions.width = 1280;
        let mut wrong_fps = crate::camera_finalization_test_support::declaration();
        wrong_fps.fps_num = 24;
        let mut wrong_frame_count = crate::camera_finalization_test_support::declaration();
        wrong_frame_count.frame_count = 31;
        let mut wrong_duration = crate::camera_finalization_test_support::declaration();
        wrong_duration.duration_ms = 1_001;

        for (declared, expected_message) in [
            (wrong_dimensions, "dimensions do not match"),
            (wrong_fps, "FPS does not match"),
            (wrong_frame_count, "frame count does not match"),
            (wrong_duration, "duration does not match"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let mut staged = staged(temp.path());
            staged.declared = declared;
            let owner = owner(temp.path());
            let probe = FixtureProbe::new(facts());
            let mut closer = FixtureCloser::with_stage(temp.path());

            let error =
                finalize_staged_camera_media(&mut closer, &probe, &owner, &staged).unwrap_err();

            assert!(error.message.contains(expected_message));
            assert!(!temp.path().join("camera/camera.mp4").exists());
        }
    }

    #[test]
    fn existing_destination_is_a_no_replace_collision_and_is_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let staged = staged(temp.path());
        let owner = owner(temp.path());
        let destination = temp.path().join("camera/camera.mp4");
        fs::write(&destination, b"existing recording").unwrap();
        let probe = FixtureProbe::new(facts());
        let mut closer = FixtureCloser::with_stage(temp.path());

        let error = finalize_staged_camera_media(&mut closer, &probe, &owner, &staged).unwrap_err();

        assert!(error.message.contains("already exists"));
        assert_eq!(fs::read(destination).unwrap(), b"existing recording");
    }

    #[test]
    fn absolute_or_parent_destination_paths_are_refused_before_probe() {
        for video in ["../outside.mp4", "/tmp/outside.mp4"] {
            let temp = tempfile::tempdir().unwrap();
            let mut staged = staged(temp.path());
            staged.video = video.into();
            let owner = owner(temp.path());
            let probe = FixtureProbe::new(facts());
            let mut closer = FixtureCloser::with_stage(temp.path());
            let error =
                finalize_staged_camera_media(&mut closer, &probe, &owner, &staged).unwrap_err();
            assert!(error.message.contains("escapes"));
            assert_eq!(probe.calls.get(), 0);
        }
    }

    #[test]
    fn stage_with_a_name_is_refused_before_probe() {
        struct NamedCloser(Option<std::fs::File>);
        unsafe impl CameraNativeCloser for NamedCloser {
            fn close_media(&mut self) -> record_core::Result<std::fs::File> {
                Ok(self.0.take().unwrap())
            }
        }

        let temp = tempfile::tempdir().unwrap();
        let staged = staged(temp.path());
        let owner = owner(temp.path());
        let named = temp.path().join(".camera-staging/named.mp4");
        fs::write(&named, b"named stage").unwrap();
        let mut closer = NamedCloser(Some(std::fs::File::open(&named).unwrap()));
        let probe = FixtureProbe::new(facts());

        let error = finalize_staged_camera_media(&mut closer, &probe, &owner, &staged).unwrap_err();

        assert!(error.message.contains("mutable pathname"));
        assert_eq!(probe.calls.get(), 0);
        assert_eq!(fs::read(named).unwrap(), b"named stage");
    }

    #[test]
    fn writable_stage_descriptor_is_refused_before_probe_or_publication() {
        let temp = tempfile::tempdir().unwrap();
        let staged = staged(temp.path());
        let owner = owner(temp.path());
        let (read_only, writable) = nameless_stage(temp.path());
        drop(read_only);
        let probe = FixtureProbe::new(facts());

        let error = finalize_after_close_for_test(
            &probe,
            &owner,
            &staged,
            writable,
            || Ok(()),
            || Ok(()),
            || Ok(()),
            || Ok(()),
        )
        .unwrap_err();

        assert!(error.message.contains("descriptor is writable"));
        assert_eq!(probe.calls.get(), 0);
        assert!(!temp.path().join("camera/camera.mp4").exists());
    }

    #[test]
    fn published_file_sync_failure_cannot_return_a_seal() {
        let temp = tempfile::tempdir().unwrap();
        let staged = staged(temp.path());
        let owner = owner(temp.path());
        let (stage, _) = nameless_stage(temp.path());
        let probe = FixtureProbe::new(facts());
        let destination = temp.path().join("camera/camera.mp4");

        let error = finalize_after_close_with_sync_for_test(
            &probe,
            &owner,
            &staged,
            stage,
            || Ok(()),
            || Ok(()),
            || Ok(()),
            || Ok(()),
            |_| {
                Err(record_core::RecordError::new(
                    "capture",
                    "injected published file sync failure",
                    "test durability failure",
                ))
            },
            |_| panic!("directory sync must not run after file sync failure"),
        )
        .unwrap_err();

        assert_eq!(error.message, "injected published file sync failure");
        assert_eq!(fs::read(destination).unwrap(), b"closed camera media");
    }

    #[test]
    fn publication_directory_sync_failure_cannot_return_a_seal() {
        let temp = tempfile::tempdir().unwrap();
        let staged = staged(temp.path());
        let owner = owner(temp.path());
        let (stage, _) = nameless_stage(temp.path());
        let probe = FixtureProbe::new(facts());
        let destination = temp.path().join("camera/camera.mp4");
        let file_sync_called = std::cell::Cell::new(false);

        let error = finalize_after_close_with_sync_for_test(
            &probe,
            &owner,
            &staged,
            stage,
            || Ok(()),
            || Ok(()),
            || Ok(()),
            || Ok(()),
            |_| {
                file_sync_called.set(true);
                Ok(())
            },
            |_| {
                assert!(file_sync_called.get());
                Err(record_core::RecordError::new(
                    "capture",
                    "injected publication directory sync failure",
                    "test durability failure",
                ))
            },
        )
        .unwrap_err();

        assert_eq!(error.message, "injected publication directory sync failure");
        assert!(file_sync_called.get());
        assert_eq!(fs::read(destination).unwrap(), b"closed camera media");
    }
}

#[test]
fn production_seal_and_publication_are_source_private_and_anchored() {
    let finalizer = include_str!("camera_finalization.rs");
    let seal = include_str!("camera_finalization_seal.rs");
    let anchored = include_str!("camera_finalization_anchored.rs");
    let identity = include_str!("camera_finalization_identity.rs");
    let owner = include_str!("camera_finalization_owner.rs");
    let publication = include_str!("camera_finalization_publication.rs");
    let durability = include_str!("camera_finalization_durability.rs");
    let test_seam = include_str!("camera_finalization_test_seam.rs");

    assert!(seal.contains("pub(super) fn from_finalizer"));
    assert!(!seal.contains("pub(crate) fn from_finalizer"));
    assert!(finalizer.contains("#[cfg(test)]\npub(super) use test_seam"));
    assert!(test_seam.contains("pub(crate) fn finalize_after_close_for_test"));
    assert!(test_seam.contains("pub(crate) fn finalize_after_close_with_sync_for_test"));
    assert!(test_seam.contains("pub(crate) fn finalize_after_close_with_post_sync_for_test"));
    assert!(finalizer.contains("unsafe trait CameraNativeCloser"));
    assert!(finalizer.contains("paths.revalidate_owner()?;"));
    assert!(finalizer.contains("sync_file(durable_publication.file().file())?"));
    assert!(finalizer.contains("sync_directory(post_file_sync.destination_parent())?"));
    assert!(finalizer.contains("let post_directory_sync = verify_published"));
    assert!(finalizer.contains("equal-privilege process will"));
    assert!(finalizer.contains("trusted native\n/// closer and capture-directory owner"));
    assert!(!finalizer.contains("capture_dir: &Path"));
    assert!(!finalizer.contains("pub(crate) staged_file"));
    assert!(anchored.contains("libc::openat"));
    assert!(anchored.contains("libc::linkat"));
    assert!(anchored.contains("libc::F_GETFL"));
    assert!(anchored.contains("libc::fchmod"));
    assert!(anchored.contains("/proc/self/fd/"));
    assert!(identity.contains("has_windows_reparse_attributes"));
    assert!(owner.contains("parent: AnchoredDirectory"));
    assert!(owner.contains("capture_dir(&self)"));
    assert!(publication.contains("VerifiedCameraPublication"));
    assert!(durability.contains("sync_published_file"));
    assert!(durability.contains("sync_publication_directory"));
    assert!(finalizer.contains("post-handoff protection or absolute no-TOCTOU"));
    assert!(!anchored.contains("canonicalize("));
    assert!(!publication.contains("remove_file"));
    assert!(!publication.contains("unlinkat"));
}

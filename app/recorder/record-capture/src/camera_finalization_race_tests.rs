#[cfg(target_os = "linux")]
mod linux {
    use std::fs;
    use std::path::Path;

    use crate::camera_finalization::finalize_after_close_for_test;
    use crate::camera_finalization_test_support::{
        facts, nameless_stage, owner, rewrite_stage, staged, FixtureProbe,
    };

    #[test]
    fn stage_bytes_changed_after_hash_fail_before_any_publication() {
        let temp = tempfile::tempdir().unwrap();
        let staged = staged(temp.path());
        let owner = owner(temp.path());
        let (stage, mut writer) = nameless_stage(temp.path());
        let probe = FixtureProbe::new(facts());

        let error = finalize_after_close_for_test(
            &probe,
            &owner,
            &staged,
            stage,
            || Ok(()),
            || {
                rewrite_stage(&mut writer);
                Ok(())
            },
            || Ok(()),
            || Ok(()),
        )
        .unwrap_err();

        assert!(error.message.contains("changed after hashing"));
        assert!(!temp.path().join("camera/camera.mp4").exists());
    }

    #[test]
    fn writable_alias_change_after_publication_rejects_same_inode_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let staged = staged(temp.path());
        let owner = owner(temp.path());
        let (stage, mut writer) = nameless_stage(temp.path());
        let probe = FixtureProbe::new(facts());
        let destination = temp.path().join("camera/camera.mp4");

        let error = finalize_after_close_for_test(
            &probe,
            &owner,
            &staged,
            stage,
            || Ok(()),
            || Ok(()),
            || Ok(()),
            || {
                rewrite_stage(&mut writer);
                Ok(())
            },
        )
        .unwrap_err();

        assert!(error.message.contains("published media bytes changed"));
        assert_eq!(fs::read(destination).unwrap(), b"mutated camera data");
    }

    #[test]
    fn root_swap_before_finalizer_entry_fails_from_the_reserved_owner() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let staged = staged(&root);
        let owner = owner(&root);
        let (stage, _) = nameless_stage(&root);
        let probe = FixtureProbe::new(facts());
        let displaced = root.with_extension("camera-owner-displaced");

        swap_capture_root(&root, &displaced);

        let error = finalize_after_close_for_test(
            &probe,
            &owner,
            &staged,
            stage,
            || Ok(()),
            || Ok(()),
            || Ok(()),
            || Ok(()),
        )
        .unwrap_err();

        assert!(error.message.contains("capture directory identity changed"));
        assert!(!root.join("camera/camera.mp4").exists());
        assert!(!displaced.join("camera/camera.mp4").exists());
        fs::remove_dir_all(displaced).unwrap();
    }

    #[test]
    fn root_swap_after_anchoring_refuses_before_publication() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let staged = staged(&root);
        let owner = owner(&root);
        let (stage, _) = nameless_stage(&root);
        let probe = FixtureProbe::new(facts());
        let displaced = root.with_extension("camera-anchor-displaced");

        let error = finalize_after_close_for_test(
            &probe,
            &owner,
            &staged,
            stage,
            || {
                swap_capture_root(&root, &displaced);
                Ok(())
            },
            || Ok(()),
            || Ok(()),
            || Ok(()),
        )
        .unwrap_err();

        assert!(error.message.contains("capture directory identity changed"));
        assert!(!root.join("camera/camera.mp4").exists());
        assert!(!displaced.join("camera/camera.mp4").exists());
        fs::remove_dir_all(displaced).unwrap();
    }

    #[test]
    fn root_swap_after_link_refuses_without_a_new_root_seal_claim() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let staged = staged(&root);
        let owner = owner(&root);
        let (stage, _) = nameless_stage(&root);
        let probe = FixtureProbe::new(facts());
        let displaced = root.with_extension("camera-link-displaced");

        let error = finalize_after_close_for_test(
            &probe,
            &owner,
            &staged,
            stage,
            || Ok(()),
            || Ok(()),
            || {
                swap_capture_root(&root, &displaced);
                Ok(())
            },
            || Ok(()),
        )
        .unwrap_err();

        assert!(error.message.contains("capture directory identity changed"));
        assert!(!root.join("camera/camera.mp4").exists());
        assert_eq!(
            fs::read(displaced.join("camera/camera.mp4")).unwrap(),
            b"closed camera media"
        );
        fs::remove_dir_all(displaced).unwrap();
    }

    #[test]
    fn swapped_destination_parent_fails_before_descriptor_publication() {
        let temp = tempfile::tempdir().unwrap();
        let staged = staged(temp.path());
        let owner = owner(temp.path());
        let (stage, _) = nameless_stage(temp.path());
        let probe = FixtureProbe::new(facts());
        let camera = temp.path().join("camera");
        let displaced = temp.path().join("camera-displaced");

        let error = finalize_after_close_for_test(
            &probe,
            &owner,
            &staged,
            stage,
            || {
                fs::rename(&camera, &displaced).unwrap();
                fs::create_dir(&camera).unwrap();
                Ok(())
            },
            || Ok(()),
            || Ok(()),
            || Ok(()),
        )
        .unwrap_err();

        assert!(error.message.contains("parent identity changed"));
        assert!(!camera.join("camera.mp4").exists());
        assert!(!displaced.join("camera.mp4").exists());
    }

    #[test]
    fn swapped_staging_parent_cannot_redirect_the_nameless_stage() {
        let temp = tempfile::tempdir().unwrap();
        let staged = staged(temp.path());
        let owner = owner(temp.path());
        let (stage, _) = nameless_stage(temp.path());
        let probe = FixtureProbe::new(facts());
        let staging = temp.path().join(".camera-staging");
        let displaced = temp.path().join("staging-displaced");

        let seal = finalize_after_close_for_test(
            &probe,
            &owner,
            &staged,
            stage,
            || {
                fs::rename(&staging, &displaced).unwrap();
                fs::create_dir(&staging).unwrap();
                Ok(())
            },
            || Ok(()),
            || Ok(()),
            || Ok(()),
        )
        .unwrap();

        assert_eq!(seal.video(), "camera/camera.mp4");
        assert!(temp.path().join("camera/camera.mp4").is_file());
        assert!(staging.read_dir().unwrap().next().is_none());
        assert!(displaced.read_dir().unwrap().next().is_none());
    }

    #[test]
    fn replacement_after_link_is_not_unlinked_by_failed_identity_verification() {
        let temp = tempfile::tempdir().unwrap();
        let staged = staged(temp.path());
        let owner = owner(temp.path());
        let (stage, _) = nameless_stage(temp.path());
        let probe = FixtureProbe::new(facts());
        let destination = temp.path().join("camera/camera.mp4");

        let error = finalize_after_close_for_test(
            &probe,
            &owner,
            &staged,
            stage,
            || Ok(()),
            || Ok(()),
            || {
                fs::remove_file(&destination).unwrap();
                fs::write(&destination, b"replacement after link").unwrap();
                Ok(())
            },
            || Ok(()),
        )
        .unwrap_err();

        assert!(error.message.contains("identity changed"));
        assert_eq!(fs::read(destination).unwrap(), b"replacement after link");
    }

    #[test]
    fn replacement_after_publication_identity_check_survives_the_final_recheck() {
        let temp = tempfile::tempdir().unwrap();
        let staged = staged(temp.path());
        let owner = owner(temp.path());
        let (stage, _) = nameless_stage(temp.path());
        let probe = FixtureProbe::new(facts());
        let destination = temp.path().join("camera/camera.mp4");

        let error = finalize_after_close_for_test(
            &probe,
            &owner,
            &staged,
            stage,
            || Ok(()),
            || Ok(()),
            || Ok(()),
            || {
                fs::remove_file(&destination).unwrap();
                fs::write(&destination, b"replacement after identity check").unwrap();
                Ok(())
            },
        )
        .unwrap_err();

        assert!(error.message.contains("identity changed"));
        assert_eq!(
            fs::read(destination).unwrap(),
            b"replacement after identity check"
        );
    }

    fn swap_capture_root(root: &Path, displaced: &Path) {
        fs::rename(root, displaced).unwrap();
        fs::create_dir(root).unwrap();
        fs::create_dir(root.join("camera")).unwrap();
        fs::create_dir(root.join(".camera-staging")).unwrap();
    }
}

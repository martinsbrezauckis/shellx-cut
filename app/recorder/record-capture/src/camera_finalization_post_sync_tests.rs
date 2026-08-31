#[cfg(target_os = "linux")]
mod linux {
    use std::fs;
    use std::path::Path;

    use crate::camera_finalization::finalize_after_close_with_post_sync_for_test;
    use crate::camera_finalization_test_support::{
        facts, nameless_stage, owner, rewrite_stage, staged, FixtureProbe,
    };

    #[test]
    fn replacement_after_file_sync_refuses_without_destination_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let staged = staged(temp.path());
        let owner = owner(temp.path());
        let (stage, _) = nameless_stage(temp.path());
        let probe = FixtureProbe::new(facts());
        let destination = temp.path().join("camera/camera.mp4");

        let error = finalize_after_close_with_post_sync_for_test(
            &probe,
            &owner,
            &staged,
            stage,
            || {
                replace_destination(&destination, b"replacement after file sync");
                Ok(())
            },
            || Ok(()),
        )
        .unwrap_err();

        assert!(error.message.contains("publication identity changed"));
        assert_eq!(
            fs::read(destination).unwrap(),
            b"replacement after file sync"
        );
    }

    #[test]
    fn alias_mutation_after_file_sync_refuses_without_destination_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let staged = staged(temp.path());
        let owner = owner(temp.path());
        let (stage, mut writer) = nameless_stage(temp.path());
        let probe = FixtureProbe::new(facts());
        let destination = temp.path().join("camera/camera.mp4");

        let error = finalize_after_close_with_post_sync_for_test(
            &probe,
            &owner,
            &staged,
            stage,
            || {
                rewrite_stage(&mut writer);
                Ok(())
            },
            || Ok(()),
        )
        .unwrap_err();

        assert!(error.message.contains("published media bytes changed"));
        assert_eq!(fs::read(destination).unwrap(), b"mutated camera data");
    }

    #[test]
    fn replacement_after_directory_sync_refuses_without_destination_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let staged = staged(temp.path());
        let owner = owner(temp.path());
        let (stage, _) = nameless_stage(temp.path());
        let probe = FixtureProbe::new(facts());
        let destination = temp.path().join("camera/camera.mp4");

        let error = finalize_after_close_with_post_sync_for_test(
            &probe,
            &owner,
            &staged,
            stage,
            || Ok(()),
            || {
                replace_destination(&destination, b"replacement after directory sync");
                Ok(())
            },
        )
        .unwrap_err();

        assert!(error.message.contains("publication identity changed"));
        assert_eq!(
            fs::read(destination).unwrap(),
            b"replacement after directory sync"
        );
    }

    #[test]
    fn alias_mutation_after_directory_sync_refuses_without_destination_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let staged = staged(temp.path());
        let owner = owner(temp.path());
        let (stage, mut writer) = nameless_stage(temp.path());
        let probe = FixtureProbe::new(facts());
        let destination = temp.path().join("camera/camera.mp4");

        let error = finalize_after_close_with_post_sync_for_test(
            &probe,
            &owner,
            &staged,
            stage,
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

    fn replace_destination(destination: &Path, replacement: &[u8]) {
        fs::remove_file(destination).unwrap();
        fs::write(destination, replacement).unwrap();
    }
}

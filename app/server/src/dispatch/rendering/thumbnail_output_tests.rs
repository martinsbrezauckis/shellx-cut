use super::*;

#[test]
fn thumbnail_refuses_hardlinked_leaf_before_reading_media() {
    let root = tempfile::tempdir().unwrap();
    let outside = root.path().join("outside.bin");
    let thumb = root.path().join("thumb.jpg");
    std::fs::write(&outside, b"keep").unwrap();
    std::fs::hard_link(&outside, &thumb).unwrap();
    let error = write_thumbnail(&root.path().join("missing.mp4"), &thumb, 1000).unwrap_err();
    assert_eq!(error.code, cut_core::error_codes::INVALID_ARGS);
    assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
    let assessment = assess_publish_package(
        &[serde_json::json!({
            "aspect": "9:16", "pass": true, "hash": "video", "thumb": null,
        })],
        None,
    );
    assert_eq!(assessment.status, "needs_review");
    assert!(assessment
        .issues
        .iter()
        .any(|issue| issue.code == "thumbnail_missing"));
}

#[cfg(unix)]
#[test]
fn thumbnail_refuses_linked_and_dangling_leaves_without_touching_target() {
    for dangling in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let outside = root.path().join("outside.bin");
        if !dangling {
            std::fs::write(&outside, b"keep").unwrap();
        }
        let thumb = root.path().join("thumb.jpg");
        std::os::unix::fs::symlink(&outside, &thumb).unwrap();
        let error = write_thumbnail(&root.path().join("missing.mp4"), &thumb, 1000).unwrap_err();
        assert_eq!(error.code, cut_core::error_codes::INVALID_ARGS);
        if dangling {
            assert!(!outside.exists());
        } else {
            assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
        }
    }
}

#[test]
fn thumbnail_real_ffmpeg_preserves_plain_overwrite_and_shared_folder() {
    let root = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o770)).unwrap();
    }
    let media = root.path().join("source.mp4");
    cut_media::ffmpeg::run_ffmpeg(&[
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        "color=c=red:s=64x48:r=10:d=0.4".into(),
        "-c:v".into(),
        "libx264".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        media.display().to_string(),
    ])
    .unwrap();
    let thumb = root.path().join("thumb.jpg");
    std::fs::write(&thumb, b"ordinary previous thumbnail").unwrap();
    write_thumbnail(&media, &thumb, 400).unwrap();
    let bytes = std::fs::read(&thumb).unwrap();
    assert_eq!(&bytes[..2], &[0xff, 0xd8]);
    write_thumbnail(&media, &thumb, 400).unwrap();
    assert_eq!(&std::fs::read(&thumb).unwrap()[..2], &[0xff, 0xd8]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(root.path()).unwrap().permissions().mode() & 0o777,
            0o770
        );
    }
}

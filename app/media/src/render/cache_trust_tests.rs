//! Item 155: a mask or power-window alpha planted under its cache name is not
//! used; the render bakes its own (fails before the fix, passes after).

use super::graph_inputs::{bake_mask_png_atomic, mask_alpha_path};
use cut_core::{ClipMask, MaskEffect, MaskShape};

fn private_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    dir
}

fn rect(points: [[f64; 2]; 2]) -> ClipMask {
    ClipMask {
        shape: MaskShape::Rect,
        points: points.to_vec(),
        feather: 0.0,
        invert: false,
        effect: MaskEffect::Blur,
        strength: None,
        range_ms: None,
        track: None,
        regions: vec![],
    }
}

#[test]
fn planted_mask_alpha_is_baked_again_and_then_reused() {
    let project = private_dir();
    let redaction = rect([[0.1, 0.1], [0.9, 0.9]]);
    let path = mask_alpha_path(project.path(), &redaction, 64, 64);

    // The sender plants a VALID alpha of a different (tiny) region under the
    // exact name the redaction will look for.
    let elsewhere = private_dir();
    let planted = elsewhere.path().join("planted.png");
    crate::mask::bake_mask_png(&rect([[0.0, 0.0], [0.05, 0.05]]), 64, 64, &planted).unwrap();
    let parent = path.parent().unwrap();
    std::fs::create_dir_all(parent).unwrap();
    std::fs::copy(&planted, &path).unwrap();

    bake_mask_png_atomic(&redaction, 64, 64, &path).unwrap();

    let expected = elsewhere.path().join("expected.png");
    crate::mask::bake_mask_png(&redaction, 64, 64, &expected).unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        std::fs::read(&expected).unwrap(),
        "the planted alpha was used instead of the redaction's own"
    );
    assert!(
        crate::cache_trust::trusted(&path),
        "this engine's bake is recorded"
    );
}

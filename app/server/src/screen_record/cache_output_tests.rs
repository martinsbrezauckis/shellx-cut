use super::*;

#[test]
fn recorder_cache_publication_replaces_plain_files_and_preserves_warm_hits() {
    let project = tempfile::tempdir().unwrap();
    let dir = super::super::screen_record_cache_dir(project.path()).unwrap();
    let plan = dir.join("capture.plan.json");
    write(&plan, b"first").unwrap();
    write(&plan, b"scene and Studio patch").unwrap();
    assert_eq!(std::fs::read(&plan).unwrap(), b"scene and Studio patch");
    let baked = dir.join("bake.mp4");
    bake_seed(&baked, |path| {
        std::fs::write(path, b"completed media")?;
        Ok(())
    })
    .unwrap();
    bake_seed(&baked, |_| panic!("warm cache must not rerender")).unwrap();
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
fn recorder_cache_refuses_static_linked_plan_and_raw_normal_bake_leaves() {
    use std::os::unix::fs::symlink;
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("sentinel");
    std::fs::write(&sentinel, b"outside unchanged").unwrap();
    let dir = super::super::screen_record_cache_dir(project.path()).unwrap();
    for name in ["capture.plan.json", "raw_v.mp4", "v.mp4"] {
        let leaf = dir.join(name);
        for target in [&sentinel, &outside.path().join("absent")] {
            symlink(target, &leaf).unwrap();
            assert!(write(&leaf, b"overwrite").is_err());
            assert!(bake_seed(&leaf, |_| panic!("unsafe leaf reached renderer")).is_err());
            std::fs::remove_file(&leaf).unwrap();
        }
        std::fs::hard_link(&sentinel, &leaf).unwrap();
        assert!(write(&leaf, b"overwrite").is_err());
        assert!(cache_hit(&leaf).is_err());
        std::fs::remove_file(&leaf).unwrap();
        std::fs::create_dir(&leaf).unwrap();
        assert!(cache_hit(&leaf).is_err());
        std::fs::remove_dir(&leaf).unwrap();
    }
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside unchanged");
    assert!(!outside.path().join("absent").exists());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}

#[test]
fn failed_recorder_stage_leaves_previous_output_and_cleans_private_stage() {
    let project = tempfile::tempdir().unwrap();
    let dir = super::super::screen_record_cache_dir(project.path()).unwrap();
    let plan = dir.join("capture.plan.json");
    write(&plan, b"previous plan").unwrap();
    let stage = StagedOutput::new(&plan).unwrap();
    let owned = stage.path().parent().unwrap().to_owned();
    assert!(stage.publish().is_err());
    assert!(!owned.exists());
    assert_eq!(std::fs::read(&plan).unwrap(), b"previous plan");
}

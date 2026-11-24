use super::super::take_edit::tests::fixture;
use super::*;

async fn setup() -> (tempfile::TempDir, PathBuf, Project, RecordingTake, Origin) {
    let (temp, state, _) = fixture().await;
    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    let project = store.project.clone();
    let take = project.recordings[0].clone();
    let path = destination(&store.dir, &take, std::ffi::OsStr::new("render.mp4")).unwrap();
    std::fs::write(&path, b"verified render").unwrap();
    remember(&store.dir, &take, &path).unwrap();
    let origin = load(&store.dir).remove(0);
    (temp, store.dir.clone(), project, take, origin)
}

#[tokio::test]
async fn owned_previous_render_is_removed_and_reports_exact_bytes() {
    let (_temp, dir, project, take, origin) = setup().await;
    let result = retire(
        &dir,
        &project,
        &take,
        Some(&origin.path),
        &dir.join("new.mp4"),
    );
    assert_eq!(result["status"], "removed", "{result}");
    assert_eq!(result["bytes"], 15);
    assert!(!origin.path.exists());
}

#[tokio::test]
async fn outside_folder_and_same_source_other_take_are_kept() {
    let (_temp, dir, project, mut take, mut origin) = setup().await;
    let outside = dir.join("outside.mp4");
    std::fs::write(&outside, b"keep").unwrap();
    origin.path = outside.clone();
    assert_eq!(
        remove_previous_render(&dir, &project, &origin)["status"],
        "kept"
    );
    assert!(outside.exists());
    take.id = "another-take-with-same-source".into();
    assert_eq!(
        retire(
            &dir,
            &project,
            &take,
            Some(&origin.path),
            &dir.join("new.mp4")
        )["status"],
        "kept"
    );
}

#[tokio::test]
async fn original_recording_and_input_record_are_kept() {
    let (_temp, dir, mut project, _, origin) = setup().await;
    project.recordings[0].source = origin.path.display().to_string();
    let mut record = origin.clone();
    record.source = origin.path.clone();
    assert_eq!(
        remove_previous_render(&dir, &project, &record)["reason"],
        "an original recording or recording input"
    );
    project.recordings[0].source = origin.source.display().to_string();
    project.recordings[0].events = origin.path.display().to_string();
    assert_eq!(
        remove_previous_render(&dir, &project, &origin)["reason"],
        "an original recording or recording input"
    );
    assert!(origin.path.exists());
}

#[tokio::test]
async fn clip_in_use_is_kept_in_preview_and_after_confirmation() {
    let (_temp, dir, mut project, _, origin) = setup().await;
    project.assets.get_mut("a1").unwrap().path = origin.path.display().to_string();
    // Keep the clip's old asset but make the take itself use another asset.
    let mut new = project.assets["a1"].clone();
    new.path = dir.join("new.mp4").display().to_string();
    project.assets.insert("a2".into(), new);
    project.recordings[0].asset = "a2".into();
    assert_eq!(
        remove_previous_render(&dir, &project, &origin)["reason"],
        "still used by a clip or take"
    );
    let (targets, preview) = inventory(&dir, &project);
    assert!(targets.is_empty());
    assert_eq!(preview["files"][0]["removable"], false);
    assert!(referenced(&dir, &project, &origin.path));
    assert!(origin.path.exists());
}

// Unix keeps a sparse 1 TiB file (far above any real render); NTFS allocates
// the file, so Windows uses 64 MiB, still above any read bound of a test.
#[cfg(unix)]
const LARGE_LEAF: u64 = 1 << 40;
#[cfg(windows)]
const LARGE_LEAF: u64 = 64 << 20;

#[tokio::test]
async fn replaced_file_and_unowned_large_leaf_are_never_hashed_or_deleted() {
    let (_temp, dir, project, take, origin) = setup().await;
    std::fs::write(&origin.path, b"different").unwrap();
    assert_eq!(
        remove_previous_render(&dir, &project, &origin)["status"],
        "kept"
    );
    let large = root(&dir, &take).join("planted.mp4");
    std::fs::File::create(&large)
        .unwrap()
        .set_len(LARGE_LEAF)
        .unwrap();
    assert_eq!(
        retire(&dir, &project, &take, Some(&large), &dir.join("new.mp4"))["reason"],
        "no independent ownership record for this take"
    );
    assert!(large.exists());
    assert_eq!(std::fs::metadata(&large).unwrap().len(), LARGE_LEAF);
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_leaf_and_intermediate_folder_are_kept() {
    use std::os::unix::fs::symlink;
    let (_temp, dir, project, _, origin) = setup().await;
    let sentinel = dir.join("sentinel.mp4");
    std::fs::write(&sentinel, b"outside").unwrap();
    std::fs::remove_file(&origin.path).unwrap();
    symlink(&sentinel, &origin.path).unwrap();
    assert_eq!(
        remove_previous_render(&dir, &project, &origin)["status"],
        "kept"
    );
    std::fs::remove_file(&origin.path).unwrap();
    let parent = origin.path.parent().unwrap();
    let moved = dir.join("moved");
    std::fs::rename(parent, &moved).unwrap();
    symlink(&moved, parent).unwrap();
    std::fs::write(moved.join("render.mp4"), b"verified render").unwrap();
    assert_eq!(
        remove_previous_render(&dir, &project, &origin)["status"],
        "kept"
    );
    assert_eq!(std::fs::read(sentinel).unwrap(), b"outside");
    assert!(moved.join("render.mp4").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn hardlink_and_symlink_ancestors_are_kept() {
    use std::os::unix::fs::symlink;
    let (_temp, dir, project, _, origin) = setup().await;
    let sentinel = dir.join("sentinel.mp4");
    std::fs::write(&sentinel, b"outside").unwrap();
    std::fs::remove_file(&origin.path).unwrap();
    std::fs::hard_link(&sentinel, &origin.path).unwrap();
    assert_eq!(
        remove_previous_render(&dir, &project, &origin)["status"],
        "kept"
    );
    std::fs::remove_file(&origin.path).unwrap();
    let renders_dir = dir.join("cache/screen_record/renders");
    let moved_renders = dir.join("moved_renders");
    std::fs::rename(&renders_dir, &moved_renders).unwrap();
    symlink(&moved_renders, &renders_dir).unwrap();
    assert_eq!(
        remove_previous_render(&dir, &project, &origin)["status"],
        "kept"
    );
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside");
}

#[tokio::test]
async fn legacy_renders_stay_kept_and_active_clips_are_not_listed_while_confirmed_remove_deletes_only_owned_files(
) {
    let (_temp, state, capture) = take_edit::tests::fixture().await;
    let (_dir, old_owned, current_owned, legacy_unused, legacy_raw_active) = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        let dir = store.dir.clone();
        let take = store.project.recordings[0].clone();

        let old_owned =
            destination(&dir, &take, std::ffi::OsStr::new("sha256_old_render.mp4")).unwrap();
        std::fs::write(&old_owned, b"owned-old-render").unwrap();
        remember(&dir, &take, &old_owned).unwrap();

        let current_owned =
            destination(&dir, &take, std::ffi::OsStr::new("sha256_cur_render.mp4")).unwrap();
        std::fs::write(&current_owned, b"owned-cur-render").unwrap();
        remember(&dir, &take, &current_owned).unwrap();

        let mut asset_cur = store.project.assets["a1"].clone();
        asset_cur.path = current_owned.display().to_string();
        store.project.assets.insert("a2".into(), asset_cur);
        store.project.recordings[0].asset = "a2".into();
        for c in &mut store.project.tracks[0].clips {
            if let cut_core::Clip::Media(m) = c {
                if m.asset == "a1" {
                    m.asset = "a2".into();
                }
            }
        }

        let legacy_unused = dir.join(format!(
            "cache/screen_record/sha256_{}_1234567890abcdef_v.mp4",
            "a".repeat(64)
        ));
        std::fs::write(&legacy_unused, b"legacy-unused").unwrap();

        // An active raw recording clip in cache/screen_record must NOT be reported as an old/legacy render.
        let legacy_raw_active = dir.join(format!(
            "cache/screen_record/sha256_{}_raw_v.mp4",
            "b".repeat(64)
        ));
        std::fs::write(&legacy_raw_active, b"legacy-raw-active").unwrap();
        let raw_asset: cut_core::Asset = serde_json::from_value(json!({
            "path": legacy_raw_active.display().to_string(),
            "hash": "sha256:raw",
            "probe": {"duration_ms": 2000}
        }))
        .unwrap();
        store.project.assets.insert("a_raw".into(), raw_asset);
        let raw_clip: cut_core::Clip = serde_json::from_value(json!({
            "id": "c_raw",
            "kind": "media",
            "asset": "a_raw",
            "src_in_ms": 0,
            "src_out_ms": 2000,
            "start_ms": 6000
        }))
        .unwrap();
        store.project.tracks[0].clips.push(raw_clip);

        (
            dir,
            old_owned,
            current_owned,
            legacy_unused,
            legacy_raw_active,
        )
    };

    let preview = handle(&state, json!({}), false, cut_core::Actor::system())
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(preview["removable_files"], 1, "{preview}");
    assert_eq!(preview["files"].as_array().unwrap().len(), 1, "{preview}");
    let legacy_files: Vec<_> = preview["legacy"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        legacy_files.len(),
        1,
        "active raw clip must not appear in legacy files: {preview}"
    );
    assert!(legacy_files[0].ends_with("_v.mp4"), "{legacy_files:?}");
    assert!(
        !legacy_files.iter().any(|f| f.contains("raw_v")),
        "{legacy_files:?}"
    );

    // Wrong plan_id / confirm:false must refuse without removing anything.
    assert!(handle(
        &state,
        json!({"plan_id": "wrong", "confirm": true}),
        true,
        cut_core::Actor::system()
    )
    .await
    .is_err());
    assert!(old_owned.exists());

    let plan_id = preview["plan_id"].as_str().unwrap().to_string();
    let removed = handle(
        &state,
        json!({"plan_id": plan_id, "confirm": true}),
        true,
        cut_core::Actor::system(),
    )
    .await
    .unwrap()
    .result
    .unwrap();
    assert_eq!(removed["results"][0]["status"], "removed", "{removed}");
    assert!(!old_owned.exists());
    assert!(current_owned.exists());
    assert!(legacy_unused.exists());
    assert!(legacy_raw_active.exists());
    assert!(capture.join("source.mp4").exists());

    let after = handle(&state, json!({}), false, cut_core::Actor::system())
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(after["removable_files"], 0, "{after}");
    assert!(after["files"].as_array().unwrap().is_empty(), "{after}");
    assert_eq!(
        after["legacy"]["files"].as_array().unwrap().len(),
        1,
        "{after}"
    );
}

#[tokio::test]
async fn second_client_stale_plan_and_camera_asset_use_prevent_removal() {
    let (_temp, state, _) = take_edit::tests::fixture().await;
    let (dir, take, owned_old) = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        let dir = store.dir.clone();
        let take = store.project.recordings[0].clone();
        let owned_old = destination(&dir, &take, std::ffi::OsStr::new("sha256_old.mp4")).unwrap();
        std::fs::write(&owned_old, b"owned-old").unwrap();
        remember(&dir, &take, &owned_old).unwrap();
        let cur = destination(&dir, &take, std::ffi::OsStr::new("sha256_cur.mp4")).unwrap();
        std::fs::write(&cur, b"owned-cur").unwrap();
        remember(&dir, &take, &cur).unwrap();
        let mut a2 = store.project.assets["a1"].clone();
        a2.path = cur.display().to_string();
        store.project.assets.insert("a2".into(), a2);
        store.project.recordings[0].asset = "a2".into();
        for c in &mut store.project.tracks[0].clips {
            if let cut_core::Clip::Media(m) = c {
                if m.asset == "a1" {
                    m.asset = "a2".into();
                }
            }
        }
        (dir, take, owned_old)
    };

    let preview_a = handle(&state, json!({}), false, cut_core::Actor::system())
        .await
        .unwrap()
        .result
        .unwrap();
    let preview_b = handle(&state, json!({}), false, cut_core::Actor::system())
        .await
        .unwrap()
        .result
        .unwrap();
    let plan_a = preview_a["plan_id"].as_str().unwrap().to_string();
    let plan_b = preview_b["plan_id"].as_str().unwrap().to_string();

    // Another client attaches a clip to owned_old before Client B confirms.
    {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        store.project.assets.get_mut("a1").unwrap().path = owned_old.display().to_string();
        let clip: cut_core::Clip = serde_json::from_value(json!({
            "id": "c_extra",
            "kind": "media",
            "asset": "a1",
            "src_in_ms": 0,
            "src_out_ms": 1000,
            "start_ms": 0
        }))
        .unwrap();
        store.project.tracks[0].clips.push(clip);
    }
    assert!(handle(
        &state,
        json!({"plan_id": plan_b, "confirm": true}),
        true,
        cut_core::Actor::system()
    )
    .await
    .is_err());
    assert!(owned_old.exists());

    // Without extra clip, first client removes it; second client's stale plan_id fails.
    {
        let mut guard = state.project.write().await;
        guard.as_mut().unwrap().project.tracks[0]
            .clips
            .retain(|c| c.id() != Some("c_extra"));
    }
    let _ = handle(
        &state,
        json!({"plan_id": plan_a, "confirm": true}),
        true,
        cut_core::Actor::system(),
    )
    .await
    .unwrap();
    assert!(!owned_old.exists());
    assert!(handle(
        &state,
        json!({"plan_id": plan_b, "confirm": true}),
        true,
        cut_core::Actor::system()
    )
    .await
    .is_err());
    let _ = (dir, take);
}

#[tokio::test]
async fn origin_persists_across_restart_and_never_trusts_copied_project_dir() {
    let (_temp, dir, mut project, take, origin) = setup().await;
    // Simulate a fresh server instance / restart opening the same canonical dir.
    let copy_dir = dir.parent().unwrap().join("copied.cutproj");
    std::fs::create_dir_all(root(&copy_dir, &take)).unwrap();
    let copied_render = root(&copy_dir, &take).join("render.mp4");
    std::fs::copy(&origin.path, &copied_render).unwrap();

    // Copied project path has no independent origin record -> kept.
    let copy_outcome = retire(
        &copy_dir,
        &project,
        &take,
        Some(&copied_render),
        &copy_dir.join("new.mp4"),
    );
    assert_eq!(copy_outcome["status"], "kept", "{copy_outcome}");
    assert!(copied_render.exists());

    // camera_asset still pointing at origin protects it even without a clip reference.
    project.recordings[0].camera_asset = Some("a1".into());
    project.assets.get_mut("a1").unwrap().path = origin.path.display().to_string();
    assert_eq!(
        remove_previous_render(&dir, &project, &origin)["reason"],
        "still used by a clip or take"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn folder_swapped_for_a_link_to_the_same_file_is_kept_and_untouched() {
    use std::os::unix::fs::symlink;
    // The file keeps its identity (same inode, size and times) behind the link:
    // only the no-follow open of every folder can refuse this.
    let (_temp, dir, project, _, origin) = setup().await;
    let renders = dir.join("cache/screen_record/renders");
    let moved = dir.join("moved_renders");
    std::fs::rename(&renders, &moved).unwrap();
    symlink(&moved, &renders).unwrap();
    let result = remove_previous_render(&dir, &project, &origin);
    assert_eq!(result["status"], "kept", "{result}");
    let held = moved.join(origin.path.strip_prefix(&renders).unwrap());
    assert_eq!(std::fs::read(held).unwrap(), b"verified render");
}

fn replace_journal_start(dir: &Path) {
    // What a received project does: its own journal sits at this folder, with
    // its own `project.create` record (the engine never wrote this one here).
    let ops = dir.join("ops.jsonl");
    let text = std::fs::read_to_string(&ops).unwrap();
    let (first, rest) = text.split_once('\n').unwrap_or((&text, ""));
    let mut record: Value = serde_json::from_str(first).unwrap();
    record["ts"] = json!("2001-01-01T00:00:00.000Z");
    std::fs::write(&ops, format!("{record}\n{rest}")).unwrap();
}

#[tokio::test]
async fn project_replaced_in_place_with_same_take_and_source_keeps_the_old_render() {
    let (_temp, dir, project, take, origin) = setup().await;
    replace_journal_start(&dir);
    let result = retire(
        &dir,
        &project,
        &take,
        Some(&origin.path),
        &dir.join("new.mp4"),
    );
    assert_eq!(result["status"], "kept", "{result}");
    assert_eq!(
        result["reason"], "recorded for a different project at this folder",
        "{result}"
    );
    assert!(origin.path.exists());
    let (targets, preview) = inventory(&dir, &project);
    assert!(targets.is_empty(), "{preview}");
    assert_eq!(preview["files"][0]["removable"], false, "{preview}");
}

#[tokio::test]
async fn a_record_without_a_project_identity_is_kept() {
    let (_temp, dir, project, _, origin) = setup().await;
    let mut value = serde_json::to_value(&origin).unwrap();
    value.as_object_mut().unwrap().remove("project");
    let old_record: Origin = serde_json::from_value(value).unwrap();
    let result = remove_previous_render(&dir, &project, &old_record);
    assert_eq!(result["status"], "kept", "{result}");
    assert!(origin.path.exists());
}

#[tokio::test]
async fn replies_name_files_inside_the_project_only_and_kept_sizes_are_current() {
    let (_temp, state, _) = take_edit::tests::fixture().await;
    let (dir, outside_dir) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let dir = store.dir.clone();
        let take = store.project.recordings[0].clone();
        let changed = destination(&dir, &take, std::ffi::OsStr::new("sha256_changed.mp4")).unwrap();
        std::fs::write(&changed, b"small").unwrap();
        remember(&dir, &take, &changed).unwrap();
        std::fs::write(&changed, b"now a much longer file than recorded").unwrap();
        let outside_dir = dir.parent().unwrap().join("elsewhere-private-folder");
        std::fs::create_dir_all(&outside_dir).unwrap();
        let outside = outside_dir.join("outside.mp4");
        std::fs::write(&outside, b"x").unwrap();
        let identity = render_files::access(&outside, None, false).unwrap();
        let mut origins = load(&dir);
        origins.push(Origin {
            take: take.id.clone(),
            source: resolved(&dir, &take.source),
            path: outside,
            identity,
            project: project_identity(&dir).unwrap(),
        });
        save(&dir, &origins).unwrap();
        let legacy = dir.join(format!(
            "cache/screen_record/sha256_{}_1234567890abcdef_v.mp4",
            "c".repeat(64)
        ));
        std::fs::write(legacy, b"legacy").unwrap();
        (dir, outside_dir)
    };
    let preview = handle(&state, json!({}), false, cut_core::Actor::system())
        .await
        .unwrap()
        .result
        .unwrap();
    let text = preview.to_string();
    assert!(!text.contains(&*dir.to_string_lossy()), "{text}");
    assert!(!text.contains(&*outside_dir.to_string_lossy()), "{text}");
    assert!(preview["legacy"].get("folder").is_none(), "{text}");
    assert!(!text.contains("0.6.122"), "{text}");
    let files = preview["files"].as_array().unwrap();
    let changed = files
        .iter()
        .find(|f| f["name"].as_str().unwrap().ends_with("sha256_changed.mp4"))
        .unwrap_or_else(|| panic!("{text}"));
    assert_eq!(changed["bytes"], 36, "current size, not the recorded one");
    assert_eq!(changed["removable"], false);
    assert!(files
        .iter()
        .any(|f| f["name"].as_str().unwrap().ends_with("outside.mp4")
            && f["name"].as_str().unwrap().starts_with("outside project/")));
}

/// A background reader (a probe, a virus scanner) that has the old render open
/// without sharing deletion for a moment must not make the earlier render stay.
#[cfg(windows)]
#[tokio::test]
async fn render_held_open_for_a_moment_is_still_removed_on_windows() {
    use std::os::windows::fs::OpenOptionsExt;
    let (_temp, dir, project, _, origin) = setup().await;
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1 | 2) // FILE_SHARE_READ | FILE_SHARE_WRITE, not DELETE
        .open(&origin.path)
        .unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        drop(held);
    });
    let result = remove_previous_render(&dir, &project, &origin);
    release.join().unwrap();
    assert_eq!(result["status"], "removed", "{result}");
    assert!(!origin.path.exists());
}

/// A file that stays open past the bounded wait is kept, never forced.
#[cfg(windows)]
#[tokio::test]
async fn render_held_open_for_good_is_kept_on_windows() {
    use std::os::windows::fs::OpenOptionsExt;
    let (_temp, dir, project, _, origin) = setup().await;
    let _held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1 | 2)
        .open(&origin.path)
        .unwrap();
    let result = remove_previous_render(&dir, &project, &origin);
    assert_eq!(result["status"], "kept", "{result}");
    assert_eq!(result["message"], "Earlier render kept: it was in use");
    assert!(origin.path.exists());
}

#[tokio::test]
async fn remove_previous_render_removes_owned_sidecar_and_keeps_foreign_or_linked_sidecar() {
    let (_temp, dir, project, _, origin) = setup().await;
    let render_hash = cut_core::hash_file(&origin.path).unwrap();
    let sidecar = PathBuf::from(format!("{}.clip-markers.json", origin.path.display()));
    std::fs::write(
        &sidecar,
        serde_json::to_vec(&serde_json::json!({
            "schema": "shellx-cut/baked-clip-markers@1",
            "baked_hash": render_hash,
            "markers": [{"source_ms": 500, "label": "Marker 1"}]
        }))
        .unwrap(),
    )
    .unwrap();

    let result = remove_previous_render(&dir, &project, &origin);
    assert_eq!(result["status"], "removed", "{result}");
    assert!(!origin.path.exists());
    assert!(
        !sidecar.exists(),
        "owned .clip-markers.json sidecar must be removed with its earlier render"
    );

    // Foreign-schema sidecar next to an owned render must be kept even when the render is removed.
    let (_temp2, dir2, project2, _, origin2) = setup().await;
    let foreign_sidecar = PathBuf::from(format!("{}.clip-markers.json", origin2.path.display()));
    std::fs::write(&foreign_sidecar, br#"{"schema":"someone-else/1"}"#).unwrap();
    let result2 = remove_previous_render(&dir2, &project2, &origin2);
    assert_eq!(result2["status"], "removed", "{result2}");
    assert!(!origin2.path.exists());
    assert!(
        foreign_sidecar.exists(),
        "non-Cut .clip-markers.json must not be removed"
    );
}

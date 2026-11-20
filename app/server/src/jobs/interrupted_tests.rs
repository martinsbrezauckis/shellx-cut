use super::*;
use crate::jobs::JobManager;

/// Tests that write notes or tidy up all notes share one folder: one at a time,
/// or one test's tidy-up stops and counts another test's helper.
static NOTES: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn notes_lock() -> std::sync::MutexGuard<'static, ()> {
    NOTES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn notes() -> Vec<PathBuf> {
    notes_dir()
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

fn note_for(job_id: &str, project: &Path) -> Option<PathBuf> {
    note_path(project, job_id).filter(|path| path.exists())
}

#[test]
fn a_running_export_leaves_a_note_until_it_ends_and_automatic_work_none() {
    let project = tempfile::tempdir().unwrap();
    let jobs = JobManager::new(crate::events::EventBus::new());
    jobs.attach_project(project.path()).unwrap();
    let export = jobs.create("render");
    let proxy = jobs.create("proxy");
    assert!(
        note_for(&export.job_id, project.path()).is_some(),
        "an export someone asked for is noted"
    );
    assert!(
        note_for(&proxy.job_id, project.path()).is_none(),
        "automatic work is not"
    );
    jobs.finish(&export.job_id, serde_json::json!({}));
    assert!(
        note_for(&export.job_id, project.path()).is_none(),
        "the normal end removes it"
    );
}

/// A process id of a process that has already exited.
fn dead_pid() -> u32 {
    let mut child = if cfg!(windows) {
        std::process::Command::new("cmd")
            .args(["/C", "exit 0"])
            .spawn()
            .unwrap()
    } else {
        std::process::Command::new("true").spawn().unwrap()
    };
    let pid = child.id();
    child.wait().unwrap();
    pid
}

#[test]
fn a_note_of_an_ended_engine_is_listed_and_can_be_dismissed() {
    let _notes = notes_lock();
    let project = tempfile::tempdir().unwrap();
    let dir = notes_dir().unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    let key = format!("{}-test-job_901", dead_pid());
    let note = Note {
        schema: SCHEMA.into(),
        project_dir: project.path().join("Talk.cutproj"),
        job_id: "job_901".into(),
        kind: "render".into(),
        started_unix_ms: 1,
        owner_pid: key.split('-').next().unwrap().parse().unwrap(),
        owner_identity: "1:2".into(),
        leftover_files_removed: 0,
        engine_run: None,
        export_dirs: Vec::new(),
        helpers_stopped: 0,
    };
    std::fs::write(
        dir.join(format!("{key}.json")),
        serde_json::to_vec(&note).unwrap(),
    )
    .unwrap();
    let listed = interrupted();
    assert!(listed
        .iter()
        .any(|(k, n)| k == &key && n.job_id == "job_901"));
    assert!(dismiss(&key));
    assert!(!interrupted().iter().any(|(k, _)| k == &key));
    assert!(!dismiss(&key));
    assert!(!dismiss("../escape"), "a key is never a path");
}

#[test]
fn notes_of_this_running_engine_are_not_interrupted() {
    let _notes = notes_lock();
    let project = tempfile::tempdir().unwrap();
    note_started(project.path(), "job_902", "render");
    let path = note_path(project.path(), "job_902").unwrap();
    let key = key_of(&path).unwrap();
    assert!(notes().contains(&path));
    assert!(!interrupted().iter().any(|(k, _)| k == &key));
    note_ended(project.path(), "job_902");
    assert!(!path.exists());
}

#[test]
fn only_the_engines_own_leftover_names_are_removed() {
    let folder = tempfile::tempdir().unwrap();
    let write = |name: &str| std::fs::write(folder.path().join(name), b"x").unwrap();
    write(".cut-ffmpeg-AbC123.tmp.mp4");
    write(".render_002.mp4.shellx-cut-reservation.lock");
    write(".render_002.mp4.shellx-cut-reserving");
    write("render_001.mp4");
    write(".hidden-notes.txt");
    write("cut-ffmpeg-not-hidden.tmp.mp4");
    write(".render_004.4411.7.tmp.mp4");
    write(".render_004.mp4.tmp");
    let later = SystemTime::now() + std::time::Duration::from_secs(5);
    let mut removed = sweep_export_leftovers(folder.path(), later);
    removed.sort();
    assert_eq!(
        removed,
        vec![
            ".cut-ffmpeg-AbC123.tmp.mp4".to_string(),
            ".render_002.mp4.shellx-cut-reservation.lock".to_string(),
            ".render_002.mp4.shellx-cut-reserving".to_string(),
            ".render_004.4411.7.tmp.mp4".to_string(),
        ]
    );
    for kept in [
        "render_001.mp4",
        ".hidden-notes.txt",
        "cut-ffmpeg-not-hidden.tmp.mp4",
        ".render_004.mp4.tmp",
    ] {
        assert!(folder.path().join(kept).exists(), "{kept}");
    }
}

#[test]
fn nothing_is_removed_while_an_export_in_the_folder_holds_its_lock() {
    use fs2::FileExt;
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join(".cut-ffmpeg-live.tmp.mp4"), b"x").unwrap();
    let lease = folder
        .path()
        .join(".render_003.mp4.shellx-cut-reservation.lock");
    std::fs::write(&lease, b"").unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lease)
        .unwrap();
    held.lock_exclusive().unwrap();
    let later = SystemTime::now() + std::time::Duration::from_secs(5);
    assert!(sweep_export_leftovers(folder.path(), later).is_empty());
    assert!(folder.path().join(".cut-ffmpeg-live.tmp.mp4").exists());
    // Files changed after the engine started are this run's work: kept too.
    fs2::FileExt::unlock(&held).unwrap();
    drop(held);
    let earlier = SystemTime::now() - std::time::Duration::from_secs(3_600);
    assert!(sweep_export_leftovers(folder.path(), earlier).is_empty());
}

#[cfg(unix)]
#[test]
fn a_link_with_a_staging_name_is_never_followed_or_removed() {
    let folder = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("keep.mp4");
    std::fs::write(&target, b"outside").unwrap();
    let link = folder.path().join(".cut-ffmpeg-link.tmp.mp4");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let later = SystemTime::now() + std::time::Duration::from_secs(5);
    assert!(sweep_export_leftovers(folder.path(), later).is_empty());
    assert!(link.symlink_metadata().is_ok());
    assert_eq!(std::fs::read(&target).unwrap(), b"outside");
}

#[test]
fn an_interrupted_export_has_its_leftovers_removed_and_counted() {
    let _notes = notes_lock();
    let project = tempfile::tempdir().unwrap();
    let exports = project.path().join("exports");
    std::fs::create_dir_all(&exports).unwrap();
    std::fs::write(exports.join(".cut-ffmpeg-Zz9.tmp.mp4"), b"partial").unwrap();
    std::fs::write(
        exports.join(".render_002.mp4.shellx-cut-reservation.lock"),
        b"",
    )
    .unwrap();
    std::fs::write(exports.join("render_001.mp4"), b"kept").unwrap();
    let dir = notes_dir().unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    let pid = dead_pid();
    let key = format!("{pid}-left-job_903");
    let note = Note {
        schema: SCHEMA.into(),
        project_dir: project.path().to_path_buf(),
        job_id: "job_903".into(),
        kind: "render".into(),
        started_unix_ms: 2,
        owner_pid: pid,
        owner_identity: "1:2".into(),
        leftover_files_removed: 0,
        engine_run: None,
        export_dirs: Vec::new(),
        helpers_stopped: 0,
    };
    std::fs::write(
        dir.join(format!("{key}.json")),
        serde_json::to_vec(&note).unwrap(),
    )
    .unwrap();
    let listed = cleanup(true);
    let found = listed
        .iter()
        .find(|(k, _)| k == &key)
        .map(|(_, n)| n.clone());
    // Listed again: the count stays with the note, nothing more is removed.
    let again = cleanup(true);
    let again = again
        .iter()
        .find(|(k, _)| k == &key)
        .map(|(_, n)| n.leftover_files_removed);
    assert!(dismiss(&key));
    assert_eq!(
        found
            .expect("the interrupted export is listed")
            .leftover_files_removed,
        2
    );
    assert_eq!(again, Some(2));
    assert!(!exports.join(".cut-ffmpeg-Zz9.tmp.mp4").exists());
    assert!(!exports
        .join(".render_002.mp4.shellx-cut-reservation.lock")
        .exists());
    assert!(exports.join("render_001.mp4").exists());
}

#[test]
fn this_process_own_staging_names_are_never_removed() {
    let folder = tempfile::tempdir().unwrap();
    let own = format!(".render_005.{}.3.tmp.mp4", std::process::id());
    std::fs::write(folder.path().join(&own), b"x").unwrap();
    std::fs::write(folder.path().join(".render_005.1.3.tmp.mp4"), b"x").unwrap();
    let later = SystemTime::now() + std::time::Duration::from_secs(5);
    assert_eq!(
        sweep_export_leftovers(folder.path(), later),
        vec![".render_005.1.3.tmp.mp4".to_string()]
    );
    assert!(folder.path().join(own).exists());
}

/// A dead run's helper that still writes (the orphaned ffmpeg of a killed
/// engine) is stopped before its files are removed, and both are counted; the
/// app-wide export folder the note names is tidied like the project's.
// Linux: macOS hides /bin/sleep's environment (see engine_lifeline tests).
#[cfg(target_os = "linux")]
#[test]
fn a_surviving_helper_is_stopped_before_the_files_go() {
    let _notes = notes_lock();
    let project = tempfile::tempdir().unwrap();
    let chosen = tempfile::tempdir().unwrap();
    let exports = project.path().join("exports");
    std::fs::create_dir_all(&exports).unwrap();
    std::fs::write(exports.join(".render_007.mp4.shellx-cut-reserving"), b"").unwrap();
    std::fs::write(
        chosen.path().join(".cut-ffmpeg-orphan.tmp.mp4"),
        b"still being written",
    )
    .unwrap();
    let mark = format!("{}-orphan-{:x}", std::process::id(), 0x0f_u32);
    let mut writer = std::process::Command::new("sleep")
        .arg("60")
        .env(crate::engine_lifeline::ENV_ENGINE_RUN, &mark)
        .spawn()
        .unwrap();
    let dir = notes_dir().unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    let pid = dead_pid();
    let key = format!("{pid}-orphan-job_907");
    let note = Note {
        schema: SCHEMA.into(),
        project_dir: project.path().to_path_buf(),
        job_id: "job_907".into(),
        kind: "render".into(),
        started_unix_ms: 3,
        owner_pid: pid,
        owner_identity: "1:2".into(),
        leftover_files_removed: 0,
        engine_run: Some(mark),
        export_dirs: vec![exports.clone(), chosen.path().to_path_buf()],
        helpers_stopped: 0,
    };
    std::fs::write(
        dir.join(format!("{key}.json")),
        serde_json::to_vec(&note).unwrap(),
    )
    .unwrap();
    let listed = cleanup(true);
    let found = listed
        .iter()
        .find(|(k, _)| k == &key)
        .map(|(_, n)| n.clone());
    assert!(dismiss(&key));
    let found = found.expect("the interrupted export is listed");
    assert_eq!(found.helpers_stopped, 1);
    assert_eq!(found.leftover_files_removed, 2);
    assert!(
        !writer.wait().unwrap().success(),
        "the surviving writer was stopped"
    );
    assert!(!chosen.path().join(".cut-ffmpeg-orphan.tmp.mp4").exists());
    assert!(!exports
        .join(".render_007.mp4.shellx-cut-reserving")
        .exists());
}

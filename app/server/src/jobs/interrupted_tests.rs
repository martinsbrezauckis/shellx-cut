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
    let dead = dead_pid();
    let legacy = format!(".render_004.{dead}.7.tmp.mp4");
    let write = |name: &str| {
        let path = folder.path().join(name);
        std::fs::write(&path, b"x").unwrap();
        #[cfg(windows)]
        give_to_this_user(&path);
    };
    write(".cut-ffmpeg-AbC123.tmp.mp4");
    write(".render_002.mp4.shellx-cut-reservation.lock");
    write(".render_002.mp4.shellx-cut-reserving");
    write("render_001.mp4");
    write(".hidden-notes.txt");
    write("cut-ffmpeg-not-hidden.tmp.mp4");
    write(&legacy);
    write(".render_004.mp4.tmp");
    let later = SystemTime::now() + std::time::Duration::from_secs(5);
    let mut removed = sweep_export_leftovers(folder.path(), later);
    removed.sort();
    let mut expected = vec![
        ".cut-ffmpeg-AbC123.tmp.mp4".to_string(),
        ".render_002.mp4.shellx-cut-reservation.lock".to_string(),
        ".render_002.mp4.shellx-cut-reserving".to_string(),
        legacy,
    ];
    expected.sort();
    assert_eq!(removed, expected);
    for kept in [
        "render_001.mp4",
        ".hidden-notes.txt",
        "cut-ffmpeg-not-hidden.tmp.mp4",
        ".render_004.mp4.tmp",
    ] {
        assert!(folder.path().join(kept).exists(), "{kept}");
    }
}

/// `icacls /setowner`: true when the owner was changed. An elevated test shell
/// owns new files as Administrators, so a file Cut itself would have written is
/// handed to the current user first (already the owner in an ordinary session).
#[cfg(windows)]
fn set_owner(path: &Path, account: &str) -> bool {
    std::process::Command::new("icacls")
        .arg(path)
        .args(["/setowner", account])
        .output()
        .is_ok_and(|out| out.status.success())
}

#[cfg(windows)]
fn give_to_this_user(path: &Path) {
    let user = std::env::var("USERNAME").unwrap_or_default();
    let domain = std::env::var("USERDOMAIN").unwrap_or_default();
    let _ = set_owner(path, &format!("{domain}\\{user}"));
    let _ = set_owner(path, &user);
}

#[test]
fn only_the_exact_cross_volume_temp_name_of_this_user_is_swept() {
    let folder = tempfile::tempdir().unwrap();
    let dead = dead_pid();
    let valid = format!(".cut-pub-{dead}.7.tmp");
    let write = |name: &str| std::fs::write(folder.path().join(name), b"x").unwrap();
    write(&valid);
    #[cfg(windows)]
    give_to_this_user(&folder.path().join(&valid));
    for kept in [
        ".cut-pub-important.tmp",
        ".cut-pub-.tmp",
        ".cut-pub-4411.tmp",
        ".cut-pub-4411.x.tmp",
        ".cut-pub-4411.7.8.tmp",
        ".cut-pub-4411.7.tmp.bak",
    ] {
        write(kept);
    }
    let later = SystemTime::now() + std::time::Duration::from_secs(5);
    let removed = sweep_export_leftovers(folder.path(), later);
    assert_eq!(removed, vec![valid]);
    for kept in [
        ".cut-pub-important.tmp",
        ".cut-pub-.tmp",
        ".cut-pub-4411.tmp",
        ".cut-pub-4411.x.tmp",
        ".cut-pub-4411.7.8.tmp",
        ".cut-pub-4411.7.tmp.bak",
    ] {
        assert!(folder.path().join(kept).exists(), "{kept}");
    }
}

/// Needs the right to give a file away (root); as an ordinary user the case
/// cannot be built here and the test says so instead of passing silently.
#[cfg(unix)]
#[test]
fn a_cross_volume_temp_name_owned_by_another_user_is_kept() {
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("NOT RUN: needs root to give a file to another user");
        return;
    }
    use std::os::unix::ffi::OsStrExt;
    let folder = tempfile::tempdir().unwrap();
    let foreign = folder.path().join(format!(".cut-pub-{}.7.tmp", dead_pid()));
    std::fs::write(&foreign, b"theirs").unwrap();
    let c = std::ffi::CString::new(foreign.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::chown(c.as_ptr(), 65_534, 65_534) }, 0);
    let later = SystemTime::now() + std::time::Duration::from_secs(5);
    assert!(sweep_export_leftovers(folder.path(), later).is_empty());
    assert!(foreign.exists());
}

#[cfg(windows)]
#[test]
fn a_cross_volume_temp_name_owned_by_another_account_is_kept() {
    let folder = tempfile::tempdir().unwrap();
    let foreign = folder.path().join(format!(".cut-pub-{}.7.tmp", dead_pid()));
    std::fs::write(&foreign, b"theirs").unwrap();
    if !set_owner(&foreign, "NT AUTHORITY\\SYSTEM") {
        eprintln!("NOT RUN: cannot hand a file to another account here");
        return;
    }
    let later = SystemTime::now() + std::time::Duration::from_secs(5);
    assert!(sweep_export_leftovers(folder.path(), later).is_empty());
    assert!(foreign.exists());
}

#[test]
fn nothing_is_removed_while_an_export_in_the_folder_holds_its_lock() {
    use fs2::FileExt;
    let folder = tempfile::tempdir().unwrap();
    let live_tmp = folder.path().join(".cut-ffmpeg-live.tmp.mp4");
    std::fs::write(&live_tmp, b"x").unwrap();
    let lease = folder
        .path()
        .join(".render_003.mp4.shellx-cut-reservation.lock");
    std::fs::write(&lease, b"").unwrap();
    #[cfg(windows)]
    {
        give_to_this_user(&live_tmp);
        give_to_this_user(&lease);
    }
    let held = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lease)
        .unwrap();
    held.lock_exclusive().unwrap();
    let later = SystemTime::now() + std::time::Duration::from_secs(5);
    assert!(sweep_export_leftovers(folder.path(), later).is_empty());
    assert!(live_tmp.exists());
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
    let stale_ffmpeg = exports.join(".cut-ffmpeg-Zz9.tmp.mp4");
    let stale_lock = exports.join(".render_002.mp4.shellx-cut-reservation.lock");
    std::fs::write(&stale_ffmpeg, b"partial").unwrap();
    std::fs::write(&stale_lock, b"").unwrap();
    #[cfg(windows)]
    {
        give_to_this_user(&stale_ffmpeg);
        give_to_this_user(&stale_lock);
    }
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
    let dead = format!(".render_005.{}.3.tmp.mp4", dead_pid());
    let own_path = folder.path().join(&own);
    let dead_path = folder.path().join(&dead);
    std::fs::write(&own_path, b"x").unwrap();
    std::fs::write(&dead_path, b"x").unwrap();
    #[cfg(windows)]
    {
        give_to_this_user(&own_path);
        give_to_this_user(&dead_path);
    }
    let later = SystemTime::now() + std::time::Duration::from_secs(5);
    assert_eq!(sweep_export_leftovers(folder.path(), later), vec![dead]);
    assert!(own_path.exists());
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
fn live_engine_temp_files_are_kept_until_their_process_exits() {
    let folder = tempfile::tempdir().unwrap();
    let mut child = ChildGuard::spawn_sleep();
    let live_pid = child.pid();
    let pub_name = format!(".cut-pub-{live_pid}.1.tmp");
    let stem_name = format!(".x.{live_pid}.1.tmp");
    let pub_path = folder.path().join(&pub_name);
    let stem_path = folder.path().join(&stem_name);
    std::fs::write(&pub_path, b"writing").unwrap();
    std::fs::write(&stem_path, b"writing").unwrap();
    #[cfg(windows)]
    {
        give_to_this_user(&pub_path);
        give_to_this_user(&stem_path);
    }
    let later = SystemTime::now() + std::time::Duration::from_secs(5);
    assert!(
        sweep_export_leftovers(folder.path(), later).is_empty(),
        "live engine temp files must stay while pid {live_pid} is running"
    );
    assert!(pub_path.exists());
    assert!(stem_path.exists());

    child.kill_and_wait();
    let mut removed = sweep_export_leftovers(folder.path(), later);
    removed.sort();
    let mut expected = vec![pub_name, stem_name];
    expected.sort();
    assert_eq!(removed, expected);
    assert!(!pub_path.exists());
    assert!(!stem_path.exists());
}

/// Needs the right to give files away (root); as an ordinary user the case
/// cannot be built here and prints NOT RUN instead of passing silently.
#[cfg(unix)]
#[test]
fn foreign_staging_and_reservation_files_are_kept() {
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("NOT RUN: needs root to give files to another user");
        return;
    }
    use std::os::unix::ffi::OsStrExt;
    let folder = tempfile::tempdir().unwrap();
    let dead_foreign = format!(".y.{}.4.tmp", dead_pid());
    let foreign_names = [
        ".cut-ffmpeg-x.tmp.mp4",
        ".y.123.4.tmp",
        dead_foreign.as_str(),
        ".video.mp4.shellx-cut-reservation.lock",
        ".video.mp4.shellx-cut-reserving",
    ];
    for name in foreign_names {
        let path = folder.path().join(name);
        std::fs::write(&path, b"theirs").unwrap();
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::chown(c.as_ptr(), 65_534, 65_534) }, 0);
    }
    let own_name = format!(".cut-pub-{}.1.tmp", dead_pid());
    std::fs::write(folder.path().join(&own_name), b"mine").unwrap();
    let later = SystemTime::now() + std::time::Duration::from_secs(5);
    let removed = sweep_export_leftovers(folder.path(), later);
    assert_eq!(removed, vec![own_name]);
    for name in foreign_names {
        assert!(folder.path().join(name).exists(), "{name}");
    }
}

#[cfg(windows)]
#[test]
fn foreign_staging_and_reservation_files_are_kept_windows() {
    let folder = tempfile::tempdir().unwrap();
    let dead = dead_pid();
    let foreign_stem = format!(".y.{dead}.4.tmp");
    let foreign_names = [
        ".cut-ffmpeg-x.tmp.mp4",
        foreign_stem.as_str(),
        ".video.mp4.shellx-cut-reservation.lock",
        ".video.mp4.shellx-cut-reserving",
    ];
    for name in foreign_names {
        let path = folder.path().join(name);
        std::fs::write(&path, b"theirs").unwrap();
        if !set_owner(&path, "NT AUTHORITY\\SYSTEM") {
            eprintln!("NOT RUN: cannot hand a file to another account here");
            return;
        }
    }
    let own_name = format!(".cut-pub-{dead}.1.tmp");
    let own_path = folder.path().join(&own_name);
    std::fs::write(&own_path, b"mine").unwrap();
    give_to_this_user(&own_path);
    let later = SystemTime::now() + std::time::Duration::from_secs(5);
    let removed = sweep_export_leftovers(folder.path(), later);
    assert_eq!(removed, vec![own_name]);
    for name in foreign_names {
        assert!(folder.path().join(name).exists(), "{name}");
    }
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

#[cfg(windows)]
#[test]
fn elevated_default_owner_export_leftovers_are_removed_without_handover() {
    let dir = tempfile::tempdir().unwrap();
    let exports = dir.path().join("exports");
    std::fs::create_dir_all(&exports).unwrap();
    let lock = exports.join(".elevated.mp4.shellx-cut-reservation.lock");
    let reserving = exports.join(".elevated.mp4.shellx-cut-reserving");
    std::fs::write(&lock, b"pid=0\n").unwrap();
    std::fs::write(&reserving, b"staging").unwrap();
    assert_eq!(sweep_export_leftovers(&exports, SystemTime::now()).len(), 2);
    assert!(
        !lock.exists(),
        "default-owner reservation lock must be removed without icacls handover"
    );
    assert!(
        !reserving.exists(),
        "default-owner reserving marker must be removed without icacls handover"
    );
}

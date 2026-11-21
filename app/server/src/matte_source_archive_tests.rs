use super::{FETCH_SRC_PY, MATANYONE_SRC_ROOT};
use sha2::{Digest, Sha256};
use std::process::Command;

#[cfg(target_os = "linux")]
#[test]
fn repeated_root_entries_are_refused_before_extraction() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("source.tar.gz");
    let dest = dir.path().join("installed-src");
    let extract_marker = dir.path().join("extract-called");
    let built = Command::new("python3")
        .arg("-c")
        .arg(
            "import io,sys,tarfile; p,root=sys.argv[1:]; t=tarfile.open(p,'w:gz'); \
             d=tarfile.TarInfo(root+'/'); d.type=tarfile.DIRTYPE; \
             [t.addfile(d) for _ in range(1500)]; \
             f=tarfile.TarInfo(root+'/hello.txt'); f.size=6; \
             t.addfile(f,io.BytesIO(b'pinned')); t.close()",
        )
        .arg(&archive)
        .arg(MATANYONE_SRC_ROOT)
        .output()
        .unwrap();
    assert!(built.status.success(), "{built:?}");

    let mut tree = Sha256::new();
    tree.update(b"hello.txt\0");
    tree.update(Sha256::digest(b"pinned"));
    let digest = format!("{:x}", tree.finalize());
    let guarded_script = format!(
        "import pathlib, tarfile\n_original_extractall = tarfile.TarFile.extractall\n\
         def checked_extractall(self, *args, **kwargs):\n    pathlib.Path({:?}).touch()\n    \
         return _original_extractall(self, *args, **kwargs)\n\
         tarfile.TarFile.extractall = checked_extractall\n{}",
        extract_marker.display().to_string(),
        FETCH_SRC_PY
    );
    let result = Command::new("python3")
        .arg("-c")
        .arg(guarded_script)
        .arg(format!("file://{}", archive.display()))
        .arg(&dest)
        .arg(digest)
        .arg("1048576")
        .arg("1048576")
        .arg(MATANYONE_SRC_ROOT)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(!result.status.success(), "archive was accepted: {result:?}");
    assert!(stderr.contains("MatAnyone2 source rejected: "), "{stderr}");
    assert!(
        stderr.contains("duplicate archive path") || stderr.contains("too many entries"),
        "{stderr}"
    );
    assert!(!extract_marker.exists(), "extraction started");
    assert!(!dest.exists(), "source was installed");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[cfg(target_os = "linux")]
#[test]
fn distinct_archive_entries_obey_the_1000_member_limit() {
    let mut tree = Sha256::new();
    tree.update(b"hello.txt\0");
    tree.update(Sha256::digest(b"pinned"));
    let digest = format!("{:x}", tree.finalize());

    for (child_dirs, accepted) in [(998, true), (999, false)] {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("source.tar.gz");
        let dest = dir.path().join("installed-src");
        let extract_marker = dir.path().join("extract-called");
        let built = Command::new("python3")
            .arg("-c")
            .arg(
                r#"import io,sys,tarfile
path,root,count=sys.argv[1:]
with tarfile.open(path,'w:gz') as archive:
    directory=tarfile.TarInfo(root+'/'); directory.type=tarfile.DIRTYPE; archive.addfile(directory)
    for index in range(int(count)):
        directory=tarfile.TarInfo(f'{root}/d{index:04}/'); directory.type=tarfile.DIRTYPE; archive.addfile(directory)
    source=tarfile.TarInfo(root+'/hello.txt'); source.size=6
    archive.addfile(source,io.BytesIO(b'pinned'))"#,
            )
            .arg(&archive)
            .arg(MATANYONE_SRC_ROOT)
            .arg(child_dirs.to_string())
            .output()
            .unwrap();
        assert!(built.status.success(), "{built:?}");

        let guarded_script = format!(
            "import pathlib, tarfile\n_original_extractall = tarfile.TarFile.extractall\n\
             def checked_extractall(self, *args, **kwargs):\n    pathlib.Path({:?}).touch()\n    \
             return _original_extractall(self, *args, **kwargs)\n\
             tarfile.TarFile.extractall = checked_extractall\n{}",
            extract_marker.display().to_string(),
            FETCH_SRC_PY
        );
        let result = Command::new("python3")
            .arg("-c")
            .arg(guarded_script)
            .arg(format!("file://{}", archive.display()))
            .arg(&dest)
            .arg(&digest)
            .arg("1048576")
            .arg("1048576")
            .arg(MATANYONE_SRC_ROOT)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&result.stderr);
        if accepted {
            assert!(result.status.success(), "{result:?}");
            assert!(extract_marker.exists(), "extraction did not start");
            assert_eq!(std::fs::read(dest.join("hello.txt")).unwrap(), b"pinned");
            assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 999);
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 3);
        } else {
            assert!(!result.status.success(), "archive was accepted: {result:?}");
            assert!(stderr.contains("archive has too many entries"), "{stderr}");
            assert!(!extract_marker.exists(), "extraction started");
            assert!(!dest.exists(), "source was installed");
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }
}

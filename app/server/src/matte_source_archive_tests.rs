use super::{source_archive::SAM2_SRC_ROOT, FETCH_SRC_PY, MATANYONE_SRC_ROOT};
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

#[cfg(target_os = "linux")]
#[test]
fn sam2_archive_with_internal_symlinks_materializes_verified_files() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("sam2-symlinks.tar.gz");
    let dest = dir.path().join("installed-sam2");
    let cfg_body = b"model:\n  _target_: sam2.modeling.sam2_base.SAM2Base\n";
    let built = Command::new("python3")
        .arg("-c")
        .arg(
            r#"import io, sys, tarfile
path, root, body = sys.argv[1], sys.argv[2], sys.argv[3].encode()
with tarfile.open(path, 'w:gz') as archive:
    for d in [root, root + '/sam2', root + '/sam2/configs', root + '/sam2/configs/sam2']:
        di = tarfile.TarInfo(d)
        di.type = tarfile.DIRTYPE
        archive.addfile(di)
    fi = tarfile.TarInfo(root + '/sam2/configs/sam2/sam2_hiera_b+.yaml')
    fi.size = len(body)
    archive.addfile(fi, io.BytesIO(body))
    si = tarfile.TarInfo(root + '/sam2/sam2_hiera_b+.yaml')
    si.type = tarfile.SYMTYPE
    si.linkname = 'configs/sam2/sam2_hiera_b+.yaml'
    archive.addfile(si)
"#,
        )
        .arg(&archive)
        .arg(SAM2_SRC_ROOT)
        .arg(std::str::from_utf8(cfg_body).unwrap())
        .output()
        .unwrap();
    assert!(built.status.success(), "{built:?}");

    let mut tree = Sha256::new();
    tree.update(b"sam2/configs/sam2/sam2_hiera_b+.yaml\0");
    tree.update(Sha256::digest(cfg_body));
    tree.update(b"sam2/sam2_hiera_b+.yaml\0symlink\0sam2/configs/sam2/sam2_hiera_b+.yaml");
    let digest = format!("{:x}", tree.finalize());

    // Disallowed mode (MatAnyone2 default) must still reject symlinks.
    let rejected = Command::new("python3")
        .arg("-c")
        .arg(FETCH_SRC_PY)
        .arg(format!("file://{}", archive.display()))
        .arg(&dest)
        .arg(&digest)
        .arg("1048576")
        .arg("1048576")
        .arg(SAM2_SRC_ROOT)
        .arg("MatAnyone2")
        .arg("0")
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("unexpected archive member type"),
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );

    // SAM2 mode verifies the symlink target and materializes it as a regular file (no os.symlink).
    let accepted = Command::new("python3")
        .arg("-c")
        .arg(FETCH_SRC_PY)
        .arg(format!("file://{}", archive.display()))
        .arg(&dest)
        .arg(&digest)
        .arg("1048576")
        .arg("1048576")
        .arg(SAM2_SRC_ROOT)
        .arg("SAM2")
        .arg("1")
        .output()
        .unwrap();
    assert!(
        accepted.status.success(),
        "SAM2 archive with internal symlinks was rejected: {}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let materialized = dest.join("sam2/sam2_hiera_b+.yaml");
    assert!(
        !std::fs::symlink_metadata(&materialized)
            .unwrap()
            .file_type()
            .is_symlink(),
        "materialized file must be a regular copy, not a symlink"
    );
    assert_eq!(std::fs::read(&materialized).unwrap(), cfg_body);
}

#[cfg(target_os = "linux")]
#[test]
fn sam2_archive_rejects_escaping_missing_and_chained_symlinks_before_extraction() {
    let cfg_body = b"model:\n  _target_: sam2.modeling.sam2_base.SAM2Base\n";
    let mut tree = Sha256::new();
    tree.update(b"sam2/configs/sam2/sam2_hiera_b+.yaml\0");
    tree.update(Sha256::digest(cfg_body));
    let dummy_digest = format!("{:x}", tree.finalize());

    for (link_path, link_target, expected_err) in [
        (
            "sam2/escape.yaml",
            "../../../etc/passwd",
            "symlink escapes archive root",
        ),
        ("sam2/abs.yaml", "/etc/passwd", "unexpected symlink target"),
        (
            "sam2/missing.yaml",
            "configs/sam2/missing.yaml",
            "symlink target is not a regular file",
        ),
        (
            "sam2/dirlink",
            "configs/sam2",
            "symlink target is not a regular file",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("bad-sam2.tar.gz");
        let dest = dir.path().join("installed-sam2");
        let extract_marker = dir.path().join("extract-called");
        let built = Command::new("python3")
            .arg("-c")
            .arg(
                r#"import io, sys, tarfile
path, root, body, lpath, ltarget = sys.argv[1], sys.argv[2], sys.argv[3].encode(), sys.argv[4], sys.argv[5]
with tarfile.open(path, 'w:gz') as archive:
    for d in [root, root + '/sam2', root + '/sam2/configs', root + '/sam2/configs/sam2']:
        di = tarfile.TarInfo(d)
        di.type = tarfile.DIRTYPE
        archive.addfile(di)
    fi = tarfile.TarInfo(root + '/sam2/configs/sam2/sam2_hiera_b+.yaml')
    fi.size = len(body)
    archive.addfile(fi, io.BytesIO(body))
    si = tarfile.TarInfo(root + '/' + lpath)
    si.type = tarfile.SYMTYPE
    si.linkname = ltarget
    archive.addfile(si)
"#,
            )
            .arg(&archive)
            .arg(SAM2_SRC_ROOT)
            .arg(std::str::from_utf8(cfg_body).unwrap())
            .arg(link_path)
            .arg(link_target)
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
            .arg(&dummy_digest)
            .arg("1048576")
            .arg("1048576")
            .arg(SAM2_SRC_ROOT)
            .arg("SAM2")
            .arg("1")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(
            !result.status.success(),
            "bad symlink was accepted: {link_path} -> {link_target}"
        );
        assert!(stderr.contains("SAM2 source rejected: "), "{stderr}");
        assert!(
            stderr.contains(expected_err),
            "expected {expected_err}, got: {stderr}"
        );
        assert!(
            !extract_marker.exists(),
            "extraction started for bad symlink"
        );
        assert!(!dest.exists());
    }
}

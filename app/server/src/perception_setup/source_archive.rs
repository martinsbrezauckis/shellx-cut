//! Pinned GitHub source archives for the optional MatAnyone2/SAM2 premium matte runtime.

use super::{check_cancelled, run_streaming, ProgressFn};
use cut_core::{error_codes, CutError};
use std::path::{Path, PathBuf};

/// Pinned source archive fetched over HTTPS and verified by tree SHA-256 and size caps.
pub(super) struct SourceArchiveSpec {
    pub label: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub root: &'static str,
    pub archive_max_bytes: &'static str,
    pub source_max_bytes: &'static str,
    pub allow_internal_symlinks: bool,
}

/// The matanyone2 source, pinned to a validated commit. We install it EDITABLE
/// from a local extraction (NOT `git+`): the upstream wheel build hits a hatchling
/// `force-include` bug (it duplicates `matanyone2/config/__init__.py`), and an
/// editable install from a local dir skips the wheel build entirely.
pub(super) const MATANYONE_SRC_URL: &str =
    "https://github.com/pq-yang/MatAnyone2/archive/e3370127319c63a6dc8a49c69de2d41d90137f91.tar.gz";
pub(super) const MATANYONE_SRC_SHA256: &str =
    "2dc341b915d8cc887e3b889fa9d03f950f6de7c13cee800bee9748d5aa3c7e18";
pub(super) const MATANYONE_SRC_ROOT: &str = "MatAnyone2-e3370127319c63a6dc8a49c69de2d41d90137f91";
pub(super) const MATANYONE_SRC_MAX_BYTES: &str = "33554432";

pub(super) const MATANYONE_SRC: SourceArchiveSpec = SourceArchiveSpec {
    label: "MatAnyone2",
    url: MATANYONE_SRC_URL,
    sha256: MATANYONE_SRC_SHA256,
    root: MATANYONE_SRC_ROOT,
    archive_max_bytes: MATANYONE_SRC_MAX_BYTES,
    source_max_bytes: MATANYONE_SRC_MAX_BYTES,
    allow_internal_symlinks: false,
};

/// SAM2 (Apache-2.0) — the click-to-pick-subject seed for the premium matte.
/// Pinned to the same commit as a vendor tarball so installation never needs Git.
pub(super) const SAM2_SRC_ROOT: &str = "sam2-2b90b9f5ceec907a1c18123530e92e794ad901a4";
pub(super) const SAM2_SRC_MAX_BYTES: &str = "100663296";

pub(super) const SAM2_SRC: SourceArchiveSpec = SourceArchiveSpec {
    label: "SAM2",
    url: "https://github.com/facebookresearch/sam2/archive/2b90b9f5ceec907a1c18123530e92e794ad901a4.tar.gz",
    sha256: "68fc21ebc17c7cadaa2b30ae4b3f93dbaedd694ac0744cdae8d4e82e4472b39d",
    root: SAM2_SRC_ROOT,
    archive_max_bytes: SAM2_SRC_MAX_BYTES,
    source_max_bytes: SAM2_SRC_MAX_BYTES,
    allow_internal_symlinks: true,
};

/// Check the pinned source before extraction with the venv's Python stdlib.
/// When `allow_symlinks` is enabled (`"1"`), internal relative symlinks that
/// resolve strictly inside `root` to an archive regular file are pinned in the
/// tree hash and materialized as regular file copies without calling `os.symlink`.
pub(super) const FETCH_SRC_PY: &str = r#"
import hashlib, os, posixpath, shutil, sys, tarfile, tempfile, urllib.request
if len(sys.argv) == 7:
    url, dest, expected, archive_limit, source_limit, root = sys.argv[1:]
    label, allow_symlinks = 'MatAnyone2', False
elif len(sys.argv) == 9:
    url, dest, expected, archive_limit, source_limit, root, label, allow_flag = sys.argv[1:]
    if allow_flag not in ('0', '1'):
        raise SystemExit(label + ' source rejected: invalid symlink mode')
    allow_symlinks = allow_flag == '1'
else:
    raise SystemExit('source rejected: unexpected argument count')
archive_limit, source_limit = int(archive_limit), int(source_limit)

def reject(reason):
    raise SystemExit(label + ' source rejected: ' + reason)

parent = os.path.dirname(os.path.abspath(dest))
os.makedirs(parent, exist_ok=True)
with tempfile.TemporaryDirectory(prefix='.s-', dir=parent) as td:
    tgz = os.path.join(td, 'source.tar.gz')
    size = 0
    with urllib.request.urlopen(url, timeout=60) as response, open(tgz, 'xb') as output:
        while chunk := response.read(65536):
            size += len(chunk)
            if size > archive_limit:
                reject('compressed download exceeds the size limit')
            output.write(chunk)

    records, seen, members = [], set(), []
    file_sizes, symlinks = {}, []
    extracted = 0
    with tarfile.open(tgz, 'r:gz') as archive:
        for member in archive:
            if len(members) >= 1000:
                reject('archive has too many entries')
            members.append(member)
            name = member.name.rstrip('/') if member.isdir() else member.name
            if name == root and member.isdir():
                relative = ''
            else:
                parts = name.split('/')
                if (len(parts) < 2 or parts[0] != root or
                        any(part in ('', '.', '..') for part in parts[1:]) or
                        '\\' in name or ':' in name):
                    reject('unexpected archive layout: ' + name)
                relative = '/'.join(parts[1:])
            if relative.casefold() in seen:
                reject('duplicate archive path: ' + (relative or root))
            seen.add(relative.casefold())
            if member.isdir():
                continue
            if member.issym():
                if not allow_symlinks:
                    reject('unexpected archive member type: ' + relative)
                if (not member.linkname or '\\' in member.linkname or ':' in member.linkname or
                        member.linkname.startswith('/')):
                    reject('unexpected symlink target: ' + relative)
                link_parent = posixpath.dirname(relative)
                joined = posixpath.join(link_parent, member.linkname) if link_parent else member.linkname
                target_rel = posixpath.normpath(joined)
                target_parts = target_rel.split('/')
                if (target_rel in ('', '.', '..') or
                        any(part in ('', '.', '..') for part in target_parts)):
                    reject('symlink escapes archive root: ' + relative)
                symlinks.append((relative, target_rel))
                continue
            if not member.isfile():
                reject('unexpected archive member type: ' + relative)
            if member.size > source_limit - extracted:
                reject('extracted source exceeds the size limit')
            file_hash = hashlib.sha256()
            actual = 0
            with archive.extractfile(member) as source:
                while chunk := source.read(65536):
                    actual += len(chunk)
                    if actual > member.size:
                        reject('archive member exceeds its declared size')
                    file_hash.update(chunk)
            if actual != member.size:
                reject('archive member is incomplete')
            extracted += actual
            file_sizes[relative] = actual
            records.append((relative, file_hash.digest()))
        for relative, target_rel in symlinks:
            if target_rel not in file_sizes:
                reject('symlink target is not a regular file: ' + relative)
            target_size = file_sizes[target_rel]
            if target_size > source_limit - extracted:
                reject('extracted source exceeds the size limit')
            extracted += target_size
            records.append((relative, b'symlink\0' + target_rel.encode('utf-8')))
        tree = hashlib.sha256()
        for relative, digest in sorted(records):
            tree.update(relative.encode('utf-8') + b'\0' + digest)
        if tree.hexdigest() != expected:
            reject('source SHA-256 mismatch: expected ' + expected + ', got ' + tree.hexdigest())
        extract_members = [m for m in members if not m.issym()]
        archive.extractall(td, members=extract_members, filter='data')
        for relative, target_rel in symlinks:
            dst_path = os.path.join(td, root, *relative.split('/'))
            src_path = os.path.join(td, root, *target_rel.split('/'))
            os.makedirs(os.path.dirname(dst_path), exist_ok=True)
            shutil.copyfile(src_path, dst_path)
    if os.path.lexists(dest):
        shutil.rmtree(dest)
    shutil.move(os.path.join(td, root), dest)
print('extracted', dest)
"#;

pub(super) fn fetch_verified_source_archive(
    python: &Path,
    spec: &SourceArchiveSpec,
    dest: &Path,
    progress: &ProgressFn,
    band_lo: f32,
    band_hi: f32,
) -> Result<(), CutError> {
    let dest_str = dest.to_str().ok_or_else(|| {
        CutError::new(
            error_codes::IO,
            format!("{} source destination path is not valid text", spec.label),
            dest.display().to_string(),
        )
    })?;
    let label_lower = spec.label.to_ascii_lowercase();
    let step_label = format!("verify {label_lower} source");
    let allow_symlinks = if spec.allow_internal_symlinks {
        "1"
    } else {
        "0"
    };
    run_streaming(
        python,
        &[
            "-c",
            FETCH_SRC_PY,
            spec.url,
            dest_str,
            spec.sha256,
            spec.archive_max_bytes,
            spec.source_max_bytes,
            spec.root,
            spec.label,
            allow_symlinks,
        ],
        &step_label,
        progress,
        band_lo,
        band_hi,
    )
    .map_err(|e| {
        CutError::new(
            error_codes::JOB_FAILED,
            "Premium background removal could not verify its source; nothing was installed",
            format!("{}: {}", e.message, e.cause),
        )
        .with_suggested_action("Try installing premium background removal again")
    })
}

/// Remove a staged source directory when the step finishes or fails.
struct TempSourceDir(PathBuf);

impl Drop for TempSourceDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Download and verify the pinned SAM2 source archive, then install it into the
/// premium venv without invoking Git.
pub(super) fn install_sam2_from_spec(
    venv_python: &Path,
    uv: &Path,
    venv_python_str: &str,
    matanyone_dir: &Path,
    spec: &SourceArchiveSpec,
    progress: &ProgressFn,
) -> Result<(), CutError> {
    let src_dir = matanyone_dir.join("s2");
    let _cleanup = TempSourceDir(src_dir.clone());
    let src_str = src_dir.to_str().ok_or_else(|| {
        CutError::new(
            error_codes::IO,
            "SAM2 source destination path is not valid text",
            src_dir.display().to_string(),
        )
    })?;
    progress(0.88, "verifying SAM2 source");
    fetch_verified_source_archive(venv_python, spec, &src_dir, progress, 0.88, 0.90)?;
    check_cancelled()?;
    progress(0.90, "installing SAM2 (pick-which-subject)");
    run_streaming(
        uv,
        &[
            "pip",
            "install",
            "--python",
            venv_python_str,
            "--no-build-isolation",
            src_str,
        ],
        "uv pip install sam2",
        progress,
        0.90,
        0.92,
    )
}

#[cfg(test)]
mod pin_tests {
    use super::*;

    #[test]
    fn sam2_source_pin_uses_vendor_archive_not_git() {
        assert_eq!(
            SAM2_SRC.url,
            "https://github.com/facebookresearch/sam2/archive/2b90b9f5ceec907a1c18123530e92e794ad901a4.tar.gz"
        );
        assert!(!SAM2_SRC.url.starts_with("git+"));
        assert_eq!(
            SAM2_SRC.root,
            "sam2-2b90b9f5ceec907a1c18123530e92e794ad901a4"
        );
        assert_eq!(SAM2_SRC.sha256.len(), 64);
        assert!(SAM2_SRC
            .sha256
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        for (spec, expected_symlinks) in [(&SAM2_SRC, true), (&MATANYONE_SRC, false)] {
            assert_eq!(spec.allow_internal_symlinks, expected_symlinks);
            assert!(spec.url.starts_with("https://github.com/"));
            assert!(spec.url.ends_with(".tar.gz"));
            assert!(!spec.url.contains("git+"));
            assert_eq!(spec.sha256.len(), 64);
            assert!(spec
                .sha256
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    fn serve_once(body: Vec<u8>) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/sam2.tar.gz", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut req = [0u8; 1024];
            let _ = stream.read(&mut req);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        });
        (url, handle)
    }

    fn build_sam2_fixture_archive(path: &Path) -> &'static str {
        let setup_py = b"from setuptools import setup\nsetup(name='SAM-2', version='1.0')\n";
        let yaml_cfg = b"model:\n  _target_: sam2.modeling.sam2_base.SAM2Base\n";
        let built = Command::new("python3")
            .arg("-c")
            .arg(
                r#"import io, sys, tarfile
archive_path, root = sys.argv[1], sys.argv[2]
setup_py = b"from setuptools import setup\nsetup(name='SAM-2', version='1.0')\n"
yaml_cfg = b"model:\n  _target_: sam2.modeling.sam2_base.SAM2Base\n"
with tarfile.open(archive_path, "w:gz") as tar:
    for d in [root, f"{root}/sam2", f"{root}/sam2/configs", f"{root}/sam2/configs/sam2"]:
        di = tarfile.TarInfo(d)
        di.type = tarfile.DIRTYPE
        tar.addfile(di)
    for rel, body in [("setup.py", setup_py), ("sam2/configs/sam2/sam2_hiera_b+.yaml", yaml_cfg)]:
        fi = tarfile.TarInfo(f"{root}/{rel}")
        fi.size = len(body)
        tar.addfile(fi, io.BytesIO(body))
    si = tarfile.TarInfo(f"{root}/sam2/sam2_hiera_b+.yaml")
    si.type = tarfile.SYMTYPE
    si.linkname = "configs/sam2/sam2_hiera_b+.yaml"
    tar.addfile(si)
"#,
            )
            .arg(path)
            .arg(SAM2_SRC_ROOT)
            .output()
            .unwrap();
        assert!(built.status.success(), "{built:?}");

        let mut tree = Sha256::new();
        tree.update(b"sam2/configs/sam2/sam2_hiera_b+.yaml\0");
        tree.update(Sha256::digest(yaml_cfg));
        tree.update(b"sam2/sam2_hiera_b+.yaml\0symlink\0sam2/configs/sam2/sam2_hiera_b+.yaml");
        tree.update(b"setup.py\0");
        tree.update(Sha256::digest(setup_py));
        Box::leak(format!("{:x}", tree.finalize()).into_boxed_str())
    }

    fn write_fake_uv(path: &Path, log_path: &Path, exit_code: i32) {
        std::fs::write(
            path,
            format!(
                r#"#!/bin/sh
for a in "$@"; do
  printf '%s\n' "$a" >> "{}"
  case "$a" in
    git+*)
      echo "error: Git operation failed" >&2
      echo "cause: Git executable not found. Ensure that Git is installed and available." >&2
      exit 1
      ;;
  esac
done
src=""
for a in "$@"; do src="$a"; done
[ -f "$src/setup.py" ] || {{ echo "missing setup.py in $src" >&2; exit 2; }}
[ -f "$src/sam2/sam2_hiera_b+.yaml" ] || {{ echo "missing materialized yaml in $src" >&2; exit 2; }}
[ ! -L "$src/sam2/sam2_hiera_b+.yaml" ] || {{ echo "symlink must be materialized as regular file" >&2; exit 2; }}
exit {}
"#,
                log_path.display(),
                exit_code
            ),
        )
        .unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn sam2_install_succeeds_without_git_using_pinned_archive() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("sam2-fixture.tar.gz");
        let digest = build_sam2_fixture_archive(&archive);

        let (url, server) = serve_once(std::fs::read(&archive).unwrap());
        let url_static: &'static str = Box::leak(url.into_boxed_str());

        let fake_uv = dir.path().join("uv");
        let uv_log = dir.path().join("uv-args.txt");
        write_fake_uv(&fake_uv, &uv_log, 0);

        let matanyone_dir = dir.path().join("matanyone");
        std::fs::create_dir_all(&matanyone_dir).unwrap();
        let py = PathBuf::from("/usr/bin/python3");
        let spec = SourceArchiveSpec {
            label: "SAM2",
            url: url_static,
            sha256: digest,
            root: SAM2_SRC_ROOT,
            archive_max_bytes: SAM2_SRC_MAX_BYTES,
            source_max_bytes: SAM2_SRC_MAX_BYTES,
            allow_internal_symlinks: true,
        };
        let progress = |_: f32, _: &str| {};
        let res = install_sam2_from_spec(
            &py,
            &fake_uv,
            py.to_str().unwrap(),
            &matanyone_dir,
            &spec,
            &progress,
        );
        server.join().unwrap();
        assert!(res.is_ok(), "SAM2 install failed without git: {res:?}");
        let args = std::fs::read_to_string(&uv_log).unwrap();
        assert!(!args.contains("git+"), "uv must not receive git+: {args}");
        assert!(args.contains("--no-build-isolation"), "{args}");
        assert!(
            !matanyone_dir.join("s2").exists(),
            "temporary SAM2 source dir must be cleaned up after install"
        );
    }

    #[test]
    fn sam2_install_rejects_tampered_archive_before_invoking_uv() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("sam2-fixture.tar.gz");
        let _ = build_sam2_fixture_archive(&archive);
        let (url, server) = serve_once(std::fs::read(&archive).unwrap());
        let url_static: &'static str = Box::leak(url.into_boxed_str());
        let bad_digest: &'static str = Box::leak("0".repeat(64).into_boxed_str());

        let fake_uv = dir.path().join("uv");
        let uv_log = dir.path().join("uv-args.txt");
        write_fake_uv(&fake_uv, &uv_log, 0);

        let matanyone_dir = dir.path().join("matanyone");
        std::fs::create_dir_all(&matanyone_dir).unwrap();
        let py = PathBuf::from("/usr/bin/python3");
        let spec = SourceArchiveSpec {
            label: "SAM2",
            url: url_static,
            sha256: bad_digest,
            root: SAM2_SRC_ROOT,
            archive_max_bytes: SAM2_SRC_MAX_BYTES,
            source_max_bytes: SAM2_SRC_MAX_BYTES,
            allow_internal_symlinks: true,
        };
        let progress = |_: f32, _: &str| {};
        let err = install_sam2_from_spec(
            &py,
            &fake_uv,
            py.to_str().unwrap(),
            &matanyone_dir,
            &spec,
            &progress,
        )
        .unwrap_err();
        server.join().unwrap();
        assert!(err.message.contains("could not verify its source"), "{err}");
        assert!(err.cause.contains("SHA-256 mismatch"), "{err}");
        assert!(!uv_log.exists(), "uv must not run on unverified archive");
        assert!(!matanyone_dir.join("s2").exists());
    }

    #[test]
    fn sam2_install_cleans_up_staged_source_when_uv_fails() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("sam2-fixture.tar.gz");
        let digest = build_sam2_fixture_archive(&archive);
        let (url, server) = serve_once(std::fs::read(&archive).unwrap());
        let url_static: &'static str = Box::leak(url.into_boxed_str());

        let fake_uv = dir.path().join("uv");
        let uv_log = dir.path().join("uv-args.txt");
        write_fake_uv(&fake_uv, &uv_log, 7);

        let matanyone_dir = dir.path().join("matanyone");
        std::fs::create_dir_all(&matanyone_dir).unwrap();
        let py = PathBuf::from("/usr/bin/python3");
        let spec = SourceArchiveSpec {
            label: "SAM2",
            url: url_static,
            sha256: digest,
            root: SAM2_SRC_ROOT,
            archive_max_bytes: SAM2_SRC_MAX_BYTES,
            source_max_bytes: SAM2_SRC_MAX_BYTES,
            allow_internal_symlinks: true,
        };
        let progress = |_: f32, _: &str| {};
        let err = install_sam2_from_spec(
            &py,
            &fake_uv,
            py.to_str().unwrap(),
            &matanyone_dir,
            &spec,
            &progress,
        )
        .unwrap_err();
        server.join().unwrap();
        assert!(err.message.contains("code 7"), "{err}");
        assert!(uv_log.exists());
        assert!(
            !matanyone_dir.join("s2").exists(),
            "temporary SAM2 source dir must be removed when uv fails"
        );
    }
}

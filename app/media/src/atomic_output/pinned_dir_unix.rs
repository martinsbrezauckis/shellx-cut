//! Unix (`O_DIRECTORY | O_NOFOLLOW` + `openat`) parent-directory pinning and
//! parent-swap verification for export publication.

use super::reserved::everyone_can_replace_files;
use cut_core::{error_codes, CutError};
use std::ffi::{CString, OsStr, OsString};
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

fn output_folder_error(folder: &Path, error: &std::io::Error) -> CutError {
    crate::folder_access::folder_error("output folder", folder, error)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DirIdentity {
    dev: u64,
    ino: u64,
}

pub(super) struct UnixPinnedDir {
    fd: OwnedFd,
    identity: DirIdentity,
    chain: Vec<(OsString, DirIdentity)>,
}

fn to_cstring(s: &OsStr) -> Result<CString, CutError> {
    CString::new(s.as_bytes()).map_err(|_| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "path component contains an interior NUL byte",
            "Unix paths cannot contain NUL",
        )
    })
}

fn validate_single_component(name: &OsStr) -> Result<CString, CutError> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "expected a single safe directory entry name",
            name.to_string_lossy().into_owned(),
        ));
    }
    to_cstring(name)
}

#[allow(clippy::unnecessary_cast)]
fn dir_identity_from_stat(stat: &libc::stat) -> Result<DirIdentity, CutError> {
    if (stat.st_mode as u32 & libc::S_IFMT as u32) != (libc::S_IFDIR as u32) {
        return Err(CutError::new(
            error_codes::IO,
            "output directory is not a local plain directory",
            "expected a directory",
        ));
    }
    Ok(DirIdentity {
        dev: stat.st_dev as u64,
        ino: stat.st_ino as u64,
    })
}

pub(super) fn fstat_fd(fd: RawFd) -> std::io::Result<libc::stat> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::zeroed();
    if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { stat.assume_init() })
}

fn fstatat_nofollow(parent_fd: RawFd, c_name: &CString) -> Option<libc::stat> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::zeroed();
    let rc = unsafe {
        libc::fstatat(
            parent_fd,
            c_name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    (rc == 0).then(|| unsafe { stat.assume_init() })
}

#[allow(clippy::unnecessary_cast)]
fn open_component_nofollow(
    parent_fd: RawFd,
    name: &OsStr,
    depth: usize,
    total: usize,
    sofar: &Path,
) -> Result<(OwnedFd, DirIdentity, libc::stat), CutError> {
    let c_name = to_cstring(name)?;
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let raw = unsafe { libc::openat(parent_fd, c_name.as_ptr(), flags) };
    if raw >= 0 {
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let stat = fstat_fd(fd.as_raw_fd())?;
        return Ok((fd, dir_identity_from_stat(&stat)?, stat));
    }
    let err = std::io::Error::last_os_error();
    let raw_err = err.raw_os_error();
    let is_symlink = raw_err == Some(libc::ELOOP)
        || fstatat_nofollow(parent_fd, &c_name)
            .is_some_and(|st| (st.st_mode as u32 & libc::S_IFMT as u32) == (libc::S_IFLNK as u32));
    if is_symlink {
        let link_uid = fstatat_nofollow(parent_fd, &c_name).map(|st| st.st_uid);
        // A link the current user made above the export folder (a linked
        // Videos or project folder) is followed as in 0.6.121: the directory it
        // names is pinned, and a later swap is caught by `verify_still_at_path`.
        // A link owned by another user is refused: that is the redirect.
        if depth < total && link_uid == Some(unsafe { libc::geteuid() }) {
            let own_raw = unsafe {
                libc::openat(
                    parent_fd,
                    c_name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
                )
            };
            if own_raw >= 0 {
                let fd = unsafe { OwnedFd::from_raw_fd(own_raw) };
                let stat = fstat_fd(fd.as_raw_fd())?;
                return Ok((fd, dir_identity_from_stat(&stat)?, stat));
            }
        }
        // OS-level depth-1 root-owned symlinks (macOS `/var -> /private/var`,
        // `/tmp -> /private/tmp`; Fedora `/home -> /var/home`) are permitted
        // only when both the link and its target directory belong to root (uid 0).
        if depth == 1 && link_uid == Some(0) {
            let os_raw = unsafe {
                libc::openat(
                    parent_fd,
                    c_name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
                )
            };
            if os_raw >= 0 {
                let fd = unsafe { OwnedFd::from_raw_fd(os_raw) };
                let stat = fstat_fd(fd.as_raw_fd())?;
                if stat.st_uid == 0 && !everyone_can_replace_files(stat.st_mode as u32) {
                    return Ok((fd, dir_identity_from_stat(&stat)?, stat));
                }
            }
        }
        let detail = if depth == total {
            ""
        } else {
            " (a link owned by another user is refused)"
        };
        return Err(CutError::new(
            error_codes::IO,
            format!(
                "output directory is not a local plain directory{detail}: {}",
                sofar.display()
            ),
            "an export folder that is itself a link, or that is reached through a link another user owns, could send the file somewhere else",
        )
        .with_suggested_action(
            "nothing was exported. Export into the real folder the link points to, or choose another export folder",
        ));
    }
    if raw_err == Some(libc::ENOTDIR) {
        return Err(CutError::new(
            error_codes::IO,
            format!(
                "output directory is not a local plain directory: {}",
                sofar.display()
            ),
            "expected a directory",
        ));
    }
    Err(output_folder_error(sofar, &err))
}

fn walk_no_follow(path: &Path) -> Result<(UnixPinnedDir, libc::stat), CutError> {
    let root_c = CString::new("/").unwrap();
    let root_raw = unsafe {
        libc::open(
            root_c.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if root_raw < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut cur_fd = unsafe { OwnedFd::from_raw_fd(root_raw) };
    let mut cur_stat = fstat_fd(cur_fd.as_raw_fd())?;
    let mut cur_id = dir_identity_from_stat(&cur_stat)?;

    let normals: Vec<OsString> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_os_string()),
            _ => None,
        })
        .collect();
    let total = normals.len();
    let mut chain = Vec::with_capacity(total);
    let mut sofar = PathBuf::from("/");

    for (idx, part) in normals.into_iter().enumerate() {
        sofar.push(&part);
        let (next_fd, next_id, next_stat) =
            open_component_nofollow(cur_fd.as_raw_fd(), &part, idx + 1, total, &sofar)?;
        chain.push((part, next_id));
        cur_fd = next_fd;
        cur_id = next_id;
        cur_stat = next_stat;
    }

    Ok((
        UnixPinnedDir {
            fd: cur_fd,
            identity: cur_id,
            chain,
        },
        cur_stat,
    ))
}

#[allow(clippy::unnecessary_cast)]
fn harden_owned_project_dir_fd(
    path: &Path,
    fd: &OwnedFd,
    opened: &libc::stat,
) -> Result<libc::stat, CutError> {
    let in_project = path
        .ancestors()
        .any(|a| a.extension().is_some_and(|e| e == "cutproj"));
    let euid = unsafe { libc::geteuid() };
    if !in_project || opened.st_uid != euid || (opened.st_mode as u32 & 0o022) == 0 {
        return Ok(*opened);
    }
    let mode = ((opened.st_mode as u32) & 0o7777) & !0o022;
    if unsafe { libc::fchmod(fd.as_raw_fd(), mode as libc::mode_t) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let hardened = fstat_fd(fd.as_raw_fd())?;
    if (hardened.st_mode as u32 & 0o022) != 0 {
        return Err(CutError::new(
            error_codes::IO,
            format!(
                "project output directory could not be secured: {}",
                path.display()
            ),
            "export publication requires a stable directory protected from external writes",
        ));
    }
    Ok(hardened)
}

impl UnixPinnedDir {
    #[allow(clippy::unnecessary_cast)]
    pub(super) fn open(normalized_path: &Path, harden_project: bool) -> Result<Self, CutError> {
        let (pinned, mut stat) = walk_no_follow(normalized_path)?;
        if harden_project {
            stat = harden_owned_project_dir_fd(normalized_path, &pinned.fd, &stat)?;
        }
        if everyone_can_replace_files(stat.st_mode as u32) {
            return Err(CutError::new(
                error_codes::IO,
                format!(
                    "output directory can be written by anyone on this computer: {}",
                    normalized_path.display()
                ),
                "export destination must not be writable by everyone without the sticky bit",
            ));
        }
        Ok(pinned)
    }

    pub(super) fn verify_still_at_path(&self, normalized_path: &Path) -> Result<(), CutError> {
        let live_id = dir_identity_from_stat(&fstat_fd(self.fd.as_raw_fd())?)?;
        if live_id != self.identity {
            return Err(CutError::new(
                error_codes::IO,
                format!(
                    "export directory identity changed while exporting: {}",
                    normalized_path.display()
                ),
                "refusing publication into a replaced directory handle",
            ));
        }
        let (current, _) = walk_no_follow(normalized_path)?;
        if current.identity != self.identity || current.chain != self.chain {
            return Err(CutError::new(
                error_codes::IO,
                format!(
                    "export directory parent was moved or replaced while exporting: {}",
                    normalized_path.display()
                ),
                "export publication refuses a parent directory that changed mid-export",
            ));
        }
        Ok(())
    }

    pub(super) fn raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    #[allow(clippy::unnecessary_cast)]
    pub(super) fn clear_stale_empty_child(&self, name: &OsStr) -> Result<(), CutError> {
        let c_name = validate_single_component(name)?;
        let Some(stat) = fstatat_nofollow(self.fd.as_raw_fd(), &c_name) else {
            let err = std::io::Error::last_os_error();
            return if err.kind() == std::io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(err.into())
            };
        };
        if (stat.st_mode as u32 & libc::S_IFMT as u32) == (libc::S_IFREG as u32)
            && stat.st_size == 0
            && unsafe { libc::unlinkat(self.fd.as_raw_fd(), c_name.as_ptr(), 0) } != 0
        {
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::NotFound {
                return Err(err.into());
            }
        }
        Ok(())
    }

    pub(super) fn rename_from_stage(
        &self,
        stage_fd: Option<RawFd>,
        stage_leaf_or_path: &OsStr,
        target_name: &OsStr,
    ) -> std::io::Result<()> {
        let to_io = |e: CutError| std::io::Error::new(std::io::ErrorKind::InvalidInput, e.message);
        let c_target = to_cstring(target_name).map_err(to_io)?;
        let (from_fd, c_from) = match stage_fd {
            Some(fd) => (
                fd,
                validate_single_component(stage_leaf_or_path).map_err(to_io)?,
            ),
            None => (
                libc::AT_FDCWD,
                to_cstring(stage_leaf_or_path).map_err(to_io)?,
            ),
        };
        if unsafe {
            libc::renameat(
                from_fd,
                c_from.as_ptr(),
                self.fd.as_raw_fd(),
                c_target.as_ptr(),
            )
        } == 0
        {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    pub(super) fn create_exclusive_temp(&self, tmp_name: &OsStr) -> Result<File, CutError> {
        let c_name = validate_single_component(tmp_name)?;
        let flags =
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let raw = unsafe {
            libc::openat(
                self.fd.as_raw_fd(),
                c_name.as_ptr(),
                flags,
                // The mode is a C variadic argument: pass it as c_uint (macOS mode_t is u16, which
                // cannot be passed to a variadic function; Linux mode_t is already c_uint).
                0o600 as libc::c_uint,
            )
        };
        if raw < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(unsafe { File::from_raw_fd(raw) })
    }

    pub(super) fn unlink_child_best_effort(&self, name: &OsStr) {
        if let Ok(c_name) = validate_single_component(name) {
            unsafe {
                libc::unlinkat(self.fd.as_raw_fd(), c_name.as_ptr(), 0);
            }
        }
    }

    pub(super) fn fsync_best_effort(&self) {
        let _ = unsafe { libc::fsync(self.fd.as_raw_fd()) };
    }
}

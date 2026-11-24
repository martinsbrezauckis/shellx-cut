//! Private per-user export staging directory inside Cut's own data folder.
//!
//! Exports never create temporary files inside shared export directories:
//! every render/export opens or writes into `<cut_data_dir>/export-staging/<stage-...>`
//! (`0700` owned by `geteuid` on Unix; protected user-only DACL on Windows),
//! verifies the finished file, and then publishes it into the pinned export
//! directory.

use super::reserved::ReservedFile;
use cut_core::{error_codes, CutError};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

const STAGE_DIR_NAME: &str = "export-staging";
const STAGE_PREFIX: &str = "stage-";
const CRASH_STAGE_AGE: Duration = Duration::from_secs(12 * 3600);
static STAGE_FILE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Strip a Windows `\\?\` / `\\?\UNC\` extended prefix before handing a path
/// to FFmpeg, which misreads verbatim paths.
pub(crate) fn strip_windows_verbatim(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.display().to_string();
        if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{unc}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

/// Cut's own per-user data root (`SHELLX_CUT_HOME` when set, matching process
/// isolation, else the parent of `<app data>/ShellX Cut/tools`).
pub(crate) fn cut_data_dir() -> Result<PathBuf, CutError> {
    if let Some(home) = std::env::var_os("SHELLX_CUT_HOME").filter(|h| !h.is_empty()) {
        return Ok(PathBuf::from(home));
    }
    crate::toolpath::appdata_tools_dir()
        .and_then(|tools| tools.parent().map(Path::to_path_buf))
        .ok_or_else(|| {
            CutError::new(
                error_codes::IO,
                "no private export staging folder for this engine",
                "neither SHELLX_CUT_HOME nor the app-data folder is known",
            )
            .with_suggested_action("set SHELLX_CUT_HOME to a folder owned by your user")
        })
}

/// Return `<cut_data_dir>/export-staging`, creating and hardening it to
/// current-user-only access (`0700` on Unix; protected user-only DACL on Windows).
pub fn export_staging_root() -> Result<PathBuf, CutError> {
    let data = cut_data_dir()?;
    fs::create_dir_all(&data).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("could not create Cut data directory {}", data.display()),
            e.to_string(),
        )
    })?;
    let root = data.join(STAGE_DIR_NAME);
    match fs::symlink_metadata(&root) {
        Ok(meta) => {
            if !meta.file_type().is_dir() || cut_core::reparse::names_another_entry(&meta) {
                return Err(CutError::new(
                    error_codes::IO,
                    format!(
                        "export staging root is not a plain directory: {}",
                        root.display()
                    ),
                    "refusing symlinked or non-directory export staging root",
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => match fs::create_dir(&root) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(err) => {
                return Err(CutError::new(
                    error_codes::IO,
                    format!("could not create export staging folder {}", root.display()),
                    err.to_string(),
                ));
            }
        },
        Err(e) => return Err(e.into()),
    }
    harden_private_dir(&root)?;
    Ok(strip_windows_verbatim(&root))
}

#[cfg(unix)]
fn harden_private_dir(dir: &Path) -> Result<std::os::fd::OwnedFd, CutError> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(dir)?;
    let meta = file.metadata()?;
    if !meta.file_type().is_dir() || meta.file_type().is_symlink() {
        return Err(CutError::new(
            error_codes::IO,
            format!("staging folder is not a plain directory: {}", dir.display()),
            "expected an unlinked local directory",
        ));
    }
    let euid = unsafe { libc::geteuid() };
    if meta.uid() != euid {
        return Err(CutError::new(
            error_codes::IO,
            format!(
                "staging folder {} is not owned by the current user",
                dir.display()
            ),
            format!("uid {} != euid {}", meta.uid(), euid),
        ));
    }
    if meta.mode() & 0o777 != 0o700
        && unsafe { libc::fchmod(file.as_raw_fd(), 0o700 as libc::mode_t) } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let hardened = file.metadata()?;
    if hardened.uid() != euid || (hardened.mode() & 0o777) != 0o700 {
        return Err(CutError::new(
            error_codes::IO,
            format!("could not lock down staging folder {}", dir.display()),
            "staging directory must remain owner-only (0700)",
        ));
    }
    Ok(file.into())
}

#[cfg(windows)]
fn harden_private_dir(dir: &Path) -> Result<(), CutError> {
    super::windows_acl::harden_and_verify_private_dir(dir)
}

#[cfg(not(any(unix, windows)))]
fn harden_private_dir(_dir: &Path) -> Result<(), CutError> {
    Ok(())
}

/// An exclusively created per-export private staging folder (`stage-<pid>-XXXXXX`).
/// Removed on Drop in every success, failure, and cancellation path.
pub struct PrivateStageDir {
    path: PathBuf,
    #[cfg(unix)]
    dir_fd: Option<std::os::fd::OwnedFd>,
}

pub(super) struct ReservedStageFile {
    pub(super) path: PathBuf,
    pub(super) leaf_name: String,
    pub(super) identity: ReservedFile,
}

impl PrivateStageDir {
    /// Create a fresh, owner-only staging folder under `export_staging_root()`.
    pub fn new() -> Result<Self, CutError> {
        let root = export_staging_root()?;
        sweep_stale_export_staging_in(&root, CRASH_STAGE_AGE);
        let dir = tempfile::Builder::new()
            .prefix(&format!("{STAGE_PREFIX}{}-", std::process::id()))
            .tempdir_in(&root)?
            .keep();
        let dir = strip_windows_verbatim(&dir);
        #[cfg(unix)]
        let dir_fd = Some(harden_private_dir(&dir)?);
        #[cfg(not(unix))]
        harden_private_dir(&dir)?;
        Ok(Self {
            path: dir,
            #[cfg(unix)]
            dir_fd,
        })
    }

    /// Filesystem path of this private staging folder (non-verbatim on Windows).
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(unix)]
    pub(crate) fn raw_fd(&self) -> Option<std::os::fd::RawFd> {
        use std::os::fd::AsRawFd;
        self.dir_fd.as_ref().map(|fd| fd.as_raw_fd())
    }

    /// Exclusively create an empty staged file matching `target_out`'s extension
    /// so external writers like FFmpeg can overwrite it safely by path.
    pub(super) fn reserve_file_for(
        &self,
        target_out: &Path,
    ) -> Result<ReservedStageFile, CutError> {
        let raw_stem = target_out
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("export");
        let mut stem: String = raw_stem
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .take(40)
            .collect();
        if stem.is_empty() || stem.starts_with('-') {
            stem = "export".to_string();
        }
        let ext = target_out
            .extension()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or("tmp");
        let seq = STAGE_FILE_SEQ.fetch_add(1, Ordering::Relaxed);
        let leaf_name = format!("{stem}.{}.{seq}.{ext}", std::process::id());
        self.reserve_named_leaf(&leaf_name)
    }

    /// Exclusively reserve an empty staged file with a specific validated filename.
    pub fn reserve_named_file(&self, leaf_name: &str) -> Result<PathBuf, CutError> {
        Ok(self.reserve_named_leaf(leaf_name)?.path)
    }

    fn reserve_named_leaf(&self, leaf_name: &str) -> Result<ReservedStageFile, CutError> {
        if leaf_name.is_empty()
            || leaf_name == "."
            || leaf_name == ".."
            || leaf_name.contains(['/', '\\', '\0'])
        {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("invalid staging filename: {leaf_name}"),
                "staging leaf must be a single safe filename",
            ));
        }
        let staged = self.path.join(leaf_name);
        let mut opts = OpenOptions::new();
        opts.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let file = opts.open(&staged)?;
        drop(file);
        let identity = ReservedFile::of(&staged);
        Ok(ReservedStageFile {
            path: strip_windows_verbatim(&staged),
            leaf_name: leaf_name.to_string(),
            identity,
        })
    }
}

impl Drop for PrivateStageDir {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let _ = self.dir_fd.take();
        }
        let _ = fs::remove_dir_all(&self.path);
    }
}

enum StageEntryOwner {
    Pid(u32),
    KeepOnDoubt,
    NoPid,
}

fn stage_entry_owner(name: &str) -> StageEntryOwner {
    let Some(rest) = name.strip_prefix(STAGE_PREFIX) else {
        return StageEntryOwner::NoPid;
    };
    let Some((pid_part, _)) = rest.split_once('-') else {
        return StageEntryOwner::NoPid;
    };
    if pid_part.is_empty() || !pid_part.bytes().all(|b| b.is_ascii_digit()) {
        return StageEntryOwner::NoPid;
    }
    match pid_part.parse::<u32>() {
        Ok(pid) => StageEntryOwner::Pid(pid),
        Err(_) => StageEntryOwner::KeepOnDoubt,
    }
}

/// Remove stale staging directories older than `max_age` left behind by an
/// abrupt crash (`SIGKILL`/power loss) whose owning process is no longer running.
/// Never follows symbolic links.
pub fn sweep_stale_export_staging(max_age: Duration) {
    if let Ok(root) = export_staging_root() {
        sweep_stale_export_staging_in(&root, max_age);
    }
}

pub(crate) fn sweep_stale_export_staging_in(root: &Path, max_age: Duration) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if !name.starts_with(STAGE_PREFIX) {
            continue;
        }
        // A folder whose owning process is gone is a killed export's leftover:
        // it goes at once, whatever its age. Entries without a pid keep the age rule.
        let owner_gone = match stage_entry_owner(&name) {
            StageEntryOwner::Pid(pid)
                if pid == std::process::id() || super::process_is_running(pid) =>
            {
                continue;
            }
            StageEntryOwner::KeepOnDoubt => continue,
            StageEntryOwner::Pid(_) => true,
            StageEntryOwner::NoPid => false,
        };
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        let stale = owner_gone
            || max_age.is_zero()
            || meta
                .modified()
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .is_some_and(|d| d >= max_age);
        if !stale {
            continue;
        }
        if meta.file_type().is_dir() && !cut_core::reparse::names_another_entry(&meta) {
            let _ = fs::remove_dir_all(&path);
        } else {
            let _ = fs::remove_file(&path);
        }
    }
}

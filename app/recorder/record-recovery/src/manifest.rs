//! Append-only checkpoint manifest. A publication record follows an atomic rename,
//! so a journal entry always names an immutable, completely finalized segment.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::contract::{
    valid_receipt, CaptureManifest, CaptureStart, ManifestError, RecoveryReceipt, SCHEMA,
};
use crate::journal::Entry;

pub const MANIFEST_FILE: &str = "capture.manifest.jsonl";

pub(crate) fn valid_capture_id(capture_id: &str) -> bool {
    !capture_id.is_empty()
        && capture_id.len() <= 128
        && capture_id != "."
        && capture_id != ".."
        && !capture_id.contains(['/', '\\', ':'])
}

/// Fixed journal-only identifier for an open segment. It is not an active
/// filesystem path: native encoders use a private random stage and recovery
/// deliberately ignores this logical name.
pub(crate) fn staging_name(sequence: u64) -> String {
    format!(".checkpoint-{sequence:06}.open.mp4")
}

pub(crate) fn checkpoint_name(sequence: u64) -> String {
    format!("checkpoints/segment-{sequence:06}.mp4")
}

pub(crate) fn checkpoint_path(root: &Path, sequence: u64) -> PathBuf {
    root.join(checkpoint_name(sequence))
}

/// Return false for a missing file, a link/reparse point, or a non-regular file.
/// Callers derive the path from the trusted capture root and sequence; they must
/// not probe, hash, stitch, or move the file until this succeeds.
pub(crate) fn is_local_checkpoint_file(root: &Path, sequence: u64) -> Result<bool, ManifestError> {
    Ok(is_plain_dir(&root.join("checkpoints"))?
        && is_plain_regular_file(&checkpoint_path(root, sequence))?)
}

pub fn is_plain_regular_file(path: &Path) -> Result<bool, ManifestError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => return Err(io(path, source)),
    };
    Ok(metadata.file_type().is_file()
        && !metadata.file_type().is_symlink()
        && !is_reparse(&metadata))
}

pub fn is_plain_dir(path: &Path) -> Result<bool, ManifestError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => return Err(io(path, source)),
    };
    Ok(metadata.file_type().is_dir()
        && !metadata.file_type().is_symlink()
        && !is_reparse(&metadata))
}

#[cfg(windows)]
fn is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

#[cfg(not(windows))]
fn is_reparse(_metadata: &fs::Metadata) -> bool {
    false
}
/// Exclusive writer for one capture. The server creates it before a backend starts;
/// a backend re-opens it to append only its own completed checkpoint records.
pub struct ManifestOwner {
    pub(crate) root: PathBuf,
    pub(crate) file: File,
    pub(crate) manifest: CaptureManifest,
    pub(crate) staging: Option<crate::staging::PrivateStaging>,
}

impl ManifestOwner {
    pub fn begin(root: &Path, start: CaptureStart) -> Result<Self, ManifestError> {
        if start.schema != SCHEMA
            || !valid_capture_id(&start.capture_id)
            || start.checkpoint_interval_ms == 0
        {
            return Err(ManifestError::Invalid(
                "capture id and interval are required".into(),
            ));
        }
        create_plain_dir(root)?;
        create_plain_dir(&root.join("checkpoints"))?;
        let path = root.join(MANIFEST_FILE);
        if fs::symlink_metadata(&path).is_ok() {
            return Err(ManifestError::Invalid(format!(
                "refusing to replace existing {}",
                path.display()
            )));
        }
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)
            .map_err(|source| io(&path, source))?;
        let mut owner = Self {
            root: root.to_path_buf(),
            file,
            manifest: CaptureManifest {
                start: start.clone(),
                checkpoints: Vec::new(),
                receipt: None,
                openings: Vec::new(),
                torn_tail: false,
                valid_prefix: Vec::new(),
                torn_tail_bytes: None,
            },
            staging: None,
        };
        owner.append(&Entry::Start(start))?;
        Ok(owner)
    }

    pub fn open(root: &Path) -> Result<Self, ManifestError> {
        let manifest = read_manifest(root)?;
        if manifest.torn_tail {
            return Err(ManifestError::Corrupt(
                "manifest has a torn final entry; recovery must quarantine it first".into(),
            ));
        }
        let path = root.join(MANIFEST_FILE);
        if !is_plain_dir(root)? || !is_plain_regular_file(&path)? {
            return Err(ManifestError::Invalid(
                "manifest root or file is not a local regular path".into(),
            ));
        }
        let file = open_append_nofollow(&path)?;
        Ok(Self {
            root: root.to_path_buf(),
            file,
            manifest,
            staging: None,
        })
    }

    pub fn manifest(&self) -> &CaptureManifest {
        &self.manifest
    }

    pub fn publish_receipt(&mut self, receipt: RecoveryReceipt) -> Result<(), ManifestError> {
        if self.manifest.receipt.is_some() {
            return Ok(());
        }
        if !valid_receipt(
            &receipt,
            &self.manifest.checkpoints,
            !self.manifest.openings.is_empty(),
        ) {
            return Err(ManifestError::Invalid(
                "recovery receipt is inconsistent with the capture journal".into(),
            ));
        }
        self.append(&Entry::Receipt(receipt.clone()))?;
        self.manifest.receipt = Some(receipt);
        Ok(())
    }

    pub(crate) fn append(&mut self, entry: &Entry) -> Result<(), ManifestError> {
        let bytes = serde_json::to_vec(entry)?;
        self.file
            .write_all(&bytes)
            .map_err(|source| io(&self.root, source))?;
        self.file
            .write_all(b"\n")
            .map_err(|source| io(&self.root, source))?;
        self.file
            .sync_all()
            .map_err(|source| io(&self.root, source))?;
        sync_parent(&self.root.join(MANIFEST_FILE))
    }
}

fn create_plain_dir(path: &Path) -> Result<(), ManifestError> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            return is_plain_dir(path)?
                .then_some(())
                .ok_or_else(|| ManifestError::Invalid("capture directory is not local".into()));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(io(path, source)),
    }
    let parent = path
        .parent()
        .ok_or_else(|| ManifestError::Invalid("capture directory has no safe parent".into()))?;
    if parent != path {
        create_plain_dir(parent)?;
    }
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(source) => return Err(io(path, source)),
    };
    is_plain_dir(path)?
        .then_some(())
        .ok_or_else(|| ManifestError::Invalid("capture directory is not local".into()))
}

fn open_append_nofollow(path: &Path) -> Result<File, ManifestError> {
    let mut options = OpenOptions::new();
    options.append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path).map_err(|source| io(path, source))
}
pub fn read_manifest(root: &Path) -> Result<CaptureManifest, ManifestError> {
    crate::journal::read(root)
}

pub(crate) fn sha256(path: &Path) -> Result<String, ManifestError> {
    use std::io::Read;
    let mut file = File::open(path).map_err(|source| io(path, source))?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| io(path, source))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<(), ManifestError> {
    File::open(path.parent().unwrap_or(Path::new(".")))
        .and_then(|f| f.sync_all())
        .map_err(|source| io(path, source))
}
#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> Result<(), ManifestError> {
    Ok(())
}
pub(crate) fn io(path: &Path, source: std::io::Error) -> ManifestError {
    ManifestError::Io {
        path: path.to_path_buf(),
        source,
    }
}

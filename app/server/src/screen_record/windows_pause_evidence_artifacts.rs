//! Owned-manifest checkpoint identity checks for private pause evidence.

use super::windows_pause_adapter::WindowsPauseAdapterError;
use record_capture::windows_pause_pilot::WindowsSealedAudioRun;
use record_recovery::{is_plain_dir, is_plain_regular_file, Checkpoint, RecordingStream};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// Revalidates the concrete immutable checkpoint artifact before the private
/// journal may name it. Production proves its owned manifest and one file
/// handle; deterministic tests may inject a bounded verifier.
pub(crate) trait WindowsPauseArtifactVerifier {
    fn verify(&self, checkpoint: &Checkpoint) -> Result<(), WindowsPauseAdapterError>;

    fn verify_audio(&self, _audio: &WindowsSealedAudioRun) -> Result<(), WindowsPauseAdapterError> {
        Err(WindowsPauseAdapterError::EvidenceRejected)
    }
}

pub(crate) struct LocalWindowsPauseArtifactVerifier {
    capture_dir: PathBuf,
}

impl LocalWindowsPauseArtifactVerifier {
    pub(crate) fn new(capture_dir: PathBuf) -> Self {
        Self { capture_dir }
    }

    fn verify_after_hash<F>(
        &self,
        checkpoint: &Checkpoint,
        after_hash: F,
    ) -> Result<(), WindowsPauseAdapterError>
    where
        F: FnOnce(&Path),
    {
        manifest_matches(&self.capture_dir, checkpoint)?;
        if checkpoint.file != format!("checkpoints/segment-{:06}.mp4", checkpoint.sequence)
            || checkpoint.bytes == 0
        {
            return Err(WindowsPauseAdapterError::EvidenceRejected);
        }
        let path = contained_file(&self.capture_dir, &checkpoint.file)?;
        let mut first = open_nofollow(&path)?;
        let metadata = first
            .metadata()
            .map_err(|_| WindowsPauseAdapterError::EvidenceRejected)?;
        if !metadata.file_type().is_file()
            || is_reparse(&metadata)
            || metadata.len() != checkpoint.bytes
        {
            return Err(WindowsPauseAdapterError::EvidenceRejected);
        }
        if hash_open(&mut first)? != checkpoint.sha256 {
            return Err(WindowsPauseAdapterError::EvidenceRejected);
        }
        let after_hash_metadata = first
            .metadata()
            .map_err(|_| WindowsPauseAdapterError::EvidenceRejected)?;
        if !same_metadata(&metadata, &after_hash_metadata) {
            return Err(WindowsPauseAdapterError::EvidenceRejected);
        }
        after_hash(&path);
        let current = open_nofollow(&path)?;
        let current_metadata = current
            .metadata()
            .map_err(|_| WindowsPauseAdapterError::EvidenceRejected)?;
        if !same_open_file(&first, &current)? || !same_metadata(&metadata, &current_metadata) {
            return Err(WindowsPauseAdapterError::EvidenceRejected);
        }
        manifest_matches(&self.capture_dir, checkpoint)
    }
}

impl WindowsPauseArtifactVerifier for LocalWindowsPauseArtifactVerifier {
    fn verify(&self, checkpoint: &Checkpoint) -> Result<(), WindowsPauseAdapterError> {
        self.verify_after_hash(checkpoint, |_| {})
    }

    fn verify_audio(&self, audio: &WindowsSealedAudioRun) -> Result<(), WindowsPauseAdapterError> {
        let expected = match audio.stream {
            RecordingStream::MicrophoneAudio => {
                format!(
                    "recording-microphone-generation-{:020}.wav",
                    audio.source_generation
                )
            }
            RecordingStream::SystemAudio => {
                format!(
                    "recording-system-generation-{:020}.wav",
                    audio.source_generation
                )
            }
            _ => return Err(WindowsPauseAdapterError::EvidenceRejected),
        };
        if audio.source_generation == 0
            || audio.artifact != expected
            || audio.bytes == 0
            || audio.sha256.len() != 64
            || !audio.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            || audio.media_duration_ms == 0
            || audio.native_ready_unix_ms == 0
            || audio.raw_end_ms <= audio.raw_start_ms
            || audio.native_ready_raw_ms < audio.raw_start_ms
            || audio.native_ready_raw_ms > audio.raw_end_ms
        {
            return Err(WindowsPauseAdapterError::EvidenceRejected);
        }
        let path = contained_file(&self.capture_dir, &audio.artifact)?;
        let mut first = open_nofollow(&path)?;
        let metadata = first
            .metadata()
            .map_err(|_| WindowsPauseAdapterError::EvidenceRejected)?;
        if !metadata.file_type().is_file()
            || is_reparse(&metadata)
            || metadata.len() != audio.bytes
            || hash_open(&mut first)? != audio.sha256
        {
            return Err(WindowsPauseAdapterError::EvidenceRejected);
        }
        let after_hash = first
            .metadata()
            .map_err(|_| WindowsPauseAdapterError::EvidenceRejected)?;
        if !same_metadata(&metadata, &after_hash) {
            return Err(WindowsPauseAdapterError::EvidenceRejected);
        }
        let current = open_nofollow(&path)?;
        let current_metadata = current
            .metadata()
            .map_err(|_| WindowsPauseAdapterError::EvidenceRejected)?;
        (same_open_file(&first, &current)? && same_metadata(&metadata, &current_metadata))
            .then_some(())
            .ok_or(WindowsPauseAdapterError::EvidenceRejected)
    }
}

fn same_metadata(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    left.len() == right.len()
        && matches!((left.modified(), right.modified()), (Ok(left), Ok(right)) if left == right)
}

fn manifest_matches(dir: &Path, checkpoint: &Checkpoint) -> Result<(), WindowsPauseAdapterError> {
    let manifest = record_recovery::read_manifest(dir)
        .map_err(|_| WindowsPauseAdapterError::EvidenceRejected)?;
    (manifest
        .checkpoints
        .iter()
        .find(|published| published.sequence == checkpoint.sequence)
        == Some(checkpoint))
    .then_some(())
    .ok_or(WindowsPauseAdapterError::EvidenceRejected)
}

fn contained_file(capture_dir: &Path, artifact: &str) -> Result<PathBuf, WindowsPauseAdapterError> {
    if !is_plain_dir(capture_dir).map_err(|_| WindowsPauseAdapterError::EvidenceRejected)? {
        return Err(WindowsPauseAdapterError::EvidenceRejected);
    }
    let components = Path::new(artifact).components().collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(WindowsPauseAdapterError::EvidenceRejected);
    }
    let mut path = capture_dir.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(component) = component else {
            unreachable!()
        };
        path.push(component);
        let local = if index + 1 == components.len() {
            is_plain_regular_file(&path)
        } else {
            is_plain_dir(&path)
        }
        .map_err(|_| WindowsPauseAdapterError::EvidenceRejected)?;
        if !local {
            return Err(WindowsPauseAdapterError::EvidenceRejected);
        }
    }
    Ok(path)
}

fn open_nofollow(path: &Path) -> Result<File, WindowsPauseAdapterError> {
    if !is_plain_regular_file(path).map_err(|_| WindowsPauseAdapterError::EvidenceRejected)? {
        return Err(WindowsPauseAdapterError::EvidenceRejected);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options
        .open(path)
        .map_err(|_| WindowsPauseAdapterError::EvidenceRejected)
}

fn hash_open(file: &mut File) -> Result<String, WindowsPauseAdapterError> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| WindowsPauseAdapterError::EvidenceRejected)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(unix)]
fn same_open_file(left: &File, right: &File) -> Result<bool, WindowsPauseAdapterError> {
    use std::os::unix::fs::MetadataExt;
    let left = left
        .metadata()
        .map_err(|_| WindowsPauseAdapterError::EvidenceRejected)?;
    let right = right
        .metadata()
        .map_err(|_| WindowsPauseAdapterError::EvidenceRejected)?;
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
}

#[cfg(windows)]
fn same_open_file(left: &File, right: &File) -> Result<bool, WindowsPauseAdapterError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
    };
    fn identity(file: &File) -> Result<(u32, u32, u32), WindowsPauseAdapterError> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: the handle is live and `info` is writable for this call.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) } == 0 {
            return Err(WindowsPauseAdapterError::EvidenceRejected);
        }
        Ok((
            info.dwVolumeSerialNumber,
            info.nFileIndexHigh,
            info.nFileIndexLow,
        ))
    }
    Ok(identity(left)? == identity(right)?)
}

#[cfg(not(any(unix, windows)))]
fn same_open_file(_left: &File, _right: &File) -> Result<bool, WindowsPauseAdapterError> {
    Err(WindowsPauseAdapterError::EvidenceRejected)
}

#[cfg(windows)]
fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

#[cfg(not(windows))]
fn is_reparse(_metadata: &std::fs::Metadata) -> bool {
    false
}

#[cfg(all(test, unix))]
#[path = "windows_pause_evidence_artifacts_tests.rs"]
mod tests;

//! Exact no-follow verification for the private per-run WAV leaves.
//!
//! A sidecar is only useful if recovery proves the leaf it names on every
//! reopen. The descriptor, not a pre-open path inspection, supplies the bytes,
//! hash, and WAV duration used below.

use super::InputAudioSource;
use record_recovery::{is_plain_dir, is_plain_regular_file, SealedRun};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

pub(super) fn verify_audio_sources(
    capture_dir: &Path,
    sources: &[InputAudioSource],
    run: &SealedRun,
) -> Result<(), String> {
    if !is_plain_dir(capture_dir)
        .map_err(|_| "inspect recording-input WAV capture directory".to_string())?
    {
        return Err("recording-input WAV capture directory is missing or linked".into());
    }
    for source in sources {
        if source.expected_artifact().as_deref() != Some(source.artifact.as_str()) {
            return Err(
                "recording-input audio artifact does not bind its source role and generation"
                    .into(),
            );
        }
        let path = contained_leaf(capture_dir, &source.artifact)?;
        let mut file = open_local(&path)?;
        let before = file
            .metadata()
            .map_err(|_| "read recording-input WAV metadata".to_string())?;
        if before.len() != source.bytes || before.len() == 0 {
            return Err("recording-input WAV byte count differs from sealed sidecar".into());
        }
        let (sha256, duration_ms) = wav_facts(&mut file)?;
        let after = file
            .metadata()
            .map_err(|_| "re-read recording-input WAV metadata".to_string())?;
        if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
            return Err("recording-input WAV changed while its sealed descriptor was read".into());
        }
        let reopened = open_local(&path)?;
        if !same_open_file(&file, &reopened)? {
            return Err("recording-input WAV was replaced while recovery verified it".into());
        }
        if sha256 != source.sha256 || duration_ms != source.media_duration_ms {
            return Err(
                "recording-input WAV hash or media facts differ from sealed sidecar".into(),
            );
        }
        if !source.matches_run(run)
            || source.logical_start_ms != run.logical_start_ms
            || source.logical_end_ms != run.logical_end_ms
            || source.raw_end_ms <= source.raw_start_ms
            || source.raw_end_ms - source.raw_start_ms
                != source.logical_end_ms - source.logical_start_ms
            || source.native_ready_raw_ms < source.raw_start_ms
            || source.native_ready_raw_ms > source.raw_end_ms
        {
            return Err(
                "recording-input WAV logical or raw span drifted from the sealed run".into(),
            );
        }
    }
    Ok(())
}

fn contained_leaf(capture_dir: &Path, artifact: &str) -> Result<PathBuf, String> {
    let components = Path::new(artifact).components().collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("recording-input WAV artifact is not a literal relative path".into());
    }
    let mut path = capture_dir.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(component) = component else {
            unreachable!();
        };
        path.push(component);
        let plain = if index + 1 == components.len() {
            is_plain_regular_file(&path)
        } else {
            is_plain_dir(&path)
        }
        .map_err(|_| "inspect recording-input WAV containment".to_string())?;
        if !plain {
            return Err(
                "recording-input WAV is missing, linked, or outside its local capture root".into(),
            );
        }
    }
    Ok(path)
}

fn wav_facts(file: &mut File) -> Result<(String, u64), String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "rewind recording-input WAV before parsing".to_string())?;
    let reader = hound::WavReader::new(
        file.try_clone()
            .map_err(|_| "clone recording-input WAV descriptor".to_string())?,
    )
    .map_err(|_| "recording-input WAV is malformed".to_string())?;
    let sample_rate = u64::from(reader.spec().sample_rate);
    let duration_ms = u64::from(reader.duration())
        .checked_mul(1_000)
        .and_then(|frames| frames.checked_div(sample_rate))
        .filter(|duration| *duration > 0)
        .ok_or_else(|| "recording-input WAV has no usable media duration".to_string())?;
    // `try_clone` shares the descriptor offset. Rewind after hound consumed
    // header/chunk bytes so the digest covers the full immutable WAV, not its
    // suffix.
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "rewind recording-input WAV before hashing".to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "hash recording-input WAV".to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok((format!("{:x}", digest.finalize()), duration_ms))
}

fn open_local(path: &Path) -> Result<File, String> {
    if !is_plain_regular_file(path)
        .map_err(|_| "inspect recording-input WAV before no-follow open".to_string())?
    {
        return Err("recording-input WAV is not a local regular file".into());
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
    let file = options
        .open(path)
        .map_err(|_| "open recording-input WAV without following links".to_string())?;
    let metadata = file
        .metadata()
        .map_err(|_| "read opened recording-input WAV metadata".to_string())?;
    (metadata.file_type().is_file() && !is_reparse(&metadata))
        .then_some(file)
        .ok_or_else(|| "opened recording-input WAV is not a local regular file".into())
}

fn same_open_file(left: &File, right: &File) -> Result<bool, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let left = left
            .metadata()
            .map_err(|_| "read first recording-input WAV identity".to_string())?;
        let right = right
            .metadata()
            .map_err(|_| "read second recording-input WAV identity".to_string())?;
        Ok(left.dev() == right.dev() && left.ino() == right.ino())
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        fn identity(file: &File) -> Result<(u32, u32, u32), String> {
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            // SAFETY: both descriptors were opened locally with no-follow semantics.
            if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
                return Err("read recording-input WAV handle identity".into());
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
    {
        let _ = (left, right);
        Err("this platform cannot prove recording-input WAV handle identity".into())
    }
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

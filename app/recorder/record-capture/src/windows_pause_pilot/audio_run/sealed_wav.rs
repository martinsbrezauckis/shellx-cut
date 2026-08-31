//! Fixed-name no-follow WAV sealing and full-file proof for private audio runs.

use super::super::{WindowsPausePilotStarted, WindowsSealedAudioRun};
use record_recovery::RecordingStream;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const MICROPHONE_PREFIX: &str = "recording-microphone-generation-";
const SYSTEM_PREFIX: &str = "recording-system-generation-";

pub(super) fn artifact_name(stream: RecordingStream, generation: u64) -> Result<String, ()> {
    let prefix = match stream {
        RecordingStream::MicrophoneAudio => MICROPHONE_PREFIX,
        RecordingStream::SystemAudio => SYSTEM_PREFIX,
        _ => return Err(()),
    };
    Ok(format!("{prefix}{generation:020}.wav"))
}

pub(super) fn sealed_file(
    stream: RecordingStream,
    artifact: String,
    path: PathBuf,
    started: WindowsPausePilotStarted,
    native_ready_offset_ms: u64,
    raw_end_ms: u64,
) -> Result<WindowsSealedAudioRun, ()> {
    if raw_end_ms <= started.observed_start_ms
        || artifact_name(stream, started.physical_generation)? != artifact
    {
        return Err(());
    }
    let mut file = open_local(&path)?;
    let before = file.metadata().map_err(|_| ())?;
    if !before.file_type().is_file() || is_reparse(&before) || before.len() == 0 {
        return Err(());
    }
    let (sha256, media_duration_ms) = wav_facts(&mut file)?;
    let after = file.metadata().map_err(|_| ())?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(());
    }
    let second = open_local(&path)?;
    if !same_open_file(&file, &second)? {
        return Err(());
    }
    let native_ready_raw_ms = started
        .observed_start_ms
        .checked_add(native_ready_offset_ms)
        .filter(|ready| *ready <= raw_end_ms)
        .ok_or(())?;
    let native_ready_unix_ms = started
        .unix_ms
        .checked_add(native_ready_offset_ms)
        .ok_or(())?;
    Ok(WindowsSealedAudioRun {
        stream,
        source_generation: started.physical_generation,
        artifact,
        bytes: before.len(),
        sha256,
        media_duration_ms,
        native_ready_unix_ms,
        native_ready_raw_ms,
        raw_start_ms: started.observed_start_ms,
        raw_end_ms,
    })
}

fn wav_facts(file: &mut File) -> Result<(String, u64), ()> {
    file.seek(SeekFrom::Start(0)).map_err(|_| ())?;
    let reader = hound::WavReader::new(file.try_clone().map_err(|_| ())?).map_err(|_| ())?;
    let spec = reader.spec();
    let frames = u64::from(reader.duration());
    let media_duration_ms = frames
        .checked_mul(1_000)
        .and_then(|value| value.checked_div(u64::from(spec.sample_rate)))
        .filter(|value| *value > 0)
        .ok_or(())?;
    // `try_clone` shares the descriptor offset, so this rewind is essential:
    // sidecar hashes must cover the complete WAV, including RIFF/header bytes.
    file.seek(SeekFrom::Start(0)).map_err(|_| ())?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|_| ())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok((format!("{:x}", digest.finalize()), media_duration_ms))
}

fn open_local(path: &Path) -> Result<File, ()> {
    if !record_recovery::is_plain_regular_file(path).map_err(|_| ())? {
        return Err(());
    }
    use std::os::windows::fs::OpenOptionsExt;
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT.0);
    let file = options.open(path).map_err(|_| ())?;
    let metadata = file.metadata().map_err(|_| ())?;
    (metadata.file_type().is_file() && !is_reparse(&metadata))
        .then_some(file)
        .ok_or(())
}

fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT.0
        != 0
}

fn same_open_file(left: &File, right: &File) -> Result<bool, ()> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    fn identity(file: &File) -> Result<(u32, u32, u32), ()> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: both handles are live, locally opened no-follow files.
        unsafe {
            GetFileInformationByHandle(HANDLE(file.as_raw_handle() as *mut _), &mut info)
                .map_err(|_| ())?;
        }
        Ok((
            info.dwVolumeSerialNumber,
            info.nFileIndexHigh,
            info.nFileIndexLow,
        ))
    }
    Ok(identity(left)? == identity(right)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_hash_rewinds_after_header_parse_and_covers_the_complete_file() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 1_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(temp.path(), spec).unwrap();
        for sample in [1_i16, 2, 3, 4, 5, 6, 7, 8, 9, 10] {
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
        let expected = format!("{:x}", Sha256::digest(std::fs::read(temp.path()).unwrap()));
        let mut file = OpenOptions::new().read(true).open(temp.path()).unwrap();
        let (actual, _) = wav_facts(&mut file).unwrap();
        assert_eq!(actual, expected);
    }
}

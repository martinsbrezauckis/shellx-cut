//! Immutable generation WAV publication and physical file evidence.

use super::super::{audio, MacosPauseAudioLayout, MacosPausePilotStarted, MacosSealedAudioRun};
use record_recovery::RecordingStream;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub(super) fn artifact_name(stream: RecordingStream, generation: u64) -> Result<String, ()> {
    let prefix = match stream {
        RecordingStream::MicrophoneAudio => "recording-microphone-generation-",
        RecordingStream::SystemAudio => "recording-system-generation-",
        _ => return Err(()),
    };
    (generation != 0)
        .then(|| format!("{prefix}{generation:020}.wav"))
        .ok_or(())
}

pub(super) fn sealed_file(
    stream: RecordingStream,
    artifact: String,
    path: &Path,
    started: &MacosPausePilotStarted,
    native_ready_offset_ms: u64,
    raw_end_ms: u64,
) -> Result<MacosSealedAudioRun, ()> {
    if raw_end_ms <= started.observed_start_ms
        || artifact_name(stream, started.physical_generation)? != artifact
    {
        return Err(());
    }
    let mut file = open_local(path)?;
    let before = file.metadata().map_err(|_| ())?;
    if !before.file_type().is_file() || before.len() == 0 {
        return Err(());
    }
    let (sha256, media_duration_ms) = wav_facts(&mut file)?;
    let after = file.metadata().map_err(|_| ())?;
    if before.len() != after.len() || !same_identity(&before, &after) {
        return Err(());
    }
    let second = open_local(path)?;
    if !same_identity(&before, &second.metadata().map_err(|_| ())?) {
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
    Ok(MacosSealedAudioRun {
        stream,
        layout: MacosPauseAudioLayout::PacketStart,
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

pub(super) fn wav_facts(file: &mut File) -> Result<(String, u64), ()> {
    file.seek(SeekFrom::Start(0)).map_err(|_| ())?;
    let reader = hound::WavReader::new(file.try_clone().map_err(|_| ())?).map_err(|_| ())?;
    let sample_rate = u64::from(reader.spec().sample_rate);
    let media_duration_ms = u64::from(reader.duration())
        .checked_mul(1_000)
        .and_then(|samples| samples.checked_div(sample_rate))
        .filter(|duration| *duration > 0)
        .ok_or(())?;
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

pub(super) fn open_local(path: &Path) -> Result<File, ()> {
    if !record_recovery::is_plain_regular_file(path).map_err(|_| ())? {
        return Err(());
    }
    use std::os::unix::fs::OpenOptionsExt;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| ())?;
    file.metadata()
        .map_err(|_| ())?
        .file_type()
        .is_file()
        .then_some(file)
        .ok_or(())
}

fn same_identity(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    left.dev() == right.dev() && left.ino() == right.ino()
}

/// The native writer retains its complete raw leaf; only the real samples
/// inside the screen interval become the immutable generation sidecar.
pub(super) fn publish_bounded_microphone_wav(
    raw_path: &Path,
    path: &Path,
    raw_start_ms: u64,
    first_packet_offset_ms: u64,
    raw_end_ms: u64,
) -> Result<(), ()> {
    let file = open_local(raw_path)?;
    let before = file.metadata().map_err(|_| ())?;
    let mut reader = hound::WavReader::new(file).map_err(|_| ())?;
    let spec = reader.spec();
    if spec.sample_format != hound::SampleFormat::Int || spec.bits_per_sample != 16 {
        return Err(());
    }
    let available = usize::try_from(reader.len()).map_err(|_| ())?;
    let count = audio::bounded_audio_samples(
        raw_start_ms,
        raw_end_ms,
        first_packet_offset_ms,
        spec.sample_rate,
        spec.channels,
        available,
    )?;
    let parent = path
        .parent()
        .filter(|parent| Some(*parent) == raw_path.parent())
        .ok_or(())?;
    let (part, file) =
        record_recovery::create_staging_file(parent, "pause-mic-boundary").map_err(|_| ())?;
    let written = (|| -> Result<(), ()> {
        let mut writer = hound::WavWriter::new(file, spec).map_err(|_| ())?;
        for sample in reader.samples::<i16>().take(count) {
            writer
                .write_sample(sample.map_err(|_| ())?)
                .map_err(|_| ())?;
        }
        writer.finalize().map_err(|_| ())?;
        let current = open_local(raw_path)?.metadata().map_err(|_| ())?;
        if before.len() != current.len() || !same_identity(&before, &current) {
            return Err(());
        }
        record_recovery::publish_new_synced(&part, path).map_err(|_| ())
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written
}

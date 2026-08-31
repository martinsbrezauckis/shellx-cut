//! Private compact audio staging for a completed pause-session projection.
//!
//! The normal recording contract keeps microphone and process-loopback WAVs as
//! separate siblings of `source.mp4`. This module preserves that contract: it
//! joins only exact sidecar-verified per-run leaves, writes `mic.wav` and/or
//! `system.wav` without replacement, and keeps system first-packet timing.

use crate::screen_record::windows_pause_input_sidecar::VerifiedInputAudio;
use cut_core::{error_codes, CutError};
use record_recovery::{
    is_plain_regular_file, publish_new_synced, CaptureRoot, PrivateStaging,
    RecordingSessionJournal, RecordingStream,
};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

mod contract;
pub(super) use contract::ProjectionAudioContract;
use contract::ProjectionAudioLeaf;

const MICROPHONE_FILE: &str = "mic.wav";
const SYSTEM_FILE: &str = "system.wav";

pub(super) struct PreparedPauseProjectionAudio {
    tracks: Vec<PreparedTrack>,
    contract: ProjectionAudioContract,
}

struct PreparedTrack {
    leaf: ProjectionAudioLeaf,
    stage: PrivateStaging,
    first_packet_offset_ms: Option<u64>,
}

impl PreparedPauseProjectionAudio {
    pub(super) fn prepare(
        capture_dir: &Path,
        root: &CaptureRoot,
        capture_id: &str,
        journal: &RecordingSessionJournal,
    ) -> Result<Self, CutError> {
        let sources = crate::screen_record::windows_pause_input_sidecar::verified_audio_sources(
            root, capture_id, journal,
        )
        .map_err(invalid)?;
        let declared_audio = journal.intent().requested_streams.iter().any(|stream| {
            matches!(
                stream,
                RecordingStream::MicrophoneAudio | RecordingStream::SystemAudio
            )
        });
        if declared_audio && sources.is_empty() {
            return Err(invalid(
                "selected private audio has no exact sidecar-verified WAV sources",
            ));
        }
        let mut tracks = Vec::new();
        for (stream, file_name) in [
            (RecordingStream::MicrophoneAudio, MICROPHONE_FILE),
            (RecordingStream::SystemAudio, SYSTEM_FILE),
        ] {
            let sources = sources
                .iter()
                .filter(|source| source.stream() == stream)
                .cloned()
                .collect::<Vec<_>>();
            if sources.is_empty() {
                continue;
            }
            let stage = PrivateStaging::create(capture_dir, "pause-projection-audio", file_name)
                .map_err(|error| invalid(format!("reserve private {file_name} stage: {error}")))?;
            stage_track(capture_dir, stage.path(), stream, &sources)?;
            let leaf = ProjectionAudioLeaf::from_local(file_name, stage.path())?;
            let first_packet_offset_ms = (stream == RecordingStream::SystemAudio)
                .then(|| {
                    sources[0]
                        .logical_start_ms()
                        .checked_add(sources[0].native_ready_offset_ms())
                        .ok_or_else(|| invalid("private system-audio packet offset overflowed"))
                })
                .transpose()?;
            tracks.push(PreparedTrack {
                leaf,
                stage,
                first_packet_offset_ms,
            });
        }
        let microphone = tracks
            .iter()
            .find(|track| track.leaf.file_name() == MICROPHONE_FILE)
            .map(|track| track.leaf.clone());
        let system = tracks
            .iter()
            .find(|track| track.leaf.file_name() == SYSTEM_FILE)
            .map(|track| (track.leaf.clone(), track.first_packet_offset_ms));
        let contract = ProjectionAudioContract::new(microphone, system)?;
        Ok(Self { tracks, contract })
    }

    pub(super) fn microphone_file(&self) -> Option<&'static str> {
        self.contract.microphone_file()
    }

    pub(super) fn contract(&self) -> &ProjectionAudioContract {
        &self.contract
    }

    pub(super) fn verify_published(&self, capture_dir: &Path) -> Result<(), CutError> {
        self.contract.verify_published(capture_dir)
    }

    pub(super) fn publish(&self, capture_dir: &Path) -> Result<(), CutError> {
        for track in &self.tracks {
            let output = capture_dir.join(track.leaf.file_name());
            match std::fs::symlink_metadata(&output) {
                Ok(_) => {
                    self.contract.verify_leaf(&output, &track.leaf)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    publish_new_synced(track.stage.path(), &output).map_err(audio_error)?;
                }
                Err(error) => return Err(audio_error(error)),
            }
        }
        self.contract.publish_system_timing(capture_dir)?;
        self.verify_published(capture_dir)
    }
}

fn stage_track(
    capture_dir: &Path,
    output: &Path,
    stream: RecordingStream,
    sources: &[VerifiedInputAudio],
) -> Result<(), CutError> {
    let mut sources = sources.to_vec();
    sources.sort_by_key(VerifiedInputAudio::logical_start_ms);
    let system_first_packet_at = (stream == RecordingStream::SystemAudio)
        .then(|| {
            sources[0]
                .logical_start_ms()
                .checked_add(sources[0].native_ready_offset_ms())
                .ok_or_else(|| invalid("private system-audio packet offset overflowed"))
        })
        .transpose()?;
    let mut writer: Option<hound::WavWriter<std::io::BufWriter<File>>> = None;
    let mut output_spec: Option<hound::WavSpec> = None;
    let mut written_samples = 0_u64;
    for source in &sources {
        let mut reader = hound::WavReader::new(open_local(&capture_dir.join(source.artifact()))?)
            .map_err(|_| {
            invalid("selected private WAV is malformed while staging projection")
        })?;
        let spec = reader.spec();
        if spec.sample_format != hound::SampleFormat::Int
            || spec.bits_per_sample != 16
            || spec.sample_rate == 0
            || spec.channels == 0
        {
            return Err(invalid(
                "selected private WAV has a format the compact audio projection cannot preserve",
            ));
        }
        if reader.duration() == 0
            || duration_ms(&reader)? != source.media_duration_ms()
            || output_spec.is_some_and(|existing| existing != spec)
        {
            return Err(invalid(
                "selected private WAV facts or format differ from the sealed audio contract",
            ));
        }
        if writer.is_none() {
            writer = Some(
                hound::WavWriter::create(output, spec)
                    .map_err(|_| invalid("create private compact audio projection"))?,
            );
            output_spec = Some(spec);
        }
        let start_ms = if let Some(first_packet_at) = system_first_packet_at {
            source
                .logical_start_ms()
                .checked_add(source.native_ready_offset_ms())
                .and_then(|at| at.checked_sub(first_packet_at))
                .ok_or_else(|| invalid("private system-audio logical placement underflowed"))?
        } else {
            source.logical_start_ms()
        };
        let desired_samples = start_ms
            .checked_mul(u64::from(spec.sample_rate))
            .and_then(|frames| frames.checked_div(1_000))
            .and_then(|frames| frames.checked_mul(u64::from(spec.channels)))
            .ok_or_else(|| invalid("private audio logical sample offset overflowed"))?;
        if written_samples > desired_samples {
            return Err(invalid(
                "selected private WAVs overlap after their sealed logical placement",
            ));
        }
        let writer = writer.as_mut().expect("writer initialized above");
        for _ in written_samples..desired_samples {
            writer
                .write_sample(0_i16)
                .map_err(|_| invalid("pad private compact audio projection"))?;
        }
        written_samples = desired_samples;
        for sample in reader.samples::<i16>() {
            writer
                .write_sample(sample.map_err(|_| invalid("decode selected private WAV"))?)
                .map_err(|_| invalid("write private compact audio projection"))?;
            written_samples = written_samples
                .checked_add(1)
                .ok_or_else(|| invalid("private audio sample count overflowed"))?;
        }
        let mut file = reader.into_inner();
        verify_reopened_source(&mut file, source)?;
    }
    writer
        .ok_or_else(|| invalid("private audio staging had no selected WAV sources"))?
        .finalize()
        .map_err(|_| invalid("finalize private compact audio projection"))?;
    Ok(())
}

fn verify_reopened_source(file: &mut File, source: &VerifiedInputAudio) -> Result<(), CutError> {
    let metadata = file.metadata().map_err(audio_error)?;
    if metadata.len() != source.bytes() {
        return Err(invalid(
            "selected private WAV byte count changed while staging",
        ));
    }
    file.seek(SeekFrom::Start(0)).map_err(audio_error)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(audio_error)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    (format!("{:x}", digest.finalize()) == source.sha256())
        .then_some(())
        .ok_or_else(|| invalid("selected private WAV hash changed while staging"))
}

fn duration_ms<R: Read>(reader: &hound::WavReader<R>) -> Result<u64, CutError> {
    u64::from(reader.duration())
        .checked_mul(1_000)
        .and_then(|frames| frames.checked_div(u64::from(reader.spec().sample_rate)))
        .filter(|duration| *duration > 0)
        .ok_or_else(|| invalid("selected private WAV has no usable duration"))
}

fn open_local(path: &Path) -> Result<File, CutError> {
    if !is_plain_regular_file(path).map_err(|error| invalid(error.to_string()))? {
        return Err(invalid(
            "selected private WAV is missing or linked during staging",
        ));
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
    let file = options.open(path).map_err(audio_error)?;
    {
        let metadata = file.metadata().map_err(audio_error)?;
        metadata.is_file() && !is_reparse(&metadata)
    }
    .then_some(file)
    .ok_or_else(|| invalid("opened selected private WAV is not regular"))
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

fn read_nofollow(path: &Path) -> Result<Vec<u8>, CutError> {
    let mut file = open_local(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(audio_error)?;
    Ok(bytes)
}

fn audio_error(error: std::io::Error) -> CutError {
    invalid(format!("private pause audio I/O failed: {error}"))
}

fn invalid(detail: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "cannot project selected private pause audio",
        detail.into(),
    )
}

#[cfg(test)]
#[path = "audio_tests.rs"]
mod tests;

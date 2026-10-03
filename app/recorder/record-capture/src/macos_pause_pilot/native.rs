//! Native Mic and Core Audio owners for the private macOS pause pilot.

use super::{
    abort_selected, MacosPauseAudioFactory, MacosPauseAudioLayout, MacosPauseAudioOwner,
    MacosPausePilotProfile, MacosPausePilotStarted, MacosSealedAudioRun,
};
use crate::macos_system_tap::SystemAudioTap;
use crate::mic_endpoint::{
    reserve_microphone_capture, spawn_reserved_microphone_capture_unpadded, MicrophoneSource,
};
use record_recovery::RecordingStream;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

/// Creates fixed generation leaves beneath a prepared capture directory. The
/// existing Mic and Core Audio writers both publish through no-replace staging;
/// this owner reopens the resulting leaf before it reports a timing fact.
pub(crate) struct RequiredMacosPauseAudioFactory {
    capture_dir: PathBuf,
    microphone_source: MicrophoneSource,
    microphone_level: Option<Arc<crate::RollingAudioLevel>>,
}

impl RequiredMacosPauseAudioFactory {
    pub(crate) fn new(
        capture_dir: PathBuf,
        microphone_source: MicrophoneSource,
        microphone_level: Option<Arc<crate::RollingAudioLevel>>,
    ) -> Self {
        Self {
            capture_dir,
            microphone_source,
            microphone_level,
        }
    }
}

impl Drop for RequiredMacosPauseAudioFactory {
    fn drop(&mut self) {
        // The factory is retained across Pause/Resume and released only when
        // the logical capture owner exits, including startup/cleanup failure.
        // DeviceLost is more specific and mark_stopped never overwrites it.
        if let Some(level) = self.microphone_level.as_ref() {
            level.mark_stopped();
        }
    }
}

impl MacosPauseAudioFactory for RequiredMacosPauseAudioFactory {
    fn start(
        &mut self,
        profile: &MacosPausePilotProfile,
        started: &MacosPausePilotStarted,
    ) -> Result<Vec<Box<dyn MacosPauseAudioOwner>>, ()> {
        let mut owners: Vec<Box<dyn MacosPauseAudioOwner>> = Vec::new();
        for stream in profile.selected_audio_streams() {
            let artifact = artifact_name(stream, started.physical_generation)?;
            let path = self.capture_dir.join(&artifact);
            let raw_path = self.capture_dir.join(format!(".{artifact}.capture.wav"));
            let owner = match stream {
                RecordingStream::MicrophoneAudio => {
                    match reserve_microphone_capture(&self.microphone_source) {
                        Ok(reserved) => {
                            let stop = Arc::new(AtomicBool::new(false));
                            let ready = Arc::new(AtomicBool::new(false));
                            let handle = spawn_reserved_microphone_capture_unpadded(
                                raw_path.to_string_lossy().into_owned(),
                                stop.clone(),
                                ready,
                                started.monotonic_at,
                                reserved,
                                None,
                                self.microphone_level.clone(),
                            );
                            Ok(Box::new(MicrophoneOwner {
                                stop,
                                handle: Some(handle),
                                artifact,
                                path,
                                raw_path,
                                started: started.clone(),
                            })
                                as Box<dyn MacosPauseAudioOwner>)
                        }
                        Err(_) => Err(()),
                    }
                }
                RecordingStream::SystemAudio => {
                    // ScreenCaptureKit has accepted its output before this
                    // Core Audio tap starts. Preserve that real launch lag in
                    // the native first-packet fact rather than calling it zero.
                    let tap_start_ms = u64::try_from(started.monotonic_at.elapsed().as_millis())
                        .unwrap_or(u64::MAX);
                    let tap = SystemAudioTap::start(tap_start_ms).ok_or(());
                    tap.map(|tap| {
                        Box::new(SystemAudioOwner {
                            tap: Some(tap),
                            artifact,
                            path,
                            started: started.clone(),
                        }) as Box<dyn MacosPauseAudioOwner>
                    })
                }
                _ => Err(()),
            };
            match owner {
                Ok(owner) => owners.push(owner),
                Err(()) => {
                    abort_selected(owners);
                    return Err(());
                }
            }
        }
        Ok(owners)
    }
}

struct MicrophoneOwner {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<record_core::Result<crate::mic::CapturedMicrophone>>>,
    artifact: String,
    path: PathBuf,
    raw_path: PathBuf,
    started: MacosPausePilotStarted,
}

impl MacosPauseAudioOwner for MicrophoneOwner {
    fn stream(&self) -> RecordingStream {
        RecordingStream::MicrophoneAudio
    }

    fn seal_after_screen(
        mut self: Box<Self>,
        screen_raw_end_ms: u64,
    ) -> Result<MacosSealedAudioRun, ()> {
        self.stop.store(true, Ordering::Release);
        let captured = self
            .handle
            .take()
            .ok_or(())?
            .join()
            .map_err(|_| ())?
            .map_err(|_| ())?;
        let expected_path = self.raw_path.to_string_lossy();
        if captured.microphone_lost || captured.path.as_deref() != Some(expected_path.as_ref()) {
            return Err(());
        }
        let first_packet_offset_ms = captured.first_packet_offset_ms.ok_or(())?;
        publish_bounded_microphone_wav(
            &self.raw_path,
            &self.path,
            self.started.observed_start_ms,
            first_packet_offset_ms,
            screen_raw_end_ms,
        )?;
        sealed_file(
            RecordingStream::MicrophoneAudio,
            self.artifact.clone(),
            &self.path,
            &self.started,
            first_packet_offset_ms,
            screen_raw_end_ms,
        )
    }

    fn abort_and_join(mut self: Box<Self>) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for MicrophoneOwner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct SystemAudioOwner {
    tap: Option<SystemAudioTap>,
    artifact: String,
    path: PathBuf,
    started: MacosPausePilotStarted,
}

impl MacosPauseAudioOwner for SystemAudioOwner {
    fn stream(&self) -> RecordingStream {
        RecordingStream::SystemAudio
    }

    fn seal_after_screen(
        mut self: Box<Self>,
        screen_raw_end_ms: u64,
    ) -> Result<MacosSealedAudioRun, ()> {
        let result = self.tap.take().ok_or(())?.finish();
        if result.rc != 0 {
            return Err(());
        }
        let samples = result.samples.ok_or(())?;
        let first_packet_offset_ms = result.first_packet_offset_ms.ok_or(())?;
        let channels = u16::try_from(result.channels)
            .ok()
            .filter(|channels| (1..=8).contains(channels))
            .ok_or(())?;
        let sample_rate = result
            .rate
            .is_finite()
            .then(|| result.rate.round() as u32)
            .filter(|sample_rate| (8_000..=384_000).contains(sample_rate))
            .ok_or(())?;
        let file_name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(())?;
        let count = super::audio::bounded_audio_samples(
            self.started.observed_start_ms,
            screen_raw_end_ms,
            first_packet_offset_ms,
            sample_rate,
            channels,
            samples.as_slice().len(),
        )?;
        crate::macos_system_audio::publish_system_wav_named(
            self.path.parent().ok_or(())?,
            file_name,
            &samples.as_slice()[..count],
            channels,
            sample_rate,
        )
        .map_err(|_| ())?;
        sealed_file(
            RecordingStream::SystemAudio,
            self.artifact.clone(),
            &self.path,
            &self.started,
            first_packet_offset_ms,
            screen_raw_end_ms,
        )
    }

    fn abort_and_join(mut self: Box<Self>) {
        // Explicitly stop the synchronous Core Audio FFI tap when no exact
        // sealed sidecar may be published; do not leave it to RAII fallback.
        if let Some(tap) = self.tap.take() {
            tap.abort();
        }
    }
}

fn artifact_name(stream: RecordingStream, generation: u64) -> Result<String, ()> {
    let prefix = match stream {
        RecordingStream::MicrophoneAudio => "recording-microphone-generation-",
        RecordingStream::SystemAudio => "recording-system-generation-",
        _ => return Err(()),
    };
    (generation != 0)
        .then(|| format!("{prefix}{generation:020}.wav"))
        .ok_or(())
}

fn sealed_file(
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

fn wav_facts(file: &mut File) -> Result<(String, u64), ()> {
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

fn open_local(path: &Path) -> Result<File, ()> {
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
fn publish_bounded_microphone_wav(
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
    let count = super::audio::bounded_audio_samples(
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

#[cfg(test)]
mod boundary_tests {
    use super::*;

    fn raw_wav(path: &Path) {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for index in 0..603_648 * 2 {
            writer.write_sample((index % 30_000) as i16).unwrap();
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn durable_audio_factory_terminalizes_meter_without_reviving_device_loss() {
        for lost in [false, true] {
            let level = Arc::new(crate::RollingAudioLevel::new());
            let factory = RequiredMacosPauseAudioFactory::new(
                PathBuf::new(),
                MicrophoneSource::SystemDefault,
                Some(level.clone()),
            );
            if lost {
                level.mark_device_lost();
            }
            drop(factory);
            level.observe_i16(&[i16::MIN]);
            let snapshot = level.snapshot();
            assert_eq!(
                snapshot.lifecycle,
                if lost {
                    crate::AudioLevelLifecycle::DeviceLost
                } else {
                    crate::AudioLevelLifecycle::Stopped
                }
            );
            assert!(snapshot.stale);
            assert_eq!(snapshot.peak_dbfs, None);
        }
    }

    #[test]
    fn publishes_only_real_pcm_inside_the_screen_boundary_without_replacing_raw() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("raw.wav");
        let final_path = dir.path().join("sealed.wav");
        raw_wav(&raw);
        let original = std::fs::read(&raw).unwrap();
        assert_eq!(hound::WavReader::open(&raw).unwrap().duration(), 603_648);
        publish_bounded_microphone_wav(&raw, &final_path, 15, 37, 10_321).unwrap();
        let mut reader = hound::WavReader::open(&final_path).unwrap();
        assert_eq!(reader.spec().channels, 2);
        assert_eq!(reader.spec().sample_rate, 48_000);
        assert_eq!(reader.duration(), 492_912);
        for (index, sample) in reader.samples::<i16>().enumerate() {
            assert_eq!(sample.unwrap(), (index % 30_000) as i16);
        }
        assert_eq!(std::fs::read(&raw).unwrap(), original);
        let mut sealed_file = open_local(&final_path).unwrap();
        let (hash, duration_ms) = wav_facts(&mut sealed_file).unwrap();
        assert_eq!(duration_ms, 10_269);
        let sealed = std::fs::read(&final_path).unwrap();
        assert_eq!(hash, format!("{:x}", Sha256::digest(&sealed)));
        assert!(publish_bounded_microphone_wav(&raw, &final_path, 15, 37, 10_321).is_err());
        assert_eq!(std::fs::read(&final_path).unwrap(), sealed);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn refuses_missing_interval_and_symlink_without_publishing_or_leaving_staging() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("raw.wav");
        let linked = dir.path().join("link.wav");
        let final_path = dir.path().join("sealed.wav");
        raw_wav(&raw);
        assert!(publish_bounded_microphone_wav(&raw, &final_path, 15, 37, 52).is_err());
        std::os::unix::fs::symlink(&raw, &linked).unwrap();
        assert!(publish_bounded_microphone_wav(&linked, &final_path, 15, 37, 10_321).is_err());
        assert!(!final_path.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}

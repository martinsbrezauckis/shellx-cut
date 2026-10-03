//! Native Mic and Core Audio owners for the private macOS pause pilot.

use super::{
    abort_selected, MacosPauseAudioFactory, MacosPauseAudioOwner, MacosPausePilotProfile,
    MacosPausePilotStarted, MacosSealedAudioRun,
};
use crate::macos_system_tap::SystemAudioTap;
use crate::mic_endpoint::{
    reserve_microphone_capture, spawn_reserved_microphone_capture_unpadded, MicrophoneSource,
};
#[path = "native_audio_files.rs"]
mod audio_files;

use audio_files::{artifact_name, publish_bounded_microphone_wav, sealed_file};
use record_recovery::RecordingStream;
use std::path::PathBuf;
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

#[cfg(test)]
#[path = "native_boundary_tests.rs"]
mod boundary_tests;

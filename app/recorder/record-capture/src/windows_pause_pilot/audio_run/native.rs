//! Real Windows microphone and process-loopback owners for one WGC generation.

use super::sealed_wav::{artifact_name, sealed_file};
use super::*;
use crate::mic_endpoint::{
    reserve_microphone_capture, spawn_reserved_microphone_capture, MicrophoneSource,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

/// The real private Windows source factory. It is passed a prepared capture
/// directory owned by the checkpoint manifest; every fixed leaf is created by
/// the existing no-replace WAV publishers in the microphone/loopback owners.
pub(crate) struct RequiredWindowsPauseAudioFactory {
    capture_dir: PathBuf,
    microphone_source: MicrophoneSource,
}

impl RequiredWindowsPauseAudioFactory {
    pub(crate) fn new(capture_dir: PathBuf, microphone_source: MicrophoneSource) -> Self {
        Self {
            capture_dir,
            microphone_source,
        }
    }
}

impl WindowsPauseAudioFactory for RequiredWindowsPauseAudioFactory {
    fn start(
        &mut self,
        profile: &WindowsPausePilotProfile,
        started: &WindowsPausePilotStarted,
    ) -> Result<Vec<Box<dyn WindowsPauseAudioOwner>>, ()> {
        let mut owners: Vec<Box<dyn WindowsPauseAudioOwner>> = Vec::new();
        for stream in profile.selected_audio_streams() {
            let artifact = artifact_name(stream, started.physical_generation)?;
            let path = self.capture_dir.join(&artifact);
            let stop = Arc::new(AtomicBool::new(false));
            let owner: Box<dyn WindowsPauseAudioOwner> = match stream {
                RecordingStream::MicrophoneAudio => {
                    let reserved =
                        reserve_microphone_capture(&self.microphone_source).map_err(|_| ())?;
                    let ready = Arc::new(AtomicBool::new(false));
                    let handle = spawn_reserved_microphone_capture(
                        path.to_string_lossy().into_owned(),
                        stop.clone(),
                        ready,
                        started.monotonic_at,
                        reserved,
                        None,
                    );
                    Box::new(MicrophoneOwner {
                        stop,
                        handle: Some(handle),
                        artifact,
                        path,
                        started: started.clone(),
                    })
                }
                RecordingStream::SystemAudio => {
                    let worker_stop = stop.clone();
                    let worker_path = path.to_string_lossy().into_owned();
                    let origin = started.monotonic_at;
                    let handle = thread::Builder::new()
                        .name("cut-pause-system-audio".into())
                        .spawn(move || {
                            crate::capture_system_loopback(&worker_path, None, worker_stop, origin)
                        })
                        .map_err(|_| ())?;
                    Box::new(SystemAudioOwner {
                        stop,
                        handle: Some(handle),
                        artifact,
                        path,
                        started: started.clone(),
                    })
                }
                _ => return Err(()),
            };
            owners.push(owner);
        }
        Ok(owners)
    }
}

struct MicrophoneOwner {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<record_core::Result<crate::mic::CapturedMicrophone>>>,
    artifact: String,
    path: PathBuf,
    started: WindowsPausePilotStarted,
}

impl WindowsPauseAudioOwner for MicrophoneOwner {
    fn stream(&self) -> RecordingStream {
        RecordingStream::MicrophoneAudio
    }

    fn seal_after_screen(
        mut self: Box<Self>,
        screen_raw_end_ms: u64,
    ) -> Result<WindowsSealedAudioRun, ()> {
        self.stop.store(true, Ordering::Release);
        let captured = self
            .handle
            .take()
            .ok_or(())?
            .join()
            .map_err(|_| ())?
            .map_err(|_| ())?;
        let expected_path = self.path.to_string_lossy().into_owned();
        if captured.microphone_lost || captured.path.as_deref() != Some(&expected_path) {
            return Err(());
        }
        let native_ready_offset_ms = captured.first_packet_offset_ms.ok_or(())?;
        // These fields leave an owner with Drop, so take them only after the
        // capture thread has stopped and identity checks have succeeded. The
        // owner remains cancelled if `sealed_file` rejects the closed leaf.
        let artifact = std::mem::take(&mut self.artifact);
        let path = std::mem::take(&mut self.path);
        let started = self.started.clone();
        sealed_file(
            RecordingStream::MicrophoneAudio,
            artifact,
            path,
            started,
            native_ready_offset_ms,
            screen_raw_end_ms,
        )
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
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<record_core::Result<crate::SystemAudioCapture>>>,
    artifact: String,
    path: PathBuf,
    started: WindowsPausePilotStarted,
}

impl WindowsPauseAudioOwner for SystemAudioOwner {
    fn stream(&self) -> RecordingStream {
        RecordingStream::SystemAudio
    }

    fn seal_after_screen(
        mut self: Box<Self>,
        screen_raw_end_ms: u64,
    ) -> Result<WindowsSealedAudioRun, ()> {
        self.stop.store(true, Ordering::Release);
        let captured = self
            .handle
            .take()
            .ok_or(())?
            .join()
            .map_err(|_| ())?
            .map_err(|_| ())?;
        let native_ready_offset_ms = captured.first_packet_offset_ms.ok_or(())?;
        if captured.path != self.path.to_string_lossy() {
            return Err(());
        }
        // Keep Drop active until the thread has been joined and the exact
        // producer path has been verified. Taking the values avoids moving
        // out of a Drop type while retaining cancellation on every failure.
        let artifact = std::mem::take(&mut self.artifact);
        let path = std::mem::take(&mut self.path);
        let started = self.started.clone();
        sealed_file(
            RecordingStream::SystemAudio,
            artifact,
            path,
            started,
            native_ready_offset_ms,
            screen_raw_end_ms,
        )
    }
}

impl Drop for SystemAudioOwner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

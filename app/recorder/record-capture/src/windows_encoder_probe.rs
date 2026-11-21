//! File-free preparation of the same Media Foundation writer used by WGC.

use crate::{VideoEncoderPurpose, VideoEncoderReadiness, VideoEncoderRequest};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Foundation::TimeSpan;
use windows::Media::Core::MediaStreamSource;
use windows::Storage::Streams::InMemoryRandomAccessStream;

type Cached = Option<(VideoEncoderRequest, Instant, VideoEncoderReadiness)>;
static CACHE: OnceLock<Mutex<Cached>> = OnceLock::new();

pub(crate) fn readiness(
    request: VideoEncoderRequest,
    check: bool,
    purpose: VideoEncoderPurpose,
) -> VideoEncoderReadiness {
    let fps = record_core::backend_fps_v1(request.fps);
    if check && !crate::screen_access::admit_native_screen_access() {
        let mut result = VideoEncoderReadiness::unchecked(fps);
        result.detail = Some("Native screen access is disabled in this process.".into());
        return result;
    }
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !check {
        return cache
            .as_ref()
            .filter(|(key, at, _)| key == &request && at.elapsed() < Duration::from_secs(30))
            .map(|(_, _, result)| result.clone())
            .unwrap_or_else(|| VideoEncoderReadiness::unchecked(fps));
    }
    let selected = request.clone();
    let result = std::thread::spawn(move || check_source(&selected, fps, purpose))
        .join()
        .unwrap_or_else(|_| {
            VideoEncoderReadiness::from_setup(
                0,
                0,
                fps,
                Err("The video encoder check worker stopped unexpectedly.".into()),
            )
        });
    *cache = Some((request, Instant::now(), result.clone()));
    result
}

fn check_source(
    request: &VideoEncoderRequest,
    fps: u32,
    purpose: VideoEncoderPurpose,
) -> VideoEncoderReadiness {
    let dimensions = dimensions(request, purpose);
    match dimensions {
        Ok((width, height)) => {
            VideoEncoderReadiness::from_setup(width, height, fps, prepare(width, height, fps))
        }
        Err(error) => source_unavailable(fps, error),
    }
}

fn source_unavailable(fps: u32, error: record_core::RecordError) -> VideoEncoderReadiness {
    if error.code == crate::windows_capture_source::WINDOW_MINIMIZED_REASON {
        return VideoEncoderReadiness::minimized_window(fps);
    }
    let mut result = VideoEncoderReadiness::unchecked(fps);
    result.reason = Some("video_encoder_source_unavailable".into());
    result.message = "The selected screen or window is unavailable for a recording check.".into();
    result.detail = Some(error.to_string());
    result
}

fn window_resolution_for_probe(
    purpose: VideoEncoderPurpose,
) -> crate::windows_capture_source::WindowResolution {
    match purpose {
        VideoEncoderPurpose::Doctor => {
            crate::windows_capture_source::WindowResolution::ReadinessCheck
        }
        VideoEncoderPurpose::RecordingStart => {
            crate::windows_capture_source::WindowResolution::RecordingStart
        }
    }
}

fn dimensions(
    request: &VideoEncoderRequest,
    purpose: VideoEncoderPurpose,
) -> record_core::Result<(u32, u32)> {
    let _dpi = crate::windows_runtime::enter_per_monitor_dpi_v2().map_err(|error| {
        crate::windows::cap_err("enter per-monitor DPI coordinate space", error)
    })?;
    if let Some(window) = request.window.as_deref() {
        let (_, width, height) =
            crate::windows_capture_source::window(window, window_resolution_for_probe(purpose))?;
        return Ok((width, height));
    }
    let monitor =
        crate::windows_capture_source::monitor(request.monitor, request.monitor_id.as_deref())?;
    Ok((
        monitor
            .width()
            .map_err(|error| crate::windows::cap_err("monitor width", error))?,
        monitor
            .height()
            .map_err(|error| crate::windows::cap_err("monitor height", error))?,
    ))
}

fn prepare(width: u32, height: u32, fps: u32) -> Result<(), String> {
    let setup = || -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        crate::windows_runtime::pin_process_mta()?;
        // The check runs on a dedicated fresh thread, just like native WGC.
        // SAFETY: this initializes only this thread; the guard balances it.
        unsafe {
            windows::Win32::System::WinRT::RoInitialize(
                windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
            )?;
        }
        let _runtime = ProbeRuntime;
        let (descriptor, profile) =
            crate::windows_wgc_encoder_profile::encoding_properties(width, height, fps)?;
        let source = MediaStreamSource::CreateFromDescriptor(&descriptor)?;
        source.SetBufferTime(TimeSpan { Duration: 300_000 })?;
        let memory = InMemoryRandomAccessStream::new()?;
        let stream = memory.cast()?;
        let prepared = crate::windows_wgc_encoder_setup::prepare(
            &source,
            &stream,
            &profile,
            Some(Duration::from_secs(5)),
        )?;
        // Deliberately do not call TranscodeAsync: no frame or recording is made.
        drop(prepared);
        memory.Close()?;
        Ok(())
    };
    setup().map_err(|error| error.to_string())
}

struct ProbeRuntime;
impl Drop for ProbeRuntime {
    fn drop(&mut self) {
        // SAFETY: this guard is created only after successful RoInitialize on
        // this same dedicated thread and never leaves it.
        unsafe {
            windows::Win32::System::WinRT::RoUninitialize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_requests_non_restoring_resolution_and_start_keeps_restoring() {
        assert_eq!(
            window_resolution_for_probe(crate::VideoEncoderPurpose::Doctor),
            crate::windows_capture_source::WindowResolution::ReadinessCheck
        );
        assert_eq!(
            window_resolution_for_probe(crate::VideoEncoderPurpose::RecordingStart),
            crate::windows_capture_source::WindowResolution::RecordingStart
        );
    }

    #[test]
    fn minimized_window_has_an_unchecked_person_and_agent_answer() {
        let result = source_unavailable(
            30,
            record_core::RecordError::new(
                crate::windows_capture_source::WINDOW_MINIMIZED_REASON,
                "The window is minimized. Restore it, then check again.",
                "minimized",
            ),
        );
        assert!(!result.checked && !result.ready);
        assert_eq!(result.reason.as_deref(), Some("window_minimized"));
        assert_eq!(
            result.message,
            "The window is minimized. Restore it, then check again."
        );
    }
}

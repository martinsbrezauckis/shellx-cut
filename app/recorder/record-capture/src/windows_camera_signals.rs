//! Thread-safe Capture Engine event/sample state and screen-clock projection.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use record_core::Result;
use windows::core::implement;
use windows::Win32::Media::MediaFoundation::{
    IMFCaptureEngineOnEventCallback, IMFCaptureEngineOnEventCallback_Impl,
    IMFCaptureEngineOnSampleCallback, IMFCaptureEngineOnSampleCallback_Impl, IMFMediaEvent,
    IMFSample, MF_CAPTURE_ENGINE_CAMERA_STREAM_BLOCKED, MF_CAPTURE_ENGINE_ERROR,
    MF_CAPTURE_ENGINE_INITIALIZED, MF_CAPTURE_ENGINE_PREVIEW_STARTED,
    MF_CAPTURE_ENGINE_RECORD_STARTED, MF_CAPTURE_ENGINE_RECORD_STOPPED,
};
use windows::Win32::System::Com::{IAgileObject, IAgileObject_Impl};

use super::camera_error;
use crate::CameraFrameObservation;

#[path = "windows_camera_sample.rs"]
mod sample;
use sample::{record_sample, SampleAnchor};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EventKind {
    Initialized,
    PreviewStarted,
    RecordStarted,
    RecordStopped,
}

#[derive(Default)]
struct EventFlags {
    initialized: Option<bool>,
    preview_started: Option<bool>,
    record_started: Option<bool>,
    record_stopped: Option<bool>,
    record_stopped_hresult: Option<u32>,
    accepting_samples: bool,
    device_lost: bool,
    fatal_error: bool,
    first_failure: Option<String>,
    first_sample: Option<SampleAnchor>,
    observations: Vec<CameraFrameObservation>,
    first_native_start_hns: Option<i64>,
    last_native_start_hns: Option<i64>,
    last_native_end_hns: Option<i64>,
}

pub(super) struct StopSignalSnapshot {
    pub(super) record_stopped: Option<bool>,
    pub(super) record_stopped_hresult: Option<u32>,
    pub(super) fatal_error: bool,
    pub(super) first_failure: Option<String>,
}

pub(super) struct CaptureSignals {
    state: Mutex<EventFlags>,
    wake: Condvar,
    screen_origin: Instant,
}

impl CaptureSignals {
    pub(super) fn new(screen_origin: Instant) -> Self {
        Self {
            state: Mutex::new(EventFlags::default()),
            wake: Condvar::new(),
            screen_origin,
        }
    }

    fn on_event(&self, kind: windows::core::GUID, status: windows::core::HRESULT) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let succeeded = status.is_ok();
        if kind == MF_CAPTURE_ENGINE_INITIALIZED {
            state.initialized = Some(succeeded);
        } else if kind == MF_CAPTURE_ENGINE_PREVIEW_STARTED {
            state.preview_started = Some(succeeded);
        } else if kind == MF_CAPTURE_ENGINE_RECORD_STARTED {
            state.record_started = Some(succeeded);
        } else if kind == MF_CAPTURE_ENGINE_RECORD_STOPPED {
            state.record_stopped = Some(succeeded);
            state.record_stopped_hresult = Some(status.0 as u32);
            // End admission under the event lock, before notifying the
            // waiting owner. A preview callback can continue after recording.
            state.accepting_samples = false;
        } else if kind == MF_CAPTURE_ENGINE_CAMERA_STREAM_BLOCKED || kind == MF_CAPTURE_ENGINE_ERROR
        {
            state.first_failure.get_or_insert_with(|| {
                format!("mf_event:kind={kind:?}:status=0x{:08x}", status.0 as u32)
            });
            state.device_lost = true;
            state.fatal_error = true;
        }
        if !succeeded {
            state.first_failure.get_or_insert_with(|| {
                format!("mf_event:kind={kind:?}:status=0x{:08x}", status.0 as u32)
            });
            state.fatal_error = true;
        }
        self.wake.notify_all();
    }

    pub(super) fn wait_for(&self, kind: EventKind, timeout: Duration) -> Result<()> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (state, timeout_result) = self
            .wake
            .wait_timeout_while(state, timeout, |flags| {
                event_result(flags, kind).is_none() && !flags.device_lost && !flags.fatal_error
            })
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.device_lost {
            return Err(event_error(
                "Media Foundation reported Windows camera device loss",
                kind,
                state.first_failure.as_deref(),
            ));
        }
        match event_result(&state, kind) {
            Some(true) => Ok(()),
            Some(false) => Err(event_error(
                "Media Foundation completed the requested camera operation with an error",
                kind,
                state.first_failure.as_deref(),
            )),
            None if timeout_result.timed_out() => Err(event_error(
                "Media Foundation did not complete the requested camera operation before timeout",
                kind,
                state.first_failure.as_deref(),
            )),
            None => Err(event_error(
                "Media Foundation reported a camera failure before the requested operation completed",
                kind,
                state.first_failure.as_deref(),
            )),
        }
    }

    pub(super) fn admit_record_samples(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .accepting_samples = true;
    }

    pub(super) fn stop_accepting_samples(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .accepting_samples = false;
    }

    pub(super) fn wait_for_first_frame(&self, timeout: Duration) -> Result<()> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (state, timeout_result) = self
            .wake
            .wait_timeout_while(state, timeout, |flags| {
                flags.observations.is_empty() && !flags.device_lost && !flags.fatal_error
            })
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.observations.is_empty() {
            Ok(())
        } else if state.device_lost {
            Err(device_lost_error())
        } else if timeout_result.timed_out() {
            Err(camera_error(
                "wait for first delivered camera frame",
                "Media Foundation did not deliver a camera frame after record start",
            ))
        } else {
            Err(camera_error(
                "wait for first delivered camera frame",
                "Media Foundation reported a camera failure before it delivered a frame",
            ))
        }
    }

    fn on_sample(&self, sample_time_hns: i64, sample_duration_hns: i64) {
        let delivered_at = Instant::now();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.accepting_samples || state.fatal_error {
            return;
        }
        let previous_end = state.last_native_end_hns;
        if let Err(reason) = record_sample(
            &mut state,
            self.screen_origin,
            delivered_at,
            sample_time_hns,
            sample_duration_hns,
        ) {
            state.first_failure.get_or_insert_with(|| {
                format!("sample_interval:{reason}:time={sample_time_hns}:duration={sample_duration_hns}:previous_end={previous_end:?}")
            });
            state.fatal_error = true;
        }
        self.wake.notify_all();
    }

    pub(super) fn device_lost(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .device_lost
    }

    fn note_sample_getter_error(&self, getter: &'static str, error: &windows::core::Error) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .first_failure
            .get_or_insert_with(|| format!("sample_getter:{getter}:hresult={:?}", error.code()));
    }

    pub(super) fn stop_snapshot(&self) -> StopSignalSnapshot {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        StopSignalSnapshot {
            record_stopped: state.record_stopped,
            record_stopped_hresult: state.record_stopped_hresult,
            fatal_error: state.fatal_error,
            first_failure: state.first_failure.clone(),
        }
    }

    pub(super) fn sample_span_hns(&self) -> Result<i64> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .first_native_start_hns
            .zip(state.last_native_end_hns)
            .and_then(|(first, end)| end.checked_sub(first))
            .filter(|span| *span > 0)
            .ok_or_else(|| {
                camera_error(
                    "measure Windows camera sample span",
                    "accepted camera callbacks did not establish a positive 100-nanosecond span",
                )
            })
    }

    pub(super) fn take_observations(&self) -> Vec<CameraFrameObservation> {
        std::mem::take(
            &mut self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .observations,
        )
    }
}

fn event_result(flags: &EventFlags, kind: EventKind) -> Option<bool> {
    match kind {
        EventKind::Initialized => flags.initialized,
        EventKind::PreviewStarted => flags.preview_started,
        EventKind::RecordStarted => flags.record_started,
        EventKind::RecordStopped => flags.record_stopped,
    }
}

fn event_error(
    cause: &str,
    requested: EventKind,
    first_failure: Option<&str>,
) -> record_core::RecordError {
    camera_error(
        "wait for Windows Camera Capture Engine event",
        format!(
            "{cause}; requested={requested:?}; first_failure={}",
            first_failure.unwrap_or("none")
        ),
    )
}

fn device_lost_error() -> record_core::RecordError {
    camera_error(
        "wait for Windows Camera Capture Engine event",
        "Media Foundation reported Windows camera device loss",
    )
}

#[implement(IMFCaptureEngineOnEventCallback, IAgileObject)]
pub(super) struct EventCallback {
    pub(super) signals: Arc<CaptureSignals>,
}

impl IMFCaptureEngineOnEventCallback_Impl for EventCallback_Impl {
    fn OnEvent(&self, event: windows::core::Ref<IMFMediaEvent>) -> windows::core::Result<()> {
        let event = event.ok()?;
        unsafe {
            self.signals
                .on_event(event.GetExtendedType()?, event.GetStatus()?);
        }
        Ok(())
    }
}

impl IAgileObject_Impl for EventCallback_Impl {}

#[implement(IMFCaptureEngineOnSampleCallback, IAgileObject)]
pub(super) struct SampleCallback {
    pub(super) signals: Arc<CaptureSignals>,
}

impl IMFCaptureEngineOnSampleCallback_Impl for SampleCallback_Impl {
    fn OnSample(&self, sample: windows::core::Ref<IMFSample>) -> windows::core::Result<()> {
        let sample = sample.ok()?;
        unsafe {
            let sample_time = sample.GetSampleTime().inspect_err(|error| {
                self.signals.note_sample_getter_error("time", error);
            })?;
            let duration = sample.GetSampleDuration().inspect_err(|error| {
                self.signals.note_sample_getter_error("duration", error);
            })?;
            self.signals.on_sample(sample_time, duration);
        }
        Ok(())
    }
}

impl IAgileObject_Impl for SampleCallback_Impl {}

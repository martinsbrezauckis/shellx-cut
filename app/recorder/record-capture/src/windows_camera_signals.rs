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

#[derive(Clone, Copy, PartialEq, Eq)]
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
    accepting_samples: bool,
    device_lost: bool,
    fatal_error: bool,
    first_sample: Option<SampleAnchor>,
    observations: Vec<CameraFrameObservation>,
    first_native_start_hns: Option<i64>,
    last_native_end_hns: Option<i64>,
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
        } else if kind == MF_CAPTURE_ENGINE_CAMERA_STREAM_BLOCKED || kind == MF_CAPTURE_ENGINE_ERROR
        {
            state.device_lost = true;
            state.fatal_error = true;
        }
        if !succeeded {
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
            return Err(device_lost_error());
        }
        match event_result(&state, kind) {
            Some(true) => Ok(()),
            Some(false) => Err(event_error(
                "Media Foundation completed the requested camera operation with an error",
            )),
            None if timeout_result.timed_out() => Err(event_error(
                "Media Foundation did not complete the requested camera operation before timeout",
            )),
            None => Err(event_error(
                "Media Foundation reported a camera failure before the requested operation completed",
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
        if record_sample(
            &mut state,
            self.screen_origin,
            delivered_at,
            sample_time_hns,
            sample_duration_hns,
        )
        .is_err()
        {
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

    pub(super) fn fatal_error(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .fatal_error
    }

    pub(super) fn event_succeeded(&self, kind: EventKind) -> bool {
        event_result(
            &self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            kind,
        ) == Some(true)
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

fn event_error(cause: &str) -> record_core::RecordError {
    camera_error("wait for Windows Camera Capture Engine event", cause)
}

fn device_lost_error() -> record_core::RecordError {
    camera_error(
        "wait for Windows Camera Capture Engine event",
        "Media Foundation reported Windows camera device loss",
    )
}

struct SampleAnchor {
    native_start_hns: i64,
    projected_start: Instant,
}

fn record_sample(
    state: &mut EventFlags,
    screen_origin: Instant,
    delivered_at: Instant,
    sample_time_hns: i64,
    sample_duration_hns: i64,
) -> std::result::Result<(), ()> {
    if sample_time_hns < 0 || sample_duration_hns <= 0 {
        return Err(());
    }
    let native_end_hns = sample_time_hns.checked_add(sample_duration_hns).ok_or(())?;
    if state
        .last_native_end_hns
        .is_some_and(|previous| sample_time_hns < previous || native_end_hns <= previous)
    {
        return Err(());
    }
    let anchor = state.first_sample.get_or_insert(SampleAnchor {
        native_start_hns: sample_time_hns,
        projected_start: ceil_screen_millisecond(screen_origin, delivered_at)?,
    });
    let started_at = map_native_time(anchor, sample_time_hns)?;
    let ended_at = map_native_time(anchor, native_end_hns)?;
    if ended_at <= started_at {
        return Err(());
    }
    state
        .observations
        .push(CameraFrameObservation::new(started_at, ended_at));
    state.first_native_start_hns.get_or_insert(sample_time_hns);
    state.last_native_end_hns = Some(native_end_hns);
    Ok(())
}

fn ceil_screen_millisecond(
    screen_origin: Instant,
    delivered_at: Instant,
) -> std::result::Result<Instant, ()> {
    let elapsed = delivered_at
        .checked_duration_since(screen_origin)
        .unwrap_or_default();
    let whole_ms = u64::try_from(elapsed.as_millis()).map_err(|_| ())?;
    let rounded_ms = whole_ms
        .checked_add(u64::from(elapsed.subsec_nanos() % 1_000_000 != 0))
        .ok_or(())?;
    screen_origin
        .checked_add(Duration::from_millis(rounded_ms))
        .ok_or(())
}

fn map_native_time(anchor: &SampleAnchor, native_hns: i64) -> std::result::Result<Instant, ()> {
    let delta_hns = native_hns.checked_sub(anchor.native_start_hns).ok_or(())?;
    let delta = Duration::from_nanos(delta_hns.unsigned_abs().checked_mul(100).ok_or(())?);
    if delta_hns >= 0 {
        anchor.projected_start.checked_add(delta).ok_or(())
    } else {
        anchor.projected_start.checked_sub(delta).ok_or(())
    }
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
            self.signals
                .on_sample(sample.GetSampleTime()?, sample.GetSampleDuration()?);
        }
        Ok(())
    }
}

impl IAgileObject_Impl for SampleCallback_Impl {}

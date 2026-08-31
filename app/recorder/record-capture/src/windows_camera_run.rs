//! Capture Engine start/stop and handle-anchored output finalization.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use record_core::Result;
use windows::core::{IUnknown, Interface};
use windows::Win32::Media::MediaFoundation::{
    CLSID_MFCaptureEngine, CLSID_MFCaptureEngineClassFactory, IMFAttributes, IMFByteStream,
    IMFCaptureEngine, IMFCaptureEngineClassFactory, IMFCaptureEngineOnEventCallback,
    IMFCaptureEngineOnSampleCallback, IMFCapturePreviewSink,
    MF_CAPTURE_ENGINE_PREFERRED_SOURCE_STREAM_FOR_VIDEO_PREVIEW,
    MF_CAPTURE_ENGINE_SINK_TYPE_PREVIEW,
};
use windows::Win32::System::Com::{CoCreateInstance, IStream, CLSCTX_INPROC_SERVER};

use super::windows_camera_devices::{find_device, CameraComApartment, MediaFoundationLease};
use super::windows_camera_record::{configure_record, PreparedRecord};
use super::windows_camera_signals::{CaptureSignals, EventCallback, EventKind, SampleCallback};
use super::{camera_error, windows_error};
use crate::camera_finalization::{
    finalize_windows_no_replace, CameraMediaSeal, WindowsNoReplaceCameraStage,
};
use crate::CameraFrameObservation;

const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);
const CAPTURE_EVENT_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) struct NativeCameraRun {
    // Keep all engine-owned objects before the MF/COM leases. `release_native`
    // takes them explicitly on every terminal path, including Close failure.
    engine: Option<IMFCaptureEngine>,
    events: Option<IMFCaptureEngineOnEventCallback>,
    samples: Option<IMFCaptureEngineOnSampleCallback>,
    output: Option<IMFByteStream>,
    writer_stream: Option<IStream>,
    signals: Arc<CaptureSignals>,
    stage: Option<WindowsNoReplaceCameraStage>,
    _mf: MediaFoundationLease,
    _com: CameraComApartment,
}

pub(super) struct StoppedCameraRun {
    pub(super) device_lost: bool,
    pub(super) observations: Vec<CameraFrameObservation>,
    pub(super) seal: Option<CameraMediaSeal>,
}

impl NativeCameraRun {
    pub(super) fn start(
        capture_directory: &Path,
        capture_id: &str,
        device_id: &str,
        screen_origin: Instant,
    ) -> Result<Self> {
        let com = CameraComApartment::initialize()?;
        let mf = MediaFoundationLease::start()?;
        // Keep this encompassing lease live from enumeration through engine
        // release: the selected IMFActivate must never outlive MFStartup.
        let selected = find_device(device_id)?.ok_or_else(|| {
            camera_error(
                "start Windows camera capture",
                "selected Windows camera is no longer enumerated",
            )
        })?;
        let signals = Arc::new(CaptureSignals::new(screen_origin));
        let event_callback: IMFCaptureEngineOnEventCallback = EventCallback {
            signals: signals.clone(),
        }
        .into();
        let sample_callback: IMFCaptureEngineOnSampleCallback = SampleCallback {
            signals: signals.clone(),
        }
        .into();
        // SAFETY: this run owns the selected activation, engine, callbacks, and
        // output stream until a terminal teardown releases them in that order.
        unsafe {
            let factory: IMFCaptureEngineClassFactory = CoCreateInstance(
                &CLSID_MFCaptureEngineClassFactory,
                None,
                CLSCTX_INPROC_SERVER,
            )
            .map_err(|error| {
                windows_error("create Windows Camera Capture Engine factory", error)
            })?;
            let engine: IMFCaptureEngine = factory
                .CreateInstance(&CLSID_MFCaptureEngine)
                .map_err(|error| windows_error("create Windows Camera Capture Engine", error))?;
            let video_source: IUnknown = selected
                .activation
                .cast()
                .map_err(|error| windows_error("bind selected Windows camera", error))?;
            engine
                .Initialize(
                    &event_callback,
                    None::<&IMFAttributes>,
                    None::<&IUnknown>,
                    &video_source,
                )
                .map_err(|error| {
                    windows_error("initialize Windows Camera Capture Engine", error)
                })?;
            if let Err(error) = signals.wait_for(EventKind::Initialized, CAPTURE_EVENT_TIMEOUT) {
                release_start_failure(
                    engine,
                    event_callback,
                    sample_callback,
                    signals,
                    None,
                    false,
                );
                return Err(error);
            }
            configure_preview(&engine, &sample_callback)?;
            if let Err(error) = engine.StartPreview() {
                release_start_failure(
                    engine,
                    event_callback,
                    sample_callback,
                    signals,
                    None,
                    false,
                );
                return Err(windows_error(
                    "start Windows camera preview sample delivery",
                    error,
                ));
            }
            if let Err(error) = signals.wait_for(EventKind::PreviewStarted, CAPTURE_EVENT_TIMEOUT) {
                release_start_failure(
                    engine,
                    event_callback,
                    sample_callback,
                    signals,
                    None,
                    false,
                );
                return Err(error);
            }
            let prepared = configure_record(capture_directory, capture_id, &engine)?;
            if let Err(error) = engine.StartRecord() {
                release_start_failure(
                    engine,
                    event_callback,
                    sample_callback,
                    signals,
                    Some(prepared),
                    false,
                );
                return Err(windows_error("start Windows camera recording", error));
            }
            if let Err(error) = signals.wait_for(EventKind::RecordStarted, CAPTURE_EVENT_TIMEOUT) {
                release_start_failure(
                    engine,
                    event_callback,
                    sample_callback,
                    signals,
                    Some(prepared),
                    true,
                );
                return Err(error);
            }
            signals.admit_record_samples();
            if let Err(error) = signals.wait_for_first_frame(FIRST_FRAME_TIMEOUT) {
                // Do not leave a deterministic CREATE_NEW stage behind. Wait
                // for RecordStopped/terminal loss, then release every engine
                // alias before stage Drop requests deletion of that exact leaf.
                release_start_failure(
                    engine,
                    event_callback,
                    sample_callback,
                    signals,
                    Some(prepared),
                    true,
                );
                return Err(error);
            }
            Ok(Self {
                engine: Some(engine),
                events: Some(event_callback),
                samples: Some(sample_callback),
                output: Some(prepared.output),
                writer_stream: Some(prepared.writer_stream),
                signals,
                stage: Some(prepared.stage),
                _mf: mf,
                _com: com,
            })
        }
    }

    pub(super) fn stop(mut self) -> Result<StoppedCameraRun> {
        let stop_result = self
            .engine
            .as_ref()
            .ok_or_else(|| {
                camera_error(
                    "stop Windows camera recording",
                    "Capture Engine is unavailable before stop",
                )
            })
            .and_then(|engine| unsafe {
                engine
                    .StopRecord(true, true)
                    .map_err(|error| windows_error("stop Windows camera recording", error))
            });
        if stop_result.is_ok() {
            let _ = self
                .signals
                .wait_for(EventKind::RecordStopped, CAPTURE_EVENT_TIMEOUT);
        }
        self.signals.stop_accepting_samples();
        let device_lost = self.signals.device_lost();
        let observations = self.signals.take_observations();
        let sample_span_hns = if observations.is_empty() {
            None
        } else {
            self.signals.sample_span_hns().ok()
        };
        let native_stop_succeeded = stop_result.is_ok()
            && self.signals.event_succeeded(EventKind::RecordStopped)
            && !self.signals.fatal_error();
        let close_error = self.release_native();
        if let Some(error) = close_error {
            return Err(error);
        }
        if !native_stop_succeeded {
            if device_lost {
                return Ok(StoppedCameraRun {
                    device_lost,
                    observations,
                    seal: None,
                });
            }
            return Err(camera_error(
                "stop Windows camera recording",
                "Camera Capture Engine did not confirm a successful record stop",
            ));
        }
        let seal = match (observations.is_empty(), sample_span_hns, self.stage.take()) {
            (false, Some(span_hns), Some(stage)) => {
                Some(finalize_windows_no_replace(stage, span_hns)?)
            }
            (false, None, _) => {
                return Err(camera_error(
                    "finalize Windows camera recording",
                    "accepted camera frames have no exact native 100-nanosecond span",
                ));
            }
            (_, _, _) => None,
        };
        Ok(StoppedCameraRun {
            device_lost,
            observations,
            seal,
        })
    }

    fn release_native(&mut self) -> Option<record_core::RecordError> {
        let close_error = self.output.take().and_then(|output| unsafe {
            output
                .Close()
                .err()
                .map(|error| windows_error("close Windows camera recording output", error))
        });
        if let Some(engine) = self.engine.as_ref() {
            let _ = unsafe { engine.StopPreview() };
        }
        // This ordering is intentional: no Capture Engine or callback can
        // retain the byte stream when the MF and COM leases later drop.
        drop(self.engine.take());
        drop(self.events.take());
        drop(self.samples.take());
        drop(self.writer_stream.take());
        close_error
    }
}

fn release_start_failure(
    engine: IMFCaptureEngine,
    events: IMFCaptureEngineOnEventCallback,
    samples: IMFCaptureEngineOnSampleCallback,
    signals: Arc<CaptureSignals>,
    prepared: Option<PreparedRecord>,
    record_start_accepted: bool,
) {
    signals.stop_accepting_samples();
    let _ = unsafe { engine.StopRecord(true, true) };
    if record_start_accepted {
        let _ = signals.wait_for(EventKind::RecordStopped, CAPTURE_EVENT_TIMEOUT);
    }
    let close_error = prepared
        .as_ref()
        .and_then(|record| unsafe { record.output.Close().err() });
    let _ = unsafe { engine.StopPreview() };
    drop(engine);
    drop(events);
    drop(samples);
    if let Some(record) = prepared {
        drop(record.output);
        drop(record.writer_stream);
        drop(record.stage);
    }
    drop(close_error);
}

fn configure_preview(
    engine: &IMFCaptureEngine,
    samples: &IMFCaptureEngineOnSampleCallback,
) -> Result<()> {
    // SAFETY: every interface belongs to this initialized Capture Engine.
    unsafe {
        let preview: IMFCapturePreviewSink = engine
            .GetSink(MF_CAPTURE_ENGINE_SINK_TYPE_PREVIEW)
            .map_err(|error| windows_error("open Windows camera preview sink", error))?
            .cast()
            .map_err(|error| windows_error("cast Windows camera preview sink", error))?;
        preview
            .RemoveAllStreams()
            .map_err(|error| windows_error("reset Windows camera preview sink", error))?;
        let source = engine
            .GetSource()
            .map_err(|error| windows_error("open Windows camera source", error))?;
        let media_type = source
            .GetCurrentDeviceMediaType(
                MF_CAPTURE_ENGINE_PREFERRED_SOURCE_STREAM_FOR_VIDEO_PREVIEW.0,
            )
            .map_err(|error| windows_error("read Windows camera preview media type", error))?;
        let mut sink_index = 0_u32;
        preview
            .AddStream(
                MF_CAPTURE_ENGINE_PREFERRED_SOURCE_STREAM_FOR_VIDEO_PREVIEW.0,
                &media_type,
                None::<&IMFAttributes>,
                Some(&mut sink_index),
            )
            .map_err(|error| windows_error("connect Windows camera preview stream", error))?;
        preview
            .SetSampleCallback(sink_index, samples)
            .map_err(|error| windows_error("receive Windows camera preview samples", error))
    }
}

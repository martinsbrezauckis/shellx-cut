//! Exact native camera Stop, output release, and failure evidence.

use super::*;

impl NativeCameraRun {
    pub(in super::super) fn stop(mut self) -> Result<StoppedCameraRun> {
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
                // Finalize and drain queued frames instead of flushing away
                // samples that have already entered the recording pipeline.
                engine.StopRecord(true, false).map_err(|error| {
                    camera_error(
                        "stop Windows camera recording",
                        format!(
                            "StopRecord hresult=0x{:08x}: {error}",
                            error.code().0 as u32
                        ),
                    )
                })
            });
        let wait_result = if stop_result.is_ok() {
            Some(
                self.signals
                    .wait_for(EventKind::RecordStopped, CAPTURE_EVENT_TIMEOUT),
            )
        } else {
            None
        };
        self.signals.stop_accepting_samples();
        let device_lost = self.signals.device_lost();
        let mut observations = self.signals.take_observations();
        let sample_span_hns = if observations.is_empty() {
            None
        } else {
            self.signals.sample_span_hns().ok()
        };
        let stop_signals = self.signals.stop_snapshot();
        let native_stop_succeeded = stop_result.is_ok()
            && stop_signals.record_stopped == Some(true)
            && !stop_signals.fatal_error;
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
            let api = stop_result
                .as_ref()
                .err()
                .map(ToString::to_string)
                .unwrap_or_else(|| "ok".into());
            let wait = match wait_result.as_ref() {
                Some(Ok(())) => "ok".into(),
                Some(Err(error)) => error.to_string(),
                None => "not_attempted".into(),
            };
            return Err(camera_error(
                "stop Windows camera recording",
                format!(
                    "Camera Capture Engine did not confirm a successful record stop; StopRecord={api}; RecordStoppedWait={wait}; RecordStoppedEvent={:?}; RecordStoppedHresult={:?}; fatalError={}; firstFailure={:?}",
                    stop_signals.record_stopped,
                    stop_signals.record_stopped_hresult.map(|status| format!("0x{status:08x}")),
                    stop_signals.fatal_error,
                    stop_signals.first_failure,
                ),
            ));
        }
        let seal = match (observations.is_empty(), sample_span_hns, self.stage.take()) {
            (false, Some(span_hns), Some(stage)) => {
                let frame_count = u64::try_from(observations.len()).map_err(|_| {
                    camera_error(
                        "finalize Windows camera recording",
                        "native frame count overflowed",
                    )
                })?;
                let (seal, encoded_duration) =
                    finalize_windows_no_replace(stage, span_hns, frame_count)?;
                let first = observations[0].started_at;
                let end = first.checked_add(encoded_duration).ok_or_else(|| {
                    camera_error(
                        "project finalized Windows camera interval",
                        "encoded sample span overflows the shared clock",
                    )
                })?;
                let last = observations
                    .last_mut()
                    .expect("nonempty camera observations");
                if end <= last.started_at {
                    return Err(camera_error(
                        "project finalized Windows camera interval",
                        "encoded sample end does not follow the last native frame",
                    ));
                }
                // Preserve the admitted first-frame CaptureClock anchor and
                // native starts; project the verified encoded end exactly.
                last.ended_at = end;
                Some(seal)
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

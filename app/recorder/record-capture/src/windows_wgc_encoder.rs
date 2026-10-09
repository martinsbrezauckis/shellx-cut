//! Video-only WGC encoder with explicit input and output cadence.
//!
//! windows-capture's encoder sets only the output profile rate, leaving the
//! uncompressed input descriptor at Windows' implicit rate. Own this narrow
//! stream boundary so a requested 24 fps also reaches the input descriptor.

use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use windows::core::HSTRING;
use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Graphics::DirectX::Direct3D11::IDirect3DSurface;
use windows::Media::Core::{
    MediaStreamSample, MediaStreamSource, MediaStreamSourceSampleRequestedEventArgs,
    MediaStreamSourceStartingEventArgs,
};
use windows::Media::Transcoding::MediaTranscoder;
use windows::Storage::{FileAccessMode, StorageFile};
use windows::System::Threading::{ThreadPool, WorkItemHandler, WorkItemOptions, WorkItemPriority};
use windows_capture::{d3d11::SendDirectX, frame::Frame};

type Error = Box<dyn std::error::Error + Send + Sync>;
type Sample = (SendDirectX<IDirect3DSurface>, i64, Option<i64>);

pub(crate) struct WgcVideoEncoder {
    sender: Option<mpsc::SyncSender<Sample>>,
    worker: Option<JoinHandle<windows::core::Result<()>>>,
    source: MediaStreamSource,
    starting: i64,
    requested: i64,
    scheduling_failure: Arc<Mutex<Option<windows::core::Error>>>,
    clock: crate::windows_wgc_sample_clock::WgcSampleClock,
    timed_origin: Option<(Instant, crate::windows_wgc_clock_origin::QpcCaptureOrigin)>,
    last_accepted: Option<(SendDirectX<IDirect3DSurface>, i64, Instant)>,
    terminal_sample_duration: i64,
    width: u32,
    height: u32,
    window_fitter: Option<crate::windows_wgc_window_fit::WindowFrameFitter>,
}

impl WgcVideoEncoder {
    #[cfg(test)]
    pub(crate) fn new(width: u32, height: u32, fps: u32, path: &Path) -> Result<Self, Error> {
        Self::new_with_clock(width, height, fps, path, None)
    }

    pub(crate) fn new_with_clock(
        width: u32,
        height: u32,
        fps: u32,
        path: &Path,
        timed_clock: Option<(
            Instant,
            crate::windows_wgc_clock_origin::QpcCaptureOrigin,
            i64,
        )>,
    ) -> Result<Self, Error> {
        if fps == 0 {
            return Err("WGC video frame rate must be positive".into());
        }
        let (descriptor, profile) =
            crate::windows_wgc_encoder_profile::encoding_properties(width, height, fps)?;
        let source = MediaStreamSource::CreateFromDescriptor(&descriptor)?;
        source.SetBufferTime(TimeSpan { Duration: 300_000 })?;
        let starting = source.Starting(&TypedEventHandler::<
            MediaStreamSource,
            MediaStreamSourceStartingEventArgs,
        >::new(|_, args| {
            let args = args.as_ref().ok_or_else(windows::core::Error::empty)?;
            args.Request()?
                .SetActualStartPosition(TimeSpan { Duration: 0 })
        }))?;
        let (sender, receiver) =
            crate::windows_wgc_sample_queue::sample_queue::<Sample>(width, height)?;
        let receiver = Arc::new(Mutex::new(receiver));
        let scheduling_failure = Arc::new(Mutex::new(None));
        let callback_failure = scheduling_failure.clone();
        let requested = source.SampleRequested(&TypedEventHandler::<
            MediaStreamSource,
            MediaStreamSourceSampleRequestedEventArgs,
        >::new(move |_, args| {
            let args = args.as_ref().ok_or_else(windows::core::Error::empty)?;
            let request = args.Request()?;
            let deferral = request.GetDeferral()?;
            let receiver = receiver.clone();
            let sample_request = request.clone();
            let sample_deferral = deferral.clone();
            let scheduled = ThreadPool::RunWithPriorityAndOptionsAsync(
                &WorkItemHandler::new(move |_| {
                    let sample = receiver
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .recv();
                    let delivered = match sample {
                        Ok((surface, timestamp, duration)) => {
                            MediaStreamSample::CreateFromDirect3D11Surface(
                                &surface.0,
                                TimeSpan {
                                    Duration: timestamp,
                                },
                            )
                            .and_then(|sample| {
                                if let Some(duration) = duration {
                                    sample.SetDuration(TimeSpan { Duration: duration })?;
                                }
                                sample_request.SetSample(&sample)
                            })
                        }
                        Err(_) => sample_request.SetSample(None),
                    };
                    // Always release the async request, including API errors.
                    let completed = sample_deferral.Complete();
                    delivered.and(completed)
                }),
                WorkItemPriority::Normal,
                WorkItemOptions::None,
            );
            if let Err(error) = scheduled {
                // No closure owns this request when scheduling fails. Supply
                // EOS and release its deferral before reporting the failure,
                // so terminal flush never waits on an abandoned request.
                let _ = request.SetSample(None);
                let _ = deferral.Complete();
                *callback_failure
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(error.clone());
                return Err(error);
            }
            Ok(())
        }))?;
        std::fs::File::create(path)?;
        let path = std::fs::canonicalize(path)?;
        let path = path.to_string_lossy();
        let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path))?.join()?;
        let stream = file.OpenAsync(FileAccessMode::ReadWrite)?.join()?;
        let transcoder = MediaTranscoder::new()?;
        transcoder.SetHardwareAccelerationEnabled(true)?;
        let prepared = transcoder
            .PrepareMediaStreamSourceTranscodeAsync(&source, &stream, &profile)?
            .join()?;
        if !prepared.CanTranscode()? {
            return Err(format!(
                "WGC video encoder cannot transcode: {:?}",
                prepared.FailureReason()?
            )
            .into());
        }
        let worker = std::thread::spawn(move || {
            let result = prepared.TranscodeAsync()?.join();
            drop(transcoder);
            result
        });
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
            source,
            starting,
            requested,
            scheduling_failure,
            clock: timed_clock.map_or_else(Default::default, |(_, _, boundary)| {
                crate::windows_wgc_sample_clock::WgcSampleClock::with_reserved_boundary(boundary)
            }),
            timed_origin: timed_clock.map(|(start, origin, _)| (start, origin)),
            last_accepted: None,
            terminal_sample_duration: (10_000_000 / i64::from(fps)).max(1),
            width,
            height,
            window_fitter: None,
        })
    }

    pub(crate) fn send_frame(&mut self, frame: &Frame<'_>) -> Result<bool, Error> {
        let native = frame.timestamp()?.Duration;
        let surface = crate::windows_wgc_surface::snapshot(frame, self.width, self.height)?;
        self.send_snapshot(surface, native, Instant::now())
    }

    /// Window-only: return the exact fit only after immutable sample admission.
    pub(crate) fn send_window_frame(
        &mut self,
        frame: &Frame<'_>,
    ) -> Result<Option<crate::window_frame_fit::WindowFrameFit>, Error> {
        let fit = crate::window_frame_fit::WindowFrameFit::new(
            (frame.width(), frame.height()),
            (self.width, self.height),
        )
        .ok_or("invalid full-window fit dimensions")?;
        let native = frame.timestamp()?.Duration;
        let (surface, fit) = if fit.is_identity() {
            (
                crate::windows_wgc_surface::snapshot(frame, self.width, self.height)?,
                fit,
            )
        } else {
            if self.window_fitter.is_none() {
                self.window_fitter = Some(crate::windows_wgc_window_fit::WindowFrameFitter::new(
                    frame.device(),
                )?);
            }
            self.window_fitter
                .as_mut()
                .ok_or("window fitter unavailable")?
                .snapshot(frame, (self.width, self.height))?
        };
        Ok(self
            .send_snapshot(surface, native, Instant::now())?
            .then_some(fit))
    }

    fn send_snapshot(
        &mut self,
        surface: IDirect3DSurface,
        native: i64,
        accepted_at: Instant,
    ) -> Result<bool, Error> {
        if let Some(error) = self
            .scheduling_failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            return Err(error.clone().into());
        }
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            return Err("WGC video encoder terminated before capture close".into());
        }
        let Some(timestamp) = self.clock.admit_timestamp(native)? else {
            return Ok(false);
        };
        let retained = crate::windows_wgc_surface::retained_copy(&surface)?;
        // Keep the native elapsed clock. No frame-count-derived timestamps or
        // forced durations may compress sparse callbacks or capture stalls.
        crate::windows_wgc_sample_queue::offer(
            self.sender
                .as_ref()
                .ok_or("WGC video encoder already closed")?,
            (SendDirectX::new(surface), timestamp, None),
        )?;
        // Only the latest accepted independent copy remains owned. The queued
        // resource can be consumed without changing the eventual held pixels.
        self.last_accepted = Some((SendDirectX::new(retained), timestamp, accepted_at));
        Ok(true)
    }

    /// Called only after the native capture control joined successfully. A
    /// still desktop gets one final same-pixel sample at the observed Stop
    /// boundary, before EOS; failed Stop and source loss use ordinary close.
    pub(crate) fn finish_after_control_stop(
        mut self,
        stop_at: Instant,
    ) -> Result<Option<u64>, Error> {
        let held_ms = self.append_held_sample(stop_at)?;
        self.close()?;
        Ok(held_ms)
    }

    fn append_held_sample(&mut self, stop_at: Instant) -> Result<Option<u64>, Error> {
        let Some((surface, last_timestamp, accepted_at)) = self.last_accepted.take() else {
            return Ok(None);
        };
        let (timestamp, elapsed_100ns) =
            self.clock
                .held_at_stop(last_timestamp, accepted_at, stop_at, self.timed_origin)?;
        // An ordinary final frame already spans one requested frame period.
        // Avoid an extra queue entry at normal cadence, especially when the
        // bounded queue is full at Stop.
        if elapsed_100ns <= self.terminal_sample_duration {
            return Ok(None);
        }
        if let Some(error) = self
            .scheduling_failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            return Err(error.clone().into());
        }
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            return Err("WGC video encoder terminated before held sample".into());
        }
        crate::windows_wgc_sample_queue::offer(
            self.sender
                .as_ref()
                .ok_or("WGC video encoder already closed")?,
            (surface, timestamp, Some(self.terminal_sample_duration)),
        )?;
        Ok(Some(
            u64::try_from(elapsed_100ns / 10_000).unwrap_or(u64::MAX),
        ))
    }

    pub(crate) fn finish(mut self) -> Result<(), Error> {
        self.close()
    }

    fn close(&mut self) -> Result<(), Error> {
        // Closing the channel supplies EOS to every pending sample request.
        self.sender.take();
        let result = self
            .worker
            .take()
            .map(|worker| {
                worker
                    .join()
                    .map_err(|_| "WGC transcoder thread panicked")?
                    .map_err(Error::from)
            })
            .transpose();
        let starting = self.source.RemoveStarting(self.starting);
        let requested = self.source.RemoveSampleRequested(self.requested);
        result?;
        if let Some(error) = self
            .scheduling_failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            return Err(error.into());
        }
        starting?;
        requested?;
        Ok(())
    }
}

impl Drop for WgcVideoEncoder {
    fn drop(&mut self) {
        if self.sender.is_some() || self.worker.is_some() {
            let _ = self.close();
        }
    }
}

#[cfg(test)]
#[path = "windows_wgc_encoder_tests.rs"]
mod tests;

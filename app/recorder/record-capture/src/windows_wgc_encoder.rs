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
    MediaStreamSourceStartingEventArgs, VideoStreamDescriptor,
};
use windows::Media::MediaProperties::{
    MediaEncodingProfile, MediaEncodingSubtypes, VideoEncodingProperties, VideoEncodingQuality,
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
    last_accepted: Option<(SendDirectX<IDirect3DSurface>, i64, Instant)>,
    terminal_sample_duration: i64,
    width: u32,
    height: u32,
}

impl WgcVideoEncoder {
    pub(crate) fn new(width: u32, height: u32, fps: u32, path: &Path) -> Result<Self, Error> {
        if fps == 0 {
            return Err("WGC video frame rate must be positive".into());
        }
        let (descriptor, profile) = encoding_properties(width, height, fps)?;
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
            clock: Default::default(),
            last_accepted: None,
            terminal_sample_duration: (10_000_000 / i64::from(fps)).max(1),
            width,
            height,
        })
    }

    pub(crate) fn send_frame(&mut self, frame: &Frame<'_>) -> Result<(), Error> {
        let native = frame.timestamp()?.Duration;
        let surface = crate::windows_wgc_surface::snapshot(frame, self.width, self.height)?;
        self.send_snapshot(surface, native, Instant::now())
    }

    fn send_snapshot(
        &mut self,
        surface: IDirect3DSurface,
        native: i64,
        accepted_at: Instant,
    ) -> Result<(), Error> {
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
        let timestamp = self.clock.timestamp(native)?;
        // Keep the native elapsed clock. No frame-count-derived timestamps or
        // forced durations may compress sparse callbacks or capture stalls.
        crate::windows_wgc_sample_queue::offer(
            self.sender
                .as_ref()
                .ok_or("WGC video encoder already closed")?,
            (SendDirectX::new(surface.clone()), timestamp, None),
        )?;
        // This is another reference to the queued immutable GPU texture, not
        // a second GPU copy. Only the latest accepted surface stays owned.
        self.last_accepted = Some((SendDirectX::new(surface), timestamp, accepted_at));
        Ok(())
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
        let elapsed = stop_at.saturating_duration_since(accepted_at);
        let elapsed_100ns = i64::try_from(elapsed.as_nanos() / 100)
            .map_err(|_| "WGC held pixel interval overflow")?;
        // An ordinary final frame already spans one requested frame period.
        // Avoid an extra queue entry at normal cadence, especially when the
        // bounded queue is full at Stop.
        if elapsed_100ns <= self.terminal_sample_duration {
            return Ok(None);
        }
        let timestamp = crate::windows_wgc_sample_clock::WgcSampleClock::held_timestamp(
            last_timestamp,
            elapsed_100ns,
        )?;
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
        Ok(Some(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)))
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

fn set_cadence(properties: &VideoEncodingProperties, fps: u32) -> windows::core::Result<()> {
    properties.FrameRate()?.SetNumerator(fps)?;
    properties.FrameRate()?.SetDenominator(1)
}

fn encoding_properties(
    width: u32,
    height: u32,
    fps: u32,
) -> windows::core::Result<(VideoStreamDescriptor, MediaEncodingProfile)> {
    let input = VideoEncodingProperties::CreateUncompressed(
        &MediaEncodingSubtypes::Bgra8()?,
        width,
        height,
    )?;
    // Both sides must describe the same requested timebase. Setting only
    // the output rate lets the implicit input rate drive transcoding.
    set_cadence(&input, fps)?;
    let profile = MediaEncodingProfile::CreateMp4(VideoEncodingQuality::HD1080p)?;
    profile.SetAudio(None)?;
    let output = profile.Video()?;
    output.SetSubtype(&MediaEncodingSubtypes::Hevc()?)?;
    output.SetWidth(width)?;
    output.SetHeight(height)?;
    output.SetBitrate(15_000_000)?;
    set_cadence(&output, fps)?;
    Ok((VideoStreamDescriptor::Create(&input)?, profile))
}

#[cfg(test)]
#[path = "windows_wgc_encoder_tests.rs"]
mod tests;

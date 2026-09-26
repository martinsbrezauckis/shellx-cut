//! First-delivered-screen-frame observation for macOS live capture.

use screencapturekit::prelude::*;
use screencapturekit::{cv::CVPixelBufferLockFlags, FourCharCode};
use std::time::Instant;

use crate::{active_capture_preview::ActiveCapturePreview, CaptureReadiness};

/// Attach the observer before `start_capture`. ScreenCaptureKit invokes this
/// only for a delivered screen sample on the same stream that owns the
/// recording output; accepting stream startup is not readiness evidence.
pub(crate) fn attach_first_screen_frame_observer(
    stream: &mut SCStream,
    readiness: Option<CaptureReadiness>,
    preview: Option<(ActiveCapturePreview, u64, Instant)>,
) -> Result<(), &'static str> {
    stream
        .add_output_handler(
            move |sample, output_type| {
                if output_type == SCStreamOutputType::Screen {
                    if let Some(readiness) = readiness.as_ref() {
                        readiness.mark_first_screen_frame_delivered();
                    }
                    if let Some((preview, generation, started)) = preview.as_ref() {
                        observe_preview_sample(&sample, preview, *generation, *started);
                    }
                }
            },
            SCStreamOutputType::Screen,
        )
        .map(|_| ())
        .ok_or("ScreenCaptureKit rejected the screen-frame callback")
}

/// Read optional pixels from the recording's own delivered SCK sample. Any
/// preview readback failure stays in the mailbox; it cannot stop or alter the
/// recording output attached to this same stream.
fn observe_preview_sample(
    sample: &screencapturekit::cm::CMSampleBuffer,
    preview: &ActiveCapturePreview,
    generation: u64,
    started: Instant,
) {
    let sampled_at = Instant::now();
    if !preview.claim_sample(generation, sampled_at) {
        return;
    }
    let Some(buffer) = sample.image_buffer() else {
        return;
    };
    if buffer.pixel_format() != FourCharCode::from_bytes(*b"BGRA").as_u32() {
        preview.mark_readback_unavailable(generation);
        return;
    }
    let Ok(guard) = buffer.lock(CVPixelBufferLockFlags::READ_ONLY) else {
        preview.mark_readback_unavailable(generation);
        return;
    };
    preview.publish_bgra(
        generation,
        u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        u32::try_from(guard.width()).unwrap_or(0),
        u32::try_from(guard.height()).unwrap_or(0),
        guard.bytes_per_row(),
        guard.as_slice(),
    );
    preview.record_sample_cost(generation, sampled_at.elapsed());
}

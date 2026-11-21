//! First-delivered-screen-frame observation for macOS live capture.

use screencapturekit::prelude::*;
use screencapturekit::{cv::CVPixelBufferLockFlags, FourCharCode};
use std::time::Instant;

use crate::{active_capture_preview::ActiveCapturePreview, CaptureReadiness};

/// Attach the recorder and observations before starting the capture stream.
pub(crate) fn attach_screen_writer_observer(
    stream: &mut SCStream,
    readiness: Option<CaptureReadiness>,
    preview: Option<(ActiveCapturePreview, u64, Instant)>,
    window_clicks: Option<crate::macos_window_clicks::WindowClickCapture>,
    writer: crate::macos_screen_writer::ScreenWriter,
) -> Result<(), &'static str> {
    let delivery = writer.clone();
    stream
        .add_output_handler_with_queue(
            move |sample, output_type| {
                if output_type == SCStreamOutputType::Screen {
                    let picture = crate::macos_segment_frames::status_carries_picture(
                        crate::macos_frame_status::read_status(&sample),
                        sample.image_buffer().is_some(),
                    );
                    if picture {
                        writer.append(&sample);
                    }
                    if let Some(window_clicks) = window_clicks.as_ref() {
                        window_clicks.observe_frame(&sample);
                    }
                    if let Some(readiness) = readiness.as_ref() {
                        readiness.mark_first_screen_frame_delivered();
                    }
                    if let Some((preview, generation, started)) = preview.as_ref() {
                        observe_preview_sample(&sample, preview, *generation, *started);
                    }
                }
            },
            SCStreamOutputType::Screen,
            Some(delivery.delivery_queue()),
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

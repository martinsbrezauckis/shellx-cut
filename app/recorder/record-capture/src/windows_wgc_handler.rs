//! Private Windows Graphics Capture encoder and selected-window lifecycle adapter.
//!
//! This keeps the callback-owned source-close conclusion next to the native
//! callback that observes it. The outer Windows capture owner remains
//! responsible for selected-source admission and all terminal Stop boundaries.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use record_core::Result;
use windows_capture::{
    capture::{CaptureControl, Context, GraphicsCaptureApiHandler},
    frame::Frame,
    graphics_capture_api::InternalCaptureControl,
};

use crate::{
    region_geometry::NativePixelCrop, windows::cap_err, windows_wgc_run::WgcNativeControl,
    windows_wgc_timing::WgcTimingRecorder, CaptureReadiness, CaptureSourceLifecycle,
};

pub(crate) fn stop_with_held_sample(
    control: CaptureControl<Handler, <Handler as GraphicsCaptureApiHandler>::Error>,
) -> Result<()> {
    // Native stop posts WM_QUIT and joins without calling on_closed. Retain
    // the callback owner through that join and use this pre-join observation
    // as the final pixel boundary. A failed native Stop cannot append a hold.
    let callback = control.callback();
    let stop_at = Instant::now();
    control
        .stop()
        .map_err(|error| cap_err("finalize WGC checkpoint", error))?;
    let finished = callback
        .lock()
        .finish_after_control_stop(stop_at)
        .map_err(|error| cap_err("finalize WGC checkpoint", error));
    finished
}

pub(crate) struct LiveWgcControl {
    pub(crate) close: Option<Box<dyn FnOnce() -> Result<()> + Send>>,
    pub(crate) source_lifecycle: Option<CaptureSourceLifecycle>,
}

impl WgcNativeControl for LiveWgcControl {
    fn close(&mut self) -> Result<()> {
        if let Some(lifecycle) = self.source_lifecycle.as_ref() {
            lifecycle.expect_segment_close();
        }
        self.close
            .take()
            .ok_or_else(|| cap_err("finalize WGC checkpoint", "WGC control already closed"))?(
        )
    }
}

/// Encoder flags handed to the WGC callback (its `new` builds the encoder).
#[derive(Clone)]
pub(crate) struct EncFlags {
    pub(crate) window_clicks: Option<crate::windows_window_clicks::WindowClickCapture>,
    pub(crate) w: u32,
    pub(crate) h: u32,
    pub(crate) fps: u32,
    pub(crate) path: String,
    pub(crate) crop: Option<NativePixelCrop>,
    pub(crate) readiness: Option<CaptureReadiness>,
    pub(crate) source_lifecycle: Option<CaptureSourceLifecycle>,
    pub(crate) stop: Arc<AtomicBool>,
    pub(crate) timing: Option<WgcTimingRecorder>,
    pub(crate) timed_clock: Option<(
        Instant,
        crate::windows_wgc_clock_origin::QpcCaptureOrigin,
        i64,
    )>,
    pub(crate) active_preview: Option<(crate::active_capture_preview::ActiveCapturePreview, u64)>,
}

/// `windows-capture` handler: frames enter the encoder and a closed exact
/// window can conclude source loss before the outer wait loop exits.
pub(crate) struct Handler {
    window_clicks: Option<crate::windows_window_clicks::WindowClickCapture>,
    encoder: Option<crate::windows_wgc_encoder::WgcVideoEncoder>,
    crop: Option<NativePixelCrop>,
    crop_surface: Option<crate::windows_gpu_crop::GpuCropSurface>,
    readiness: Option<CaptureReadiness>,
    source_lifecycle: Option<CaptureSourceLifecycle>,
    stop: Arc<AtomicBool>,
    timing: Option<WgcTimingRecorder>,
    active_preview: Option<(crate::active_capture_preview::ActiveCapturePreview, u64)>,
    preview_started: Instant,
}

impl Handler {
    pub(crate) fn finish_after_control_stop(
        &mut self,
        stop_at: Instant,
    ) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // A source-close callback may already have consumed the encoder. It
        // must never be recast as a successful owned still-pixel Stop.
        if let Some(encoder) = self.encoder.take() {
            let finished = encoder.finish_after_control_stop(stop_at);
            if let Some(timing) = &self.timing {
                timing.encoder_finished(Instant::now(), finished.is_ok());
                if let Ok(Some(held_ms)) = finished.as_ref() {
                    timing.held_pixel_duration(*held_ms);
                }
            }
            finished?;
        }
        Ok(())
    }
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = EncFlags;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> std::result::Result<Self, Self::Error> {
        let flags = ctx.flags;
        let encoder = crate::windows_wgc_encoder::WgcVideoEncoder::new_with_clock(
            flags.w,
            flags.h,
            flags.fps,
            std::path::Path::new(&flags.path),
            flags.timed_clock,
        )?;
        Ok(Self {
            encoder: Some(encoder),
            window_clicks: flags.window_clicks,
            crop: flags.crop,
            crop_surface: None,
            readiness: flags.readiness,
            source_lifecycle: flags.source_lifecycle,
            stop: flags.stop,
            timing: flags.timing,
            active_preview: flags.active_preview,
            preview_started: Instant::now(),
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _control: InternalCaptureControl,
    ) -> std::result::Result<(), Self::Error> {
        if let Some(crop) = self.crop {
            crate::windows_gpu_crop::crop_frame_to_origin(frame, crop, &mut self.crop_surface)?;
        }
        if let Some(encoder) = self.encoder.as_mut() {
            let sample = self
                .timing
                .as_ref()
                .map(|_| {
                    frame
                        .timestamp()
                        .map(|timestamp| (timestamp.Duration, Instant::now()))
                })
                .transpose()?;
            let geometry = self
                .window_clicks
                .as_ref()
                .and_then(|clicks| clicks.geometry());
            let accepted = if let Some(clicks) = self.window_clicks.as_ref() {
                if let Some(fit) = encoder.send_window_frame(frame)? {
                    clicks.accepted_frame(geometry, frame.width(), frame.height(), fit);
                    true
                } else {
                    false
                }
            } else {
                encoder.send_frame(frame)?
            };
            if !accepted {
                return Ok(());
            }
            if let (Some(timing), Some((native_timestamp_100ns, callback_at))) =
                (&self.timing, sample)
            {
                timing.accepted_frame(native_timestamp_100ns, callback_at);
            }
            // WGC delivered a native frame and the encoder accepted it. A
            // callback/start success alone is not enough for readiness.
            if let Some(readiness) = self.readiness.as_ref() {
                readiness.mark_first_screen_frame_delivered();
            }
            if let Some((preview, generation)) = self.active_preview.as_ref() {
                preview.observe_wgc_frame(*generation, frame, self.preview_started);
            }
        }
        Ok(())
    }

    fn on_closed(&mut self) -> std::result::Result<(), Self::Error> {
        if let Some(clicks) = self.window_clicks.as_ref() {
            clicks.source_closed();
        }
        if let Some(timing) = &self.timing {
            timing.close_callback(Instant::now());
        }
        // Callback delivery means WGC has ended, even when it was not an
        // armed exact-window closure. Revoke ready admission before waking the
        // outer owner so status cannot briefly report an ended stream as ready.
        if let Some(readiness) = self.readiness.as_ref() {
            readiness.mark_terminal();
        }
        // WGC invokes this when its GraphicsCaptureItem closes. It becomes
        // source loss only for an armed exact-window item and only before a
        // Cut-owned close; monitor and generic capture termination stay out.
        let _source_lost = self.source_lifecycle.as_ref().is_some_and(|lifecycle| {
            lifecycle.selected_source_closed("The exact selected Windows window closed.")
        });
        // Every native close ends the shared wait.  The lifecycle transition
        // above remains the sole authority for whether this was exact-window
        // source loss rather than an ordinary terminal capture event.
        self.stop.store(true, Ordering::Release);
        if let Some(encoder) = self.encoder.take() {
            let finished = encoder.finish();
            if let Some(timing) = &self.timing {
                timing.encoder_finished(Instant::now(), finished.is_ok());
            }
            finished?;
        }
        Ok(())
    }
}

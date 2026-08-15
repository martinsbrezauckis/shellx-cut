//! Minimal Windows Graphics Capture delivery probe used only by the recorder doctor.
//!
//! The primary monitor is captured only until its first frame callback. The callback
//! drops the GPU frame, requests shutdown, and the control is always stopped/joined.

use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use windows_capture::{
    capture::{Context, GraphicsCaptureApiHandler},
    frame::Frame,
    graphics_capture_api::InternalCaptureControl,
    monitor::Monitor,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    },
};

use crate::doctor_probe::{ScreenProbe, SCREEN_PROBE_TIMEOUT};

struct ProbeHandler {
    delivered: Arc<AtomicBool>,
}

impl GraphicsCaptureApiHandler for ProbeHandler {
    type Flags = Arc<AtomicBool>;
    type Error = Infallible;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            delivered: ctx.flags,
        })
    }

    fn on_frame_arrived(
        &mut self,
        _frame: &mut Frame,
        control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        self.delivered.store(true, Ordering::Release);
        control.stop();
        Ok(())
    }
}

/// Run one first-frame WGC probe without writing a video file or showing a picker.
pub(crate) fn screen_probe() -> ScreenProbe {
    if let Err(error) = crate::windows_runtime::pin_process_mta() {
        return ScreenProbe::Failed(format!("initialize Windows capture runtime: {error}"));
    }
    let monitor = match Monitor::primary() {
        Ok(monitor) => monitor,
        Err(error) => return ScreenProbe::Failed(format!("find primary monitor: {error}")),
    };
    let delivered = Arc::new(AtomicBool::new(false));
    let settings = Settings::new(
        monitor,
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Rgba8,
        delivered.clone(),
    );
    let control = match ProbeHandler::start_free_threaded(settings) {
        Ok(control) => control,
        Err(error) => {
            let detail = format!("start WGC: {error}");
            if detail.to_ascii_lowercase().contains("access denied") {
                return ScreenProbe::PermissionDenied(detail);
            }
            return ScreenProbe::Failed(detail);
        }
    };
    let deadline = Instant::now() + SCREEN_PROBE_TIMEOUT;
    while !delivered.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep(std::time::Duration::from_millis(10));
    }
    if let Err(error) = control.stop() {
        return ScreenProbe::CleanupFailed(format!("stop WGC: {error}"));
    }
    if delivered.load(Ordering::Acquire) {
        ScreenProbe::FrameDelivered
    } else {
        ScreenProbe::TimedOut
    }
}

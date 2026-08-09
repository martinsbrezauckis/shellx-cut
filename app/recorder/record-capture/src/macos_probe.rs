//! Minimal ScreenCaptureKit delivery probe used only by the recorder doctor.
//!
//! This starts no recording output and retains no sample buffer. It first uses
//! `CGPreflightScreenCaptureAccess`, which never requests permission, then waits
//! for one 2×2 video callback and stops the stream before returning.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use screencapturekit::prelude::*;

use crate::doctor_probe::{ScreenProbe, SCREEN_PROBE_TIMEOUT};

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
}

/// Run one non-persisting ScreenCaptureKit frame probe without requesting TCC.
pub(crate) fn screen_probe() -> ScreenProbe {
    // SAFETY: CoreGraphics owns the permission state and takes no Rust pointers.
    if !unsafe { CGPreflightScreenCaptureAccess() } {
        return ScreenProbe::PermissionDenied(
            "Screen Recording is not already granted; doctor will not trigger a new permission prompt"
                .into(),
        );
    }

    crate::macos::sck_init_cg();
    let content = match SCShareableContent::get() {
        Ok(content) => content,
        Err(error) => return ScreenProbe::Failed(format!("list displays: {error:?}")),
    };
    let displays = content.displays();
    let Some(display) = displays.first() else {
        return ScreenProbe::Failed("no display is available to probe".into());
    };

    let filter = SCContentFilter::create()
        .with_display(display)
        .with_excluding_windows(&[])
        .build();
    // A tiny scaled callback verifies delivery while never writing or retaining pixels.
    let config = SCStreamConfiguration::new()
        .with_width(2)
        .with_height(2)
        .with_queue_depth(1)
        .with_fps(30);
    let delivered = Arc::new(AtomicBool::new(false));
    let callback_delivered = delivered.clone();
    let mut stream = SCStream::new(&filter, &config);
    if stream
        .add_output_handler(
            move |_sample, output_type| {
                if output_type == SCStreamOutputType::Screen {
                    callback_delivered.store(true, Ordering::Release);
                }
            },
            SCStreamOutputType::Screen,
        )
        .is_none()
    {
        return ScreenProbe::Failed("ScreenCaptureKit rejected the video callback".into());
    }
    if let Err(error) = stream.start_capture() {
        return ScreenProbe::Failed(format!("start ScreenCaptureKit: {error:?}"));
    }

    let deadline = Instant::now() + SCREEN_PROBE_TIMEOUT;
    while !delivered.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep(std::time::Duration::from_millis(10));
    }
    if let Err(error) = stream.stop_capture() {
        return ScreenProbe::CleanupFailed(format!("stop ScreenCaptureKit: {error:?}"));
    }
    if delivered.load(Ordering::Acquire) {
        ScreenProbe::FrameDelivered
    } else {
        ScreenProbe::TimedOut
    }
}

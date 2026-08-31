//! First-delivered-screen-frame observation for macOS live capture.

use screencapturekit::prelude::*;

use crate::CaptureReadiness;

/// Attach the observer before `start_capture`. ScreenCaptureKit invokes this
/// only for a delivered screen sample on the same stream that owns the
/// recording output; accepting stream startup is not readiness evidence.
pub(crate) fn attach_first_screen_frame_observer(
    stream: &mut SCStream,
    readiness: Option<CaptureReadiness>,
) -> Result<(), &'static str> {
    stream
        .add_output_handler(
            move |_sample, output_type| {
                if output_type == SCStreamOutputType::Screen {
                    if let Some(readiness) = readiness.as_ref() {
                        readiness.mark_first_screen_frame_delivered();
                    }
                }
            },
            SCStreamOutputType::Screen,
        )
        .map(|_| ())
        .ok_or("ScreenCaptureKit rejected the screen-frame callback")
}

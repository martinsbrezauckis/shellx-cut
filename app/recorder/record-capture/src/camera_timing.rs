//! Exact camera-sample timing carried from a native adapter to the session seal.
//!
//! A frame start is not a media-duration boundary. Adapters must report the
//! actual interval covered by every delivered sample so the final artifact can
//! describe the end of the final sample without guessing from its start.

use std::time::Instant;

/// One delivered camera sample measured on the screen owner's monotonic clock.
///
/// Values remain private because native adapters must not expose host timing
/// details through recorder arguments, responses, or device enumeration.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CameraFrameObservation {
    pub(crate) started_at: Instant,
    pub(crate) ended_at: Instant,
}

impl CameraFrameObservation {
    #[cfg_attr(
        not(any(
            all(windows, feature = "capture-windows"),
            all(target_os = "macos", feature = "capture-macos"),
            test
        )),
        allow(
            dead_code,
            reason = "no native camera adapter is compiled on this host"
        )
    )]
    pub(crate) fn new(started_at: Instant, ended_at: Instant) -> Self {
        Self {
            started_at,
            ended_at,
        }
    }
}

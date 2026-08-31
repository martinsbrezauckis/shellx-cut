use std::time::{Duration, Instant};

use crate::camera_session::{CameraSession, CameraSessionBackend};
use crate::CameraFrameObservation;

pub(crate) fn observe<B: CameraSessionBackend>(
    session: &mut CameraSession<B>,
    origin: Instant,
    start_ms: u64,
    end_ms: u64,
) {
    session
        .observe_frame(CameraFrameObservation::new(
            origin + Duration::from_millis(start_ms),
            origin + Duration::from_millis(end_ms),
        ))
        .unwrap();
}

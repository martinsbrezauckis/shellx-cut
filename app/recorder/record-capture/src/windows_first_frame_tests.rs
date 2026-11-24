use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::WindowsFirstFrameAnchor;
use crate::{
    windows_wgc_clock_origin::{FirstFramePlacement, QpcCaptureOrigin},
    CaptureClock,
};

#[test]
fn capture_clock_waits_for_first_wgc_frame_and_anchors_segment_zero_to_zero() {
    let clock = CaptureClock::new();
    let anchor = WindowsFirstFrameAnchor::new(Some(clock.clone()));
    let _pre_wgc_start = Instant::now();

    assert!(
        clock.started_at().is_none(),
        "CaptureClock must stay closed until the first WGC picture arrives"
    );
    assert!(anchor.input_origin_cell().get().is_none());

    // First frame arrives 559 ms later.
    let callback_at = Instant::now();
    let first_native_hns = 100_000_000 + 559 * 10_000;
    let callback_qpc = QpcCaptureOrigin::from_sample(
        10_000_000,
        first_native_hns,
        first_native_hns,
        Duration::ZERO,
    )
    .ok();
    let (origin_instant, origin) = anchor
        .admit_first_frame_with_sample(first_native_hns, callback_at, callback_qpc)
        .unwrap();

    assert_eq!(anchor.qpc_origin(), Some((origin_instant, origin)));
    assert_eq!(clock.started_at(), Some(origin_instant));
    assert_eq!(
        anchor.input_origin_cell().get().copied(),
        Some(origin_instant)
    );

    let placement = origin.place_first_frame(first_native_hns, 0).unwrap();
    let start_ms = match placement {
        FirstFramePlacement::AtOrAfterStart { start_ms } => start_ms,
        FirstFramePlacement::BeforeStart { lead_hns } => {
            panic!("first frame unexpectedly preceded segment start by {lead_hns} hns");
        }
    };
    assert_eq!(
        start_ms, 0,
        "first captured picture must begin at start_ms=0 so stitch does not clone startup frames"
    );
}

#[test]
fn stop_before_first_wgc_frame_unblocks_wait_without_opening_capture_clock() {
    let clock = CaptureClock::new();
    let anchor = WindowsFirstFrameAnchor::new(Some(clock.clone()));
    let stop = Arc::new(AtomicBool::new(false));
    let worker_anchor = anchor.clone();
    let worker_stop = stop.clone();
    let handle = std::thread::spawn(move || worker_anchor.wait_started(&worker_stop, || Ok(())));
    std::thread::sleep(Duration::from_millis(15));
    stop.store(true, Ordering::Release);
    let waited = handle.join().unwrap().unwrap();
    assert_eq!(waited, None);
    assert!(clock.started_at().is_none());
}

#[test]
fn slightly_earlier_compositor_timestamp_shifts_origin_back_cleanly() {
    let clock = CaptureClock::new();
    let anchor = WindowsFirstFrameAnchor::new(Some(clock.clone()));
    let callback_at = Instant::now();
    let callback_qpc =
        QpcCaptureOrigin::from_sample(10_000_000, 100_040_000, 100_040_000, Duration::ZERO).ok();
    let first_native_hns = 100_000_000; // 4 ms before callback arrival
    let (origin_instant, origin) = anchor
        .admit_first_frame_with_sample(first_native_hns, callback_at, callback_qpc)
        .unwrap();
    assert_eq!(
        callback_at.duration_since(origin_instant),
        Duration::from_millis(4)
    );
    assert_eq!(
        origin.place_first_frame(first_native_hns, 0).unwrap(),
        FirstFramePlacement::AtOrAfterStart { start_ms: 0 }
    );
}

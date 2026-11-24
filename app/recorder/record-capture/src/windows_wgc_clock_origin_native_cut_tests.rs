use super::{FirstFramePlacement, QpcCaptureOrigin};
use std::time::{Duration, Instant};

#[test]
fn native_cut_uses_one_capture_clock_for_journal_and_held_pixels() {
    let start = Instant::now();
    let origin =
        QpcCaptureOrigin::from_sample(10_000_000, 50_000_000, 50_000_000, Duration::ZERO).unwrap();
    let (end_ms, end_at) = origin.native_cut(start, 199_999_999).unwrap();
    assert_eq!(end_ms, 15_000);
    assert_eq!(
        end_at.duration_since(start),
        Duration::from_nanos(14_999_999_900)
    );
    assert!(origin.native_cut(start, 49_999_999).is_err());
}

#[test]
fn later_cached_segment_ends_at_its_owned_reserved_boundary() {
    let start = Instant::now();
    let origin =
        QpcCaptureOrigin::from_sample(10_000_000, 50_000_000, 50_000_000, Duration::ZERO).unwrap();
    let second_boundary = origin.reserved_boundary_hns(15_000).unwrap();
    assert_eq!(
        origin
            .place_first_frame(second_boundary - 50_000, 15_000)
            .unwrap(),
        FirstFramePlacement::BeforeStart { lead_hns: 50_000 }
    );
    assert_eq!(
        origin
            .stop_media_timestamp(
                start,
                start + Duration::from_millis(18_000),
                second_boundary
            )
            .unwrap(),
        30_000_000
    );
}

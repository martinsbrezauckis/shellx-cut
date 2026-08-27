use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

pub(super) fn record_peak(peak: &AtomicU32, sample: i16) {
    peak.fetch_max(i32::from(sample).unsigned_abs(), Ordering::Relaxed);
}

pub(super) fn mark_first_packet(
    ready: &AtomicBool,
    first_packet_offset_ms: &AtomicU64,
    origin: Instant,
) {
    ready.store(true, Ordering::Relaxed);
    let elapsed_ms = u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX);
    let _ = first_packet_offset_ms.compare_exchange(
        u64::MAX,
        elapsed_ms,
        Ordering::Relaxed,
        Ordering::Relaxed,
    );
}

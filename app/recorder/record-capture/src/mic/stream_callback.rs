use super::device::MicRecordingGate;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
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

/// Report the live native callback immediately, but withhold durable samples
/// until a standalone owner has armed its recording clock.
pub(super) fn capture_origin(
    ready: &AtomicBool,
    recording_gate: Option<&Arc<MicRecordingGate>>,
    fallback: Instant,
) -> Option<Instant> {
    ready.store(true, Ordering::Relaxed);
    recording_gate.map_or(Some(fallback), |gate| gate.origin())
}

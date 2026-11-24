use super::{SystemAudioTimingTracker, TimedAudioPacket};
use std::time::Duration;

#[test]
fn first_packet_offsets_cover_zero_small_and_multi_second_delays() {
    for (delay, expected) in [
        (Duration::ZERO, Some(0)),
        (Duration::from_millis(37), Some(37)),
        (Duration::from_millis(2_350), Some(2_350)),
    ] {
        let mut tracker = SystemAudioTimingTracker::default();
        tracker.record_packet(960, delay).unwrap();
        assert_eq!(tracker.first_packet_offset_ms(), expected);
    }
}

#[test]
fn delayed_packets_persist_only_actual_samples_without_padding() {
    let mut tracker = SystemAudioTimingTracker::default();
    tracker
        .record_packet(960, Duration::from_millis(2_000))
        .unwrap();
    tracker
        .record_packet(480, Duration::from_millis(2_010))
        .unwrap();

    assert_eq!(tracker.first_packet_offset_ms(), Some(2_000));
    assert_eq!(tracker.written_samples(), 1_440);
    // A later clock observation is not a packet and cannot add samples.
    assert_eq!(tracker.written_samples(), 1_440);
}

#[test]
fn short_sounds_with_wasapi_qpc_or_discontinuity_gaps_stay_in_sync_without_drift() {
    const RATE: u32 = 48_000;
    const CH: u16 = 2;
    const PKT_FRAMES: u64 = 480; // 10 ms
    const PKT_SAMPLES: usize = (PKT_FRAMES * CH as u64) as usize;
    const HNS_PER_PKT: u64 = 100_000; // 10 ms
    const T0_HNS: u64 = 1_000_000_000;

    let mut tracker = SystemAudioTimingTracker::default();
    let mut qpc = T0_HNS;
    let mut elapsed = Duration::from_millis(34);
    let mut total_gap_samples = 0usize;

    // Simulate 20 x 2-second bursts (40 s), where each burst loses 20 ms of
    // loopback packets (2 * 10 ms) at the SoundPlayer open/close transition.
    for burst in 0..20_u64 {
        for pkt_in_burst in 0..198_u64 {
            let discontinuity = burst > 0 && pkt_in_burst == 0 && (burst % 2 == 1);
            let gap = tracker
                .record_timed_packet(TimedAudioPacket {
                    samples: PKT_SAMPLES,
                    qpc_position_100ns: qpc,
                    discontinuity,
                    is_silent: !discontinuity && pkt_in_burst >= 10,
                    elapsed_at_capture: elapsed,
                    packet_start_elapsed: Some(elapsed.saturating_sub(Duration::from_millis(10))),
                    sample_rate: RATE,
                    channels: CH,
                })
                .unwrap();
            total_gap_samples += gap;
            qpc += HNS_PER_PKT;
            elapsed += Duration::from_millis(10);
        }
        qpc += 2 * HNS_PER_PKT;
        elapsed += Duration::from_millis(20);
    }

    assert_eq!(tracker.first_packet_offset_ms(), Some(24));
    assert_eq!(total_gap_samples, 19 * 2 * PKT_SAMPLES);
    assert!(
        tracker.written_samples() >= 3_837_120 && tracker.written_samples() <= 3_840_000,
        "short sound bursts drifted: written_samples={} total_gap_samples={total_gap_samples}",
        tracker.written_samples()
    );
}

#[test]
fn silent_no_packet_gap_after_packet_zero_is_filled_when_next_packet_arrives() {
    let mut tracker = SystemAudioTimingTracker::default();
    tracker
        .record_timed_packet(TimedAudioPacket {
            samples: 960,
            qpc_position_100ns: 500_000_000,
            discontinuity: false,
            is_silent: true,
            elapsed_at_capture: Duration::from_millis(30),
            packet_start_elapsed: Some(Duration::from_millis(20)),
            sample_rate: 48_000,
            channels: 2,
        })
        .unwrap();
    // 1.89 s of silence delivered zero packets; click arrives at 1920 ms wall time
    let gap = tracker
        .record_timed_packet(TimedAudioPacket {
            samples: 960,
            qpc_position_100ns: 519_000_000,
            discontinuity: false,
            is_silent: false,
            elapsed_at_capture: Duration::from_millis(1_930),
            packet_start_elapsed: Some(Duration::from_millis(1_920)),
            sample_rate: 48_000,
            channels: 2,
        })
        .unwrap();
    assert_eq!(gap, 181_440);
    assert_eq!(tracker.written_samples(), 183_360);
}

#[test]
fn continuous_stream_with_normal_jitter_and_mild_clock_skew_inserts_zero_silence() {
    let mut tracker = SystemAudioTimingTracker::default();
    for i in 0..4_000_u64 {
        let jitter_ms = (i % 4) as i64 - 1;
        let wall_ms = (i * 10 + 25).saturating_add_signed(jitter_ms);
        let qpc = 1_000_000_000 + i * 100_010;
        let gap = tracker
            .record_timed_packet(TimedAudioPacket {
                samples: 960,
                qpc_position_100ns: qpc,
                discontinuity: false,
                is_silent: false,
                elapsed_at_capture: Duration::from_millis(wall_ms),
                packet_start_elapsed: Some(Duration::from_millis(wall_ms.saturating_sub(10))),
                sample_rate: 48_000,
                channels: 2,
            })
            .unwrap();
        assert_eq!(
            gap, 0,
            "continuous audio must not insert mid-sound silence at packet {i}"
        );
    }
    assert_eq!(tracker.written_samples(), 4_000 * 960);
}

#[test]
fn filled_duration_never_exceeds_elapsed_capture_time() {
    let mut tracker = SystemAudioTimingTracker::default();
    tracker
        .record_timed_packet(TimedAudioPacket {
            samples: 960,
            qpc_position_100ns: 100_000_000,
            discontinuity: false,
            is_silent: true,
            elapsed_at_capture: Duration::from_millis(20),
            packet_start_elapsed: Some(Duration::from_millis(10)),
            sample_rate: 48_000,
            channels: 2,
        })
        .unwrap();
    let gap = tracker
        .record_timed_packet(TimedAudioPacket {
            samples: 960,
            qpc_position_100ns: 105_000_000,
            discontinuity: true,
            is_silent: false,
            elapsed_at_capture: Duration::from_millis(200),
            packet_start_elapsed: Some(Duration::from_millis(190)),
            sample_rate: 48_000,
            channels: 2,
        })
        .unwrap();
    let total_ms = tracker.written_samples() / (48 * 2);
    assert!(
        total_ms <= 200,
        "WAV duration {total_ms} ms exceeded elapsed 200 ms"
    );
    assert!(gap > 0);
}

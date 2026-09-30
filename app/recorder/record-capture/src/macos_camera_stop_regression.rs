//! Retained native packet regression for the camera Stop seal.

use super::*;

#[test]
fn retained_mac_camera_reordered_stop_tail_stays_rejected() {
    // Unmodified packet probes from the bd4 native failed Camera Stop.
    // Do not infer a larger native endpoint from the codec's buffered tail.
    let raw = include_bytes!("fixtures/mac_camera_stop_reordered/raw.csv");
    let edited = include_bytes!("fixtures/mac_camera_stop_reordered/edited.csv");
    let timing = NativeMovieTiming {
        start_pts_ns: 390_347_033_510_000,
        last_pts_ns: 390_352_483_930_000,
        last_duration_ns: 15_520_000,
        last_cadence_ns: 16_940_001,
        callback_count: 380,
        stop_pts_ns: 390_352_499_870_000,
        stop_cadence_ns: 15_940_000,
        observed_last_pts_ns: 390_352_549_869_999,
    };
    let verify = |edited: &[u8], duration, frames, native| {
        verify_edit_list(
            raw.as_slice(),
            edited,
            648399,
            duration,
            TimeBase {
                num: 1,
                den: 100_000,
            },
            375,
            frames,
            native,
        )
    };
    let failure = verify(edited.as_slice(), 548301, 318, timing).unwrap_err();
    assert!(failure.cause.contains("callback disagrees"));
    assert!(failure.cause.contains("\"callback_delta_ns\":32580000"));
    assert!(failure.cause.contains("\"endpoint_limit_ns\":10000"));

    // A counterfactual edit view that discards both post-Stop frames
    // satisfies the unchanged seal. All original raw samples remain
    // accounted for; only their presentation eligibility changes.
    let bounded = std::str::from_utf8(edited)
        .unwrap()
        .replace("546636,1834,2606796,___", "546636,1834,2606796,_D_")
        .replace("548300,1664,2644535,___", "548300,1664,2644535,_D_");
    let result = verify(bounded.as_bytes(), 545043, 316, timing).unwrap();
    assert_eq!(result.start_pts_ns, timing.start_pts_ns);
    assert_eq!(result.duration_ms, 5450);

    // Moving the callback endpoint to the movie's greatest PTS cannot
    // repair the evidence: it would cross the excluded native sample.
    let mut fabricated = timing;
    fabricated.last_pts_ns = timing.start_pts_ns + 5_483_000_000;
    assert!(verify(edited.as_slice(), 548301, 318, fabricated)
        .unwrap_err()
        .cause
        .contains("Stop boundary does not follow"));
    let missing = bounded.replace("545042,1664,2625629,___\n", "");
    assert!(verify(missing.as_bytes(), 545043, 315, timing).is_err());
}

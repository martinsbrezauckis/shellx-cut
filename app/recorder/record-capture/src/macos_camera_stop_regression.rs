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

#[test]
fn retained_no_reordering_movie_proves_invoking_stop_sample_and_source_durations() {
    let raw = include_bytes!("fixtures/mac_camera_stop_no_reordering/raw.csv");
    let edited = include_bytes!("fixtures/mac_camera_stop_no_reordering/edited.csv");
    let timing = no_reordering_native_timing();
    let verify = |raw: &[u8], edited: &[u8], duration, frames| {
        verify_edit_list(
            raw,
            edited,
            1904950,
            duration,
            TimeBase {
                num: 1,
                den: 300_000,
            },
            381,
            frames,
            timing,
        )
    };
    let result = verify(raw.as_slice(), edited.as_slice(), 1604451, 321).unwrap();
    assert_eq!(result.duration_ms, 5348);
    assert_eq!(result.start_pts_ns, timing.start_pts_ns);

    // The final visible packet matches the invoking callback, not a guessed
    // greatest timestamp. The next encoded sample must stay discarded.
    let edited_text = std::str::from_utf8(edited).unwrap();
    let later = edited_text.replace("1604469,5004,1253208,_D_", "1604469,5004,1253208,___");
    assert!(verify(raw.as_slice(), later.as_bytes(), 1604470, 322)
        .unwrap_err()
        .cause
        .contains("callback disagrees"));
    let missing = edited_text.replace("1599459,4791,1251647,___\n", "");
    assert!(verify(raw.as_slice(), missing.as_bytes(), 1604451, 320)
        .unwrap_err()
        .cause
        .contains("inside the edit interval was skipped"));
    let wrong_offset = edited_text.replace("4245,4992,196608,___", "4246,4992,196608,___");
    assert!(
        verify(raw.as_slice(), wrong_offset.as_bytes(), 1604451, 321)
            .unwrap_err()
            .cause
            .contains("one media-time offset")
    );
    let wrong_identity = edited_text.replace("0,4777,166495,K__", "0,4777,166495,___");
    assert!(
        verify(raw.as_slice(), wrong_identity.as_bytes(), 1604451, 321)
            .unwrap_err()
            .cause
            .contains("identity differs")
    );
    let corrupted = edited_text.replace("0,4777,166495,K__", "0,4777,166495,K_C");
    assert!(verify(raw.as_slice(), corrupted.as_bytes(), 1604451, 321)
        .unwrap_err()
        .cause
        .contains("identity differs"));
    let raw_text = std::str::from_utf8(raw).unwrap();
    let too_short = raw_text.replace("1894930,5010,1251647,___", "1894930,1,1251647,___");
    assert!(
        verify(too_short.as_bytes(), edited.as_slice(), 1604451, 321)
            .unwrap_err()
            .cause
            .contains("presentation bounds")
    );
    let too_long = raw_text.replace("1894930,5010,1251647,___", "1894930,500000,1251647,___");
    assert!(verify(too_long.as_bytes(), edited.as_slice(), 1604451, 321)
        .unwrap_err()
        .cause
        .contains("exceeds native sample cadence"));
}

#[test]
fn frozen_stop_endpoint_rejects_invalid_clocks_and_more_than_one_track_tick() {
    let timing = no_reordering_native_timing();
    let scale = 300_000;
    let exact = 1_599_459_u128 * 1_000_000_000;
    let limit = 1_000_000_000 + u128::from(scale - 1);
    assert_eq!(
        timing.stop_endpoint_elapsed(exact, scale, limit).unwrap(),
        Some(timing.stop_pts_ns - timing.start_pts_ns)
    );
    assert_eq!(
        timing
            .stop_endpoint_elapsed(exact + 2_000_000_000, scale, limit)
            .unwrap(),
        None
    );
    let mut inconsistent = timing;
    inconsistent.stop_cadence_ns += 1;
    assert!(inconsistent
        .stop_endpoint_elapsed(exact, scale, limit)
        .is_err());
    let mut invalid_origin = timing;
    invalid_origin.start_pts_ns = timing.stop_pts_ns + 1;
    assert!(invalid_origin
        .stop_endpoint_elapsed(exact, scale, limit)
        .is_err());
    let mut empty_boundary = timing;
    empty_boundary.stop_pts_ns = timing.last_pts_ns;
    assert!(empty_boundary
        .stop_endpoint_elapsed(exact, scale, limit)
        .is_err());
}

fn no_reordering_native_timing() -> NativeMovieTiming {
    NativeMovieTiming {
        start_pts_ns: 393_915_350_620_000,
        last_pts_ns: 393_920_665_510_000,
        last_duration_ns: 16_610_000,
        last_cadence_ns: 16_610_000,
        callback_count: 383,
        stop_pts_ns: 393_920_682_150_000,
        stop_cadence_ns: 16_640_000,
        observed_last_pts_ns: 393_920_682_150_000,
    }
}

#[test]
#[ignore = "requires retained native movie via SHELLX_CAMERA_STOP_NO_REORDERING_FIXTURE"]
fn retained_no_reordering_movie_runs_owned_full_probe_pipeline() {
    let fixture = std::env::var("SHELLX_CAMERA_STOP_NO_REORDERING_FIXTURE").unwrap();
    let timing = no_reordering_native_timing();
    let result = verify_movie_edit_list(
        "ffprobe",
        Path::new(&fixture),
        timing,
        "1/300000",
        1604451,
        "5.348170",
        321,
        Duration::from_secs(30),
    )
    .unwrap();
    assert_eq!(result.duration_ms, 5348);
    assert_eq!(result.start_pts_ns, timing.start_pts_ns);
}

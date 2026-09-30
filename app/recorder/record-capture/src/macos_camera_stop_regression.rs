//! Existing native movies are unqualified without an independently measured
//! writer-completion clock. Never retrofit a clock from their packet endpoint.

use super::*;

#[test]
fn retained_movies_without_writer_completion_clocks_remain_unqualified() {
    let views = [
        (
            include_bytes!("fixtures/mac_camera_stop_reordered/raw.csv").as_slice(),
            include_bytes!("fixtures/mac_camera_stop_reordered/edited.csv").as_slice(),
            648399,
            548301,
            100_000,
            375,
            318,
            NativeMovieTiming {
                start_pts_ns: 390_347_033_510_000,
                last_pts_ns: 390_352_483_930_000,
                last_duration_ns: 15_520_000,
                last_cadence_ns: 16_940_001,
                callback_count: 380,
                stop_pts_ns: 390_352_499_870_000,
                stop_cadence_ns: 15_940_000,
                observed_last_pts_ns: 390_352_549_869_999,
                ..NativeMovieTiming::default()
            },
        ),
        (
            include_bytes!("fixtures/mac_camera_stop_no_reordering/raw.csv").as_slice(),
            include_bytes!("fixtures/mac_camera_stop_no_reordering/edited.csv").as_slice(),
            1904950,
            1604451,
            300_000,
            381,
            321,
            no_reordering_native_timing(),
        ),
    ];
    for (raw, edited, raw_duration, presented_duration, den, raw_frames, frames, native) in views {
        let failure = verify_edit_list(
            raw,
            edited,
            raw_duration,
            presented_duration,
            TimeBase { num: 1, den },
            raw_frames,
            frames,
            native,
        )
        .unwrap_err();
        assert!(failure.cause.contains("writer-completion clock is missing"));
    }
}

#[test]
fn queued_frames_are_bounded_by_independent_completion_not_stop_sample_count() {
    // Synthetic independent event observations: Stop requested at 30ms,
    // consumed by the 40ms sample, writer completed at 70ms. The 60ms queued
    // frame is visible; its actual 20ms display interval extends beyond finish.
    let raw = b"0,20,50,K__\n1000,20,100,K__\n1020,20,200,___\n1040,20,300,___\n1060,20,400,___\n1080,20,500,___\n";
    let edited = "0,20,100,K__\n20,20,200,___\n40,20,300,___\n60,20,400,___\n80,20,500,_D_\n";
    let native = NativeMovieTiming {
        start_pts_ns: 100_000_000_000,
        last_pts_ns: 100_020_000_000,
        last_duration_ns: 20_000_000,
        last_cadence_ns: 20_000_000,
        stop_pts_ns: 100_040_000_000,
        stop_cadence_ns: 20_000_000,
        stop_request_clock_ns: 100_030_000_000,
        finish_clock_ns: 100_070_000_000,
        ..NativeMovieTiming::default()
    };
    let verify = |raw: &[u8], edited: &str, duration, frames, native| {
        verify_edit_list(
            raw,
            edited.as_bytes(),
            1100,
            duration,
            TimeBase { num: 1, den: 1000 },
            6,
            frames,
            native,
        )
    };
    let result = verify(raw.as_slice(), edited, 80, 4, native).unwrap();
    assert_eq!(result.duration_ms, 80);
    assert_eq!(result.start_pts_ns, native.start_pts_ns);
    let future = edited.replace("80,20,500,_D_", "80,20,500,___");
    assert!(verify(raw.as_slice(), &future, 81, 5, native)
        .unwrap_err()
        .cause
        .contains("after writer completion"));
    let lost = "0,20,100,K__\n20,20,200,___\n40,20,300,_D_\n60,20,400,_D_\n80,20,500,_D_\n";
    let lost_native = NativeMovieTiming {
        last_pts_ns: 100_040_000_000,
        stop_pts_ns: 100_060_000_000,
        stop_request_clock_ns: 100_050_000_000,
        ..native
    };
    assert!(verify(raw.as_slice(), lost, 40, 2, lost_native)
        .unwrap_err()
        .cause
        .contains("lost an included sample"));
    for invalid in [
        NativeMovieTiming {
            finish_clock_ns: 0,
            ..native
        },
        NativeMovieTiming {
            stop_request_clock_ns: 0,
            ..native
        },
        NativeMovieTiming {
            finish_clock_ns: 100_010_000_000,
            ..native
        },
        NativeMovieTiming {
            stop_request_clock_ns: 100_080_000_000,
            ..native
        },
        NativeMovieTiming {
            stop_request_clock_ns: 100_010_000_000,
            ..native
        },
        NativeMovieTiming {
            stop_cadence_ns: 19_000_000,
            ..native
        },
    ] {
        assert!(verify(raw.as_slice(), edited, 80, 4, invalid).is_err());
    }
    assert!(verify(raw.as_slice(), edited, 80, 3, native)
        .unwrap_err()
        .cause
        .contains("presented decode"));
    let missing = edited.replace("20,20,200,___\n", "");
    assert!(verify(raw.as_slice(), &missing, 80, 3, native)
        .unwrap_err()
        .cause
        .contains("inside the edit interval was skipped"));
    let wrong_offset = edited.replace("40,20,300,___", "41,20,300,___");
    assert!(verify(raw.as_slice(), &wrong_offset, 80, 4, native)
        .unwrap_err()
        .cause
        .contains("one media-time offset"));
    let corrupted = edited.replace("40,20,300,___", "40,20,300,__C");
    assert!(verify(raw.as_slice(), &corrupted, 80, 4, native)
        .unwrap_err()
        .cause
        .contains("identity differs"));
    let long_source = std::str::from_utf8(raw)
        .unwrap()
        .replace("1060,20,400,___", "1060,1000,400,___");
    assert!(verify(long_source.as_bytes(), edited, 80, 4, native)
        .unwrap_err()
        .cause
        .contains("exceeds native sample cadence"));
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
        ..NativeMovieTiming::default()
    }
}

#[test]
#[ignore = "requires retained native movie via SHELLX_CAMERA_STOP_NO_REORDERING_FIXTURE"]
fn retained_movie_full_probe_does_not_invent_missing_completion_clock() {
    let fixture = std::env::var("SHELLX_CAMERA_STOP_NO_REORDERING_FIXTURE").unwrap();
    let failure = verify_movie_edit_list(
        "ffprobe",
        Path::new(&fixture),
        no_reordering_native_timing(),
        "1/300000",
        1604451,
        "5.348170",
        321,
        Duration::from_secs(30),
    )
    .unwrap_err();
    assert!(failure.cause.contains("writer-completion clock is missing"));
}

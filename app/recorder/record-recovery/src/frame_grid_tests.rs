use record_core::FrameRate;

use crate::{quantize_run_aware_stitch, FrameGridError, RunAwareStitchPlan, RunAwareStitchSpan};

fn source(offset_ms: u64, duration_ms: u64) -> RunAwareStitchSpan {
    RunAwareStitchSpan::Source {
        run_sequence: 0,
        checkpoint_sequence: 0,
        artifact: "screen/verified.mp4".into(),
        sha256: "a".repeat(64),
        logical_offset_ms: offset_ms,
        source_duration_ms: duration_ms,
    }
}

fn padding(start_ms: u64, end_ms: u64) -> RunAwareStitchSpan {
    RunAwareStitchSpan::EncoderGapPadding {
        run_sequence: 0,
        logical_start_ms: start_ms,
        logical_end_ms: end_ms,
    }
}

fn plan(duration_ms: u64, spans: Vec<RunAwareStitchSpan>) -> RunAwareStitchPlan {
    RunAwareStitchPlan { duration_ms, spans }
}

#[test]
fn exact_rate_table_keeps_expected_frame_counts_and_endpoints() {
    let cases = [
        (FrameRate::new(25, 1).unwrap(), 1_000, 25),
        (
            FrameRate::from_server_decimal(29.97).unwrap(),
            100_000,
            2_997,
        ),
        (FrameRate::new(30, 1).unwrap(), 1_000, 30),
        (FrameRate::new(50, 1).unwrap(), 1_000, 50),
        (
            FrameRate::from_server_decimal(59.94).unwrap(),
            100_000,
            5_994,
        ),
        (FrameRate::new(60, 1).unwrap(), 1_000, 60),
    ];
    for (rate, duration_ms, expected_frames) in cases {
        let grid =
            quantize_run_aware_stitch(&plan(duration_ms, vec![source(0, duration_ms)]), rate)
                .unwrap();
        assert_eq!(grid.output_frame_count, expected_frames, "{rate:?}");
        assert_eq!(grid.spans[0].start_frame, 0);
        assert_eq!(grid.spans[0].end_frame, expected_frames);
        assert_eq!(
            u128::from(grid.expected_duration_ms.numerator) * u128::from(rate.num),
            u128::from(grid.expected_duration_ms.denominator)
                * u128::from(expected_frames)
                * u128::from(rate.den)
                * 1_000,
            "{rate:?} exact duration must equal its frame count"
        );
    }
}

#[test]
fn sub_frame_padding_can_be_zero_without_losing_adjacent_sources() {
    let grid = quantize_run_aware_stitch(
        &plan(90, vec![source(0, 40), padding(40, 50), source(50, 40)]),
        FrameRate::new(25, 1).unwrap(),
    )
    .unwrap();
    assert_eq!(grid.output_frame_count, 2);
    assert_eq!(grid.spans[0].frame_count(), 1);
    assert_eq!(grid.spans[1].frame_count(), 0);
    assert_eq!(grid.spans[2].frame_count(), 1);
    assert_contiguous(&grid);
}

#[test]
fn repeated_sub_frame_gaps_round_from_cumulative_boundaries() {
    let grid = quantize_run_aware_stitch(
        &plan(
            140,
            vec![
                source(0, 40),
                padding(40, 50),
                source(50, 40),
                padding(90, 100),
                source(100, 40),
            ],
        ),
        FrameRate::new(25, 1).unwrap(),
    )
    .unwrap();
    assert_eq!(grid.output_frame_count, 4);
    assert_eq!(grid.spans[1].frame_count(), 0);
    assert_eq!(grid.spans[3].frame_count(), 1);
    assert_contiguous(&grid);
}

#[test]
fn long_duration_keeps_exact_endpoints_without_per_span_accumulation() {
    let duration_ms = u64::MAX / 4;
    let first_duration_ms = duration_ms / 2;
    let grid = quantize_run_aware_stitch(
        &plan(
            duration_ms,
            vec![
                source(0, first_duration_ms),
                source(first_duration_ms, duration_ms - first_duration_ms),
            ],
        ),
        FrameRate::new(60, 1).unwrap(),
    )
    .unwrap();
    assert_contiguous(&grid);
    assert_eq!(
        grid.spans.last().unwrap().end_frame,
        grid.output_frame_count
    );
}

#[test]
fn malformed_or_unrepresentable_plans_fail_closed() {
    assert_eq!(
        quantize_run_aware_stitch(
            &plan(100, vec![source(10, 90)]),
            FrameRate::new(30, 1).unwrap(),
        ),
        Err(FrameGridError::MalformedPlan)
    );
    assert_eq!(
        quantize_run_aware_stitch(
            &plan(u64::MAX, vec![source(0, u64::MAX)]),
            FrameRate::new(u64::MAX, 1).unwrap(),
        ),
        Err(FrameGridError::Overflow)
    );
}

fn assert_contiguous(grid: &crate::FrameGridStitchPlan) {
    assert_eq!(grid.spans.first().unwrap().start_frame, 0);
    for pair in grid.spans.windows(2) {
        assert_eq!(pair[0].end_frame, pair[1].start_frame);
    }
    assert_eq!(
        grid.spans.last().unwrap().end_frame,
        grid.output_frame_count
    );
}

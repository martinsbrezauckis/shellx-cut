//! Exact native sample intervals and screen-clock projection.

use super::*;

pub(super) struct SampleAnchor {
    native_start_hns: i64,
    projected_start: Instant,
}

pub(super) fn record_sample(
    state: &mut EventFlags,
    screen_origin: Instant,
    delivered_at: Instant,
    sample_time_hns: i64,
    sample_duration_hns: i64,
) -> std::result::Result<(), &'static str> {
    if sample_time_hns < 0 {
        return Err("negative_time");
    }
    if sample_duration_hns <= 0 {
        return Err("nonpositive_duration");
    }
    let native_end_hns = sample_time_hns
        .checked_add(sample_duration_hns)
        .ok_or("native_end_overflow")?;
    if state
        .last_native_start_hns
        .is_some_and(|previous| sample_time_hns <= previous)
    {
        return Err("nonincreasing_native_start");
    }
    let anchor = state.first_sample.get_or_insert(SampleAnchor {
        native_start_hns: sample_time_hns,
        projected_start: ceil_screen_millisecond(screen_origin, delivered_at)
            .map_err(|_| "screen_clock_projection")?,
    });
    let started_at =
        map_native_time(anchor, sample_time_hns).map_err(|_| "native_start_projection")?;
    let ended_at = map_native_time(anchor, native_end_hns).map_err(|_| "native_end_projection")?;
    if ended_at <= started_at {
        return Err("nonpositive_projected_interval");
    }
    // A camera can report a nominal duration longer than the interval to its
    // next real sample. That next timestamp bounds the preceding frame; it
    // does not make an otherwise advancing native stream a failed capture.
    if let Some(previous) = state.observations.last_mut() {
        if previous.ended_at > started_at {
            previous.ended_at = started_at;
        }
    }
    state
        .observations
        .push(CameraFrameObservation::new(started_at, ended_at));
    state.first_native_start_hns.get_or_insert(sample_time_hns);
    state.last_native_start_hns = Some(sample_time_hns);
    state.last_native_end_hns = Some(native_end_hns);
    Ok(())
}

fn ceil_screen_millisecond(
    screen_origin: Instant,
    delivered_at: Instant,
) -> std::result::Result<Instant, ()> {
    let elapsed = delivered_at
        .checked_duration_since(screen_origin)
        .unwrap_or_default();
    let whole_ms = u64::try_from(elapsed.as_millis()).map_err(|_| ())?;
    let rounded_ms = whole_ms
        .checked_add(u64::from(!elapsed.subsec_nanos().is_multiple_of(1_000_000)))
        .ok_or(())?;
    screen_origin
        .checked_add(Duration::from_millis(rounded_ms))
        .ok_or(())
}

fn map_native_time(anchor: &SampleAnchor, native_hns: i64) -> std::result::Result<Instant, ()> {
    let delta_hns = native_hns.checked_sub(anchor.native_start_hns).ok_or(())?;
    let delta = Duration::from_nanos(delta_hns.unsigned_abs().checked_mul(100).ok_or(())?);
    if delta_hns >= 0 {
        anchor.projected_start.checked_add(delta).ok_or(())
    } else {
        anchor.projected_start.checked_sub(delta).ok_or(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_stopped_event_closes_sample_admission_before_owner_wakes() {
        let signals = CaptureSignals::new(Instant::now());
        signals.admit_record_samples();
        signals.on_sample(1_000_000, 333_333);
        signals.on_event(MF_CAPTURE_ENGINE_RECORD_STOPPED, windows::core::HRESULT(0));
        signals.on_sample(1_100_000, 333_333);
        assert_eq!(signals.take_observations().len(), 1);
        assert_eq!(signals.stop_snapshot().record_stopped, Some(true));
        assert!(signals
            .wait_for(EventKind::RecordStopped, Duration::ZERO)
            .is_ok());
    }

    #[test]
    fn failed_start_wait_retains_requested_phase_and_first_mf_hresult() {
        let signals = CaptureSignals::new(Instant::now());
        signals.on_event(
            MF_CAPTURE_ENGINE_INITIALIZED,
            windows::core::HRESULT(0x80070005_u32 as i32),
        );
        let first = signals.stop_snapshot().first_failure.unwrap();
        let error = signals
            .wait_for(EventKind::Initialized, Duration::ZERO)
            .unwrap_err()
            .to_string();
        assert!(error.contains("requested=Initialized"));
        assert!(error.contains(&first));
        assert!(error.contains("status=0x80070005"));

        signals.on_event(
            MF_CAPTURE_ENGINE_ERROR,
            windows::core::HRESULT(0x80004005_u32 as i32),
        );
        let lost = signals
            .wait_for(EventKind::PreviewStarted, Duration::ZERO)
            .unwrap_err()
            .to_string();
        assert!(lost.contains("device loss; requested=PreviewStarted"));
        assert!(lost.contains(&first));
        assert!(!lost.contains("status=0x80004005"));
    }

    #[test]
    fn timeout_wait_names_requested_phase_without_inventing_an_mf_failure() {
        let signals = CaptureSignals::new(Instant::now());
        let error = signals
            .wait_for(EventKind::RecordStarted, Duration::ZERO)
            .unwrap_err()
            .to_string();
        assert!(error.contains("before timeout; requested=RecordStarted"));
        assert!(error.contains("first_failure=none"));
        assert!(!error.contains("mf_event:"));
    }

    #[test]
    fn first_native_interval_rejection_survives_later_mf_event_failure() {
        let signals = CaptureSignals::new(Instant::now());
        signals.admit_record_samples();
        signals.on_sample(1_000_000, 333_333);
        signals.on_sample(1_000_000, 333_333);
        let first = signals.stop_snapshot().first_failure.unwrap();
        assert!(first.contains("nonincreasing_native_start"));
        assert!(first.contains("previous_end=Some(1333333)"));
        assert!(signals.stop_snapshot().fatal_error);
        signals.on_event(
            MF_CAPTURE_ENGINE_RECORD_STOPPED,
            windows::core::HRESULT(0x80004005_u32 as i32),
        );
        let snapshot = signals.stop_snapshot();
        assert_eq!(snapshot.first_failure.as_deref(), Some(first.as_str()));
        assert_eq!(snapshot.record_stopped_hresult, Some(0x80004005));
        assert_eq!(snapshot.record_stopped, Some(false));
    }

    #[test]
    fn first_getter_error_survives_later_interval_failure() {
        let signals = CaptureSignals::new(Instant::now());
        signals.note_sample_getter_error(
            "duration",
            &windows::core::Error::from_hresult(windows::core::HRESULT(0x80004005_u32 as i32)),
        );
        let first = signals.stop_snapshot().first_failure.unwrap();
        assert!(first.contains("sample_getter:duration:hresult=HRESULT(0x80004005)"));
        signals.admit_record_samples();
        signals.on_sample(1_000_000, 0);
        assert!(signals.stop_snapshot().fatal_error);
        assert_eq!(
            signals.stop_snapshot().first_failure.as_deref(),
            Some(first.as_str())
        );
    }

    #[test]
    fn failed_record_stopped_hresult_remains_first_after_getter_error() {
        let signals = CaptureSignals::new(Instant::now());
        signals.on_event(
            MF_CAPTURE_ENGINE_RECORD_STOPPED,
            windows::core::HRESULT(0x80070005_u32 as i32),
        );
        let first = signals.stop_snapshot().first_failure.unwrap();
        assert!(first.contains("status=0x80070005"));
        signals.note_sample_getter_error(
            "time",
            &windows::core::Error::from_hresult(windows::core::HRESULT(0x80004005_u32 as i32)),
        );
        let snapshot = signals.stop_snapshot();
        assert_eq!(snapshot.first_failure.as_deref(), Some(first.as_str()));
        assert_eq!(snapshot.record_stopped_hresult, Some(0x80070005));
    }

    #[test]
    fn advancing_samples_bound_nominal_overlap_at_the_next_native_start() {
        let origin = Instant::now();
        let mut state = EventFlags::default();
        record_sample(&mut state, origin, origin, 2_320_741, 333_333).unwrap();
        record_sample(&mut state, origin, origin, 2_417_504, 333_333).unwrap();
        assert_eq!(
            state.observations[0].ended_at,
            state.observations[1].started_at
        );
        assert_eq!(
            state.observations[0].ended_at - origin,
            Duration::from_nanos(9_676_300)
        );
        assert_eq!(
            state.observations[1].ended_at - origin,
            Duration::from_nanos(43_009_600)
        );
        assert_eq!(state.first_native_start_hns, Some(2_320_741));
        assert_eq!(state.last_native_end_hns, Some(2_750_837));
    }

    #[test]
    fn real_native_gaps_and_nonoverlapping_durations_are_preserved() {
        let origin = Instant::now();
        let mut state = EventFlags::default();
        record_sample(&mut state, origin, origin, 1_000_000, 100_000).unwrap();
        record_sample(&mut state, origin, origin, 1_200_000, 100_000).unwrap();
        assert_eq!(
            state.observations[0].ended_at - origin,
            Duration::from_millis(10)
        );
        assert_eq!(
            state.observations[1].started_at - origin,
            Duration::from_millis(20)
        );
        assert_eq!(
            state.observations[1].ended_at - origin,
            Duration::from_millis(30)
        );
    }

    #[test]
    fn invalid_native_samples_leave_the_preceding_interval_unchanged() {
        let origin = Instant::now();
        let mut state = EventFlags::default();
        record_sample(&mut state, origin, origin, 1_000_000, 333_333).unwrap();
        let end = state.observations[0].ended_at;
        for (time, duration, reason) in [
            (1_000_000, 333_333, "nonincreasing_native_start"),
            (999_999, 333_333, "nonincreasing_native_start"),
            (-1, 333_333, "negative_time"),
            (1_100_000, 0, "nonpositive_duration"),
            (i64::MAX, 1, "native_end_overflow"),
        ] {
            assert_eq!(
                record_sample(&mut state, origin, origin, time, duration),
                Err(reason)
            );
            assert_eq!(state.observations.len(), 1);
            assert_eq!(state.observations[0].ended_at, end);
            assert_eq!(state.last_native_start_hns, Some(1_000_000));
            assert_eq!(state.last_native_end_hns, Some(1_333_333));
        }
    }

    #[test]
    fn nominal_overlap_does_not_bypass_native_record_stopped_failure() {
        let signals = CaptureSignals::new(Instant::now());
        signals.admit_record_samples();
        signals.on_sample(1_000_000, 333_333);
        signals.on_sample(1_100_000, 333_333);
        assert!(!signals.stop_snapshot().fatal_error);
        signals.on_event(
            MF_CAPTURE_ENGINE_RECORD_STOPPED,
            windows::core::HRESULT(0x80004005_u32 as i32),
        );
        assert!(signals
            .wait_for(EventKind::RecordStopped, Duration::ZERO)
            .is_err());
        assert_eq!(signals.stop_snapshot().record_stopped, Some(false));
        assert!(signals.stop_snapshot().fatal_error);
    }
}

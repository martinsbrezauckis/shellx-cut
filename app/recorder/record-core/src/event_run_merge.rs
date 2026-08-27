//! Pure merge planning for input events from pause-separated sealed recording runs.
//!
//! A recording session's logical clock omits wall-clock pause time. Native capture
//! workers therefore hand this module only already-sealed input tracks together
//! with their compact logical offsets. This module never opens artifacts, starts
//! workers, or decides whether a run is sealed; it validates and combines their
//! in-memory facts without inventing samples for the pauses between them.

use serde::{Deserialize, Serialize};

use crate::{
    error_codes, ClickSample, CursorCorrelation, CursorSample, EventTrack, KeySample, RecordError,
    Result, ScrollSample, Settings,
};

/// One input-event track from an already-sealed recording run.
///
/// `logical_offset_ms` is a compact session-time position, not a wall-clock
/// timestamp. Each run occupies the half-open interval
/// `[logical_offset_ms, logical_offset_ms + events.duration_ms)`. The caller
/// must supply runs contiguously: a later run starts exactly at the prior end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SealedEventTrackRun {
    pub logical_offset_ms: u64,
    pub settings: Settings,
    pub events: EventTrack,
}

/// Event-track output and its matching capture settings.
///
/// `EventTrack` deliberately does not carry FPS, so preserving the actual
/// `Settings` type keeps the merged stream tied to the dimensions and frame
/// cadence that its source runs proved compatible with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergedEventTrack {
    pub settings: Settings,
    pub events: EventTrack,
}

/// Merge pause-separated, already-sealed event runs onto their compact timeline.
///
/// The input order is authoritative and every later run must start exactly at
/// the prior half-open interval's end. This keeps wall-clock pause time out of
/// the merged duration. Every input event timestamp is checked against its
/// run's half-open interval before the compact offset is applied with checked
/// arithmetic.
pub fn merge_sealed_event_tracks(runs: &[SealedEventTrackRun]) -> Result<MergedEventTrack> {
    let Some(first) = runs.first() else {
        return Err(invalid("at least one sealed event-track run is required"));
    };
    if first.logical_offset_ms != 0 {
        return Err(invalid(
            "the first sealed run must begin at logical offset zero",
        ));
    }

    let mut merged = EventTrack {
        duration_ms: 0,
        screen_w: first.events.screen_w,
        screen_h: first.events.screen_h,
        monitors: first.events.monitors.clone(),
        cursor: Vec::new(),
        clicks: Vec::new(),
        scrolls: Vec::new(),
        keys: Vec::new(),
        cursor_correlation: first.events.cursor_correlation.clone(),
    };
    let mut previous_end = 0;

    for (index, run) in runs.iter().enumerate() {
        validate_run(run, index)?;
        if index > 0 {
            validate_compatible(first, run, index)?;
        }
        if run.logical_offset_ms != previous_end {
            return Err(invalid(format!(
                "run {index} is not contiguous with the prior half-open logical interval"
            )));
        }
        let run_end = checked_timestamp(run.logical_offset_ms, run.events.duration_ms, index)?;
        append_run_events(&mut merged, run, index)?;
        if index > 0 {
            add_correlation_counts(
                &mut merged.cursor_correlation,
                &run.events.cursor_correlation,
            )?;
        }
        previous_end = run_end;
    }

    merged.duration_ms = previous_end;
    Ok(MergedEventTrack {
        settings: first.settings,
        events: merged,
    })
}

fn validate_run(run: &SealedEventTrackRun, index: usize) -> Result<()> {
    if !valid_fps(run.settings.fps) {
        return Err(invalid(format!(
            "run {index} has a non-finite or unsupported FPS"
        )));
    }
    if run.settings.width != run.events.screen_w || run.settings.height != run.events.screen_h {
        return Err(invalid(format!(
            "run {index} settings dimensions do not match its event-track geometry"
        )));
    }
    if run.events.screen_w == 0 || run.events.screen_h == 0 {
        return Err(invalid(format!(
            "run {index} has zero-sized event geometry"
        )));
    }
    if run.events.duration_ms == 0 {
        return Err(invalid(format!(
            "run {index} has zero duration and cannot be a sealed event run"
        )));
    }

    validate_cursor(&run.events.cursor, &run.events, index)?;
    validate_clicks(&run.events.clicks, &run.events, index)?;
    validate_scrolls(&run.events.scrolls, &run.events, index)?;
    validate_keys(&run.events.keys, run.events.duration_ms, index)
}

fn validate_compatible(
    first: &SealedEventTrackRun,
    run: &SealedEventTrackRun,
    index: usize,
) -> Result<()> {
    if run.settings != first.settings {
        return Err(invalid(format!(
            "run {index} settings differ from the first sealed run"
        )));
    }
    if run.events.screen_w != first.events.screen_w || run.events.screen_h != first.events.screen_h
    {
        return Err(invalid(format!(
            "run {index} event-track geometry differs from the first sealed run"
        )));
    }
    if run.events.monitors != first.events.monitors {
        return Err(invalid(format!(
            "run {index} monitor contract drifted from the first sealed run"
        )));
    }
    if !same_correlation_contract(
        &run.events.cursor_correlation,
        &first.events.cursor_correlation,
    ) {
        return Err(invalid(format!(
            "run {index} cursor-correlation contract drifted from the first sealed run"
        )));
    }
    Ok(())
}

fn valid_fps(fps: f32) -> bool {
    fps.is_finite() && (0.0 < fps && fps <= 240.0)
}

fn same_correlation_contract(left: &CursorCorrelation, right: &CursorCorrelation) -> bool {
    left.source == right.source
        && left.state == right.state
        && left.max_metadata_age_ms == right.max_metadata_age_ms
        && left.detail == right.detail
}

fn validate_cursor(samples: &[CursorSample], events: &EventTrack, index: usize) -> Result<()> {
    validate_timestamps(
        samples.iter().map(|sample| sample.t_ms),
        events.duration_ms,
        "cursor",
        index,
    )?;
    for sample in samples {
        validate_position(sample.x, sample.y, events, "cursor", index)?;
    }
    Ok(())
}

fn validate_clicks(samples: &[ClickSample], events: &EventTrack, index: usize) -> Result<()> {
    validate_timestamps(
        samples.iter().map(|sample| sample.t_ms),
        events.duration_ms,
        "click",
        index,
    )?;
    for sample in samples {
        validate_position(sample.x, sample.y, events, "click", index)?;
    }
    Ok(())
}

fn validate_scrolls(samples: &[ScrollSample], events: &EventTrack, index: usize) -> Result<()> {
    validate_timestamps(
        samples.iter().map(|sample| sample.t_ms),
        events.duration_ms,
        "scroll",
        index,
    )?;
    for sample in samples {
        validate_position(sample.x, sample.y, events, "scroll", index)?;
    }
    Ok(())
}

fn validate_keys(samples: &[KeySample], duration_ms: u64, index: usize) -> Result<()> {
    validate_timestamps(
        samples.iter().map(|sample| sample.t_ms),
        duration_ms,
        "key",
        index,
    )
}

fn validate_timestamps(
    timestamps: impl IntoIterator<Item = u64>,
    duration_ms: u64,
    kind: &str,
    run_index: usize,
) -> Result<()> {
    let mut previous = None;
    for timestamp in timestamps {
        if timestamp >= duration_ms {
            return Err(invalid(format!(
                "run {run_index} {kind} sample is outside its half-open duration"
            )));
        }
        if previous.is_some_and(|previous| timestamp < previous) {
            return Err(invalid(format!(
                "run {run_index} {kind} samples are not monotonically ordered"
            )));
        }
        previous = Some(timestamp);
    }
    Ok(())
}

fn validate_position(
    x: f64,
    y: f64,
    events: &EventTrack,
    kind: &str,
    run_index: usize,
) -> Result<()> {
    let contained = x.is_finite()
        && y.is_finite()
        && (0.0..events.screen_w as f64).contains(&x)
        && (0.0..events.screen_h as f64).contains(&y);
    if contained {
        Ok(())
    } else {
        Err(invalid(format!(
            "run {run_index} {kind} sample falls outside its half-open event geometry"
        )))
    }
}

fn append_run_events(
    merged: &mut EventTrack,
    run: &SealedEventTrackRun,
    index: usize,
) -> Result<()> {
    for sample in &run.events.cursor {
        merged.cursor.push(CursorSample {
            t_ms: checked_timestamp(run.logical_offset_ms, sample.t_ms, index)?,
            ..*sample
        });
    }
    for sample in &run.events.clicks {
        merged.clicks.push(ClickSample {
            t_ms: checked_timestamp(run.logical_offset_ms, sample.t_ms, index)?,
            ..*sample
        });
    }
    for sample in &run.events.scrolls {
        merged.scrolls.push(ScrollSample {
            t_ms: checked_timestamp(run.logical_offset_ms, sample.t_ms, index)?,
            ..*sample
        });
    }
    for sample in &run.events.keys {
        merged.keys.push(KeySample {
            t_ms: checked_timestamp(run.logical_offset_ms, sample.t_ms, index)?,
            ..sample.clone()
        });
    }
    Ok(())
}

fn checked_timestamp(offset_ms: u64, timestamp_ms: u64, run_index: usize) -> Result<u64> {
    offset_ms.checked_add(timestamp_ms).ok_or_else(|| {
        invalid(format!(
            "run {run_index} logical offset plus event timestamp overflows u64"
        ))
    })
}

fn add_correlation_counts(output: &mut CursorCorrelation, input: &CursorCorrelation) -> Result<()> {
    output.exact_clicks = checked_count(output.exact_clicks, input.exact_clicks)?;
    output.approximate_clicks = checked_count(output.approximate_clicks, input.approximate_clicks)?;
    output.unavailable_clicks = checked_count(output.unavailable_clicks, input.unavailable_clicks)?;
    Ok(())
}

fn checked_count(left: u32, right: u32) -> Result<u32> {
    left.checked_add(right)
        .ok_or_else(|| invalid("cursor-correlation count overflows u32"))
}

fn invalid(cause: impl Into<String>) -> RecordError {
    RecordError::new(
        error_codes::INVALID_ARGS,
        "merge sealed event-track runs",
        cause,
    )
    .with_action(
        "keep every run's geometry, settings, timestamps, and correlation contract compatible",
    )
}

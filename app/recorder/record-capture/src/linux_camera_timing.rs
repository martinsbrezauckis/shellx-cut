//! Native GStreamer sample timing mapped to the shared screen clock.

use std::time::{Duration, Instant};

use record_core::Result;

use super::native::*;
use super::{error, CameraFrameObservation};

pub(super) fn interval_bounds(run: &NativeRun) -> Result<(u64, u64)> {
    let count = unsafe { sxc_linux_camera_count(run.ptr()) };
    let mut first = 0;
    let mut first_end = 0;
    let mut last = 0;
    let mut end = 0;
    if count == 0
        || unsafe { sxc_linux_camera_interval(run.ptr(), 0, &mut first, &mut first_end) } != 0
        || unsafe { sxc_linux_camera_interval(run.ptr(), count - 1, &mut last, &mut end) } != 0
        || first_end <= first
        || end <= last
        || end <= first
    {
        return Err(error(
            "measure Linux camera",
            "native frame interval is invalid",
        ));
    }
    Ok((first, end))
}

pub(super) fn shape(run: &NativeRun) -> Result<(u32, u32, u32, u32)> {
    let (mut w, mut h, mut n, mut d) = (0, 0, 0, 0);
    if unsafe { sxc_linux_camera_shape(run.ptr(), &mut w, &mut h, &mut n, &mut d) } != 0 {
        return Err(error(
            "measure Linux camera",
            "native negotiated video shape is unavailable",
        ));
    }
    Ok((w, h, n, d))
}

pub(super) fn clock_instant(run: &NativeRun, gst_ns: u64) -> Result<Instant> {
    let anchor = run
        .clock_anchor
        .ok_or_else(|| error("measure Linux camera", "native clock was not calibrated"))?;
    let instant = if gst_ns >= run.gst_clock_ns {
        anchor.checked_add(Duration::from_nanos(gst_ns - run.gst_clock_ns))
    } else {
        anchor.checked_sub(Duration::from_nanos(run.gst_clock_ns - gst_ns))
    };
    instant.ok_or_else(|| error("measure Linux camera", "native sample timestamp overflow"))
}

pub(super) fn monotonic_instant(mono: u64) -> Result<Instant> {
    let before = Instant::now();
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) } != 0 {
        return Err(error(
            "measure Linux camera",
            "monotonic clock is unavailable",
        ));
    }
    let after = Instant::now();
    if after.duration_since(before) > Duration::from_millis(1) {
        return Err(error(
            "measure Linux camera",
            "monotonic clock calibration was interrupted",
        ));
    }
    let anchor = before + after.duration_since(before) / 2;
    let now_ns = u64::try_from(ts.tv_sec)
        .ok()
        .and_then(|s| s.checked_mul(1_000_000_000))
        .and_then(|s| s.checked_add(u64::try_from(ts.tv_nsec).ok()?))
        .ok_or_else(|| error("measure Linux camera", "monotonic clock overflow"))?;
    let instant = if mono >= now_ns {
        anchor.checked_add(Duration::from_nanos(mono - now_ns))
    } else {
        anchor.checked_sub(Duration::from_nanos(now_ns - mono))
    };
    instant.ok_or_else(|| error("measure Linux camera", "native sample timestamp overflow"))
}

pub(super) fn project_interval(
    start: Instant,
    end: Instant,
    origin: Instant,
) -> Result<(CameraFrameObservation, u64)> {
    let start_ns = start
        .checked_duration_since(origin)
        .ok_or_else(|| error("measure Linux camera", "camera frame precedes screen clock"))?
        .as_nanos();
    let duration_ns = end
        .checked_duration_since(start)
        .ok_or_else(|| error("measure Linux camera", "camera interval reversed"))?
        .as_nanos();
    let start_ms = u64::try_from((start_ns + 500_000) / 1_000_000)
        .map_err(|_| error("measure Linux camera", "camera start offset overflow"))?;
    let duration_ms = u64::try_from((duration_ns + 500_000) / 1_000_000)
        .map_err(|_| error("measure Linux camera", "camera duration overflow"))?;
    if duration_ms == 0 {
        return Err(error(
            "measure Linux camera",
            "native frame interval is under one millisecond",
        ));
    }
    let projected_start = origin
        .checked_add(Duration::from_millis(start_ms))
        .ok_or_else(|| error("measure Linux camera", "projected start overflow"))?;
    let projected_end = projected_start
        .checked_add(Duration::from_millis(duration_ms))
        .ok_or_else(|| error("measure Linux camera", "projected end overflow"))?;
    let end_error = if projected_end >= end {
        projected_end.duration_since(end)
    } else {
        end.duration_since(projected_end)
    };
    if end_error > Duration::from_millis(1) {
        return Err(error(
            "measure Linux camera",
            "native-to-screen millisecond projection exceeds one millisecond",
        ));
    }
    Ok((
        CameraFrameObservation::new(projected_start, projected_end),
        duration_ms,
    ))
}

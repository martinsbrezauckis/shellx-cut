//! Independent MovieFileOutput and encoded-packet clock proof for a camera seal.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::macos_camera_clock::{DecimalDuration, TimeBase};
use cut_media::ffmpeg::{run_owned_command, OwnedProcessControl};
use record_core::{error_codes, RecordError, Result};
use serde::{Deserialize, Serialize};

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_PACKET_ROW_BYTES: u64 = 64;
const PROBE_BASE_SECONDS: u64 = 60;
const PROBE_FRAMES_PER_STEP: u64 = 1_000;
const PROBE_SECONDS_PER_FRAME_STEP: u64 = 5;
const PROBE_BYTES_PER_STEP: u64 = 128 * 1024 * 1024;
const PROBE_SECONDS_PER_BYTE_STEP: u64 = 2;

#[derive(Debug, Clone, Copy, Serialize)]
pub(super) struct NativeMovieTiming {
    pub(super) start_pts_ns: u64,
    pub(super) last_pts_ns: u64,
    pub(super) last_duration_ns: u64,
    pub(super) last_cadence_ns: u64,
    pub(super) callback_count: u64,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct VerifiedMovieTiming {
    pub(super) duration_ms: u64,
    pub(super) start_pts_ns: u64,
}

#[derive(Deserialize)]
struct Probe {
    streams: Vec<Stream>,
    format: Format,
}

#[derive(Deserialize)]
struct Stream {
    time_base: String,
    duration_ts: u64,
}

#[derive(Deserialize)]
struct Format {
    duration: String,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) fn verify_movie_timing(
    ffprobe: &str,
    path: &Path,
    native: NativeMovieTiming,
    duration_ms: u64,
    decoded_frames: u64,
) -> Result<VerifiedMovieTiming> {
    let limit = packet_file_limit(decoded_frames)?;
    let movie = fs::metadata(path)
        .map_err(|cause| bad(&format!("inspect staged camera movie: {cause}")))?;
    if !movie.is_file() {
        return Err(bad("staged camera movie is not a regular file"));
    }
    // Full decode already established the frame count. A long or large movie
    // gets a proportionate finite process budget, while the owned child tree
    // is still cancelled and reaped if it stalls past that budget.
    let probe_timeout = probe_timeout(decoded_frames, movie.len())?;
    let mut metadata_command = Command::new(ffprobe);
    metadata_command
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=time_base,duration_ts:format=duration",
            "-of",
            "json",
        ])
        .arg(path);
    let metadata_control =
        OwnedProcessControl::bounded(probe_timeout, || false).with_output_cap(MAX_METADATA_BYTES);
    let metadata = run_owned_command(
        &mut metadata_command,
        &metadata_control,
        "probe macOS camera movie time base",
    )
    .map_err(|cause| bad(&format!("movie metadata probe failed: {cause}")))?;
    if !metadata.status.success() || metadata.stdout.len() >= MAX_METADATA_BYTES {
        return Err(bad(
            "movie metadata probe failed or exceeded its bounded output",
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| bad("camera staging parent is missing"))?;
    let packet_file = tempfile::NamedTempFile::new_in(parent)
        .map_err(|cause| bad(&format!("reserve private packet proof: {cause}")))?;
    let packet_path = packet_file.path().to_path_buf();
    let original = packet_file
        .as_file()
        .metadata()
        .map_err(|cause| bad(&format!("inspect private packet proof: {cause}")))?;
    if !original.file_type().is_file() || original.permissions().mode() & 0o777 != 0o600 {
        return Err(bad("private packet proof is not a plain 0600 file"));
    }
    let mut packet_command = Command::new(ffprobe);
    packet_command
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_packets",
            "-show_entries",
            "packet=pts,duration",
            "-of",
            "csv=p=0",
            "-o",
        ])
        .arg(&packet_path)
        .arg(path);
    // The owned process polls this predicate every 20ms and reaps its tree on
    // excess growth. The cap scales from the independently decoded frame count
    // while the parser retains at most one 64-byte row in memory.
    let growth_path = packet_path.clone();
    let packet_control = OwnedProcessControl::bounded(probe_timeout, move || {
        fs::metadata(&growth_path).is_ok_and(|metadata| metadata.len() > limit)
    })
    .with_output_cap(MAX_METADATA_BYTES);
    let output = run_owned_command(
        &mut packet_command,
        &packet_control,
        "probe macOS camera packets",
    )
    .map_err(|cause| bad(&format!("packet probe failed: {cause}")))?;
    if !output.status.success() || !output.stdout.is_empty() {
        return Err(bad("packet probe failed or wrote unexpected stdout"));
    }
    let after = fs::symlink_metadata(&packet_path)
        .map_err(|cause| bad(&format!("inspect private packet proof: {cause}")))?;
    if !after.file_type().is_file()
        || after.dev() != original.dev()
        || after.ino() != original.ino()
        || after.len() > limit
    {
        return Err(bad(
            "packet proof was replaced, linked, or exceeded its frame-count bound",
        ));
    }
    let ordinary = verify_probe(
        &metadata.stdout,
        BufReader::new(packet_file.as_file()),
        native,
        duration_ms,
        decoded_frames,
    );
    match ordinary {
        Ok(timing) => Ok(timing),
        Err(failure)
            if failure
                .cause
                .starts_with("video packet exceeds the container duration")
                || failure.cause == "video packet count exceeds decoded frame count" =>
        {
            let probe: Probe = serde_json::from_slice(&metadata.stdout)
                .map_err(|_| bad("movie metadata is malformed"))?;
            crate::macos_camera_edit_list::verify_movie_edit_list(
                ffprobe,
                path,
                native,
                &probe
                    .streams
                    .first()
                    .ok_or_else(|| bad("video time base is missing"))?
                    .time_base,
                probe.streams[0].duration_ts,
                &probe.format.duration,
                decoded_frames,
                probe_timeout,
            )
        }
        Err(failure) => Err(failure),
    }
}

fn probe_timeout(decoded_frames: u64, movie_bytes: u64) -> Result<Duration> {
    if decoded_frames == 0 || movie_bytes == 0 {
        return Err(bad("camera movie workload is empty"));
    }
    packet_file_limit(decoded_frames)?;
    let frame_seconds = decoded_frames
        .div_ceil(PROBE_FRAMES_PER_STEP)
        .checked_mul(PROBE_SECONDS_PER_FRAME_STEP)
        .ok_or_else(|| bad("camera movie probe frame budget overflowed"))?;
    let byte_seconds = movie_bytes
        .div_ceil(PROBE_BYTES_PER_STEP)
        .checked_mul(PROBE_SECONDS_PER_BYTE_STEP)
        .ok_or_else(|| bad("camera movie probe byte budget overflowed"))?;
    let seconds = PROBE_BASE_SECONDS
        .checked_add(frame_seconds)
        .and_then(|value| value.checked_add(byte_seconds))
        .ok_or_else(|| bad("camera movie probe budget overflowed"))?;
    Ok(Duration::from_secs(seconds))
}

fn packet_file_limit(decoded_frames: u64) -> Result<u64> {
    decoded_frames
        .checked_mul(MAX_PACKET_ROW_BYTES)
        .and_then(|bytes| bytes.checked_add(4_096))
        .filter(|_| decoded_frames > 0)
        .ok_or_else(|| bad("decoded frame count cannot bound packet proof"))
}

fn verify_probe<R: BufRead>(
    metadata_bytes: &[u8],
    mut packet_rows: R,
    native: NativeMovieTiming,
    duration_ms: u64,
    decoded_frames: u64,
) -> Result<VerifiedMovieTiming> {
    let probe: Probe = serde_json::from_slice(metadata_bytes)
        .map_err(|cause| bad(&format!("movie metadata is malformed: {cause}")))?;
    let stream = probe
        .streams
        .first()
        .ok_or_else(|| bad("video time base is missing"))?;
    let clock = TimeBase::parse(&stream.time_base)?;
    let (numerator, denominator) = (u128::from(clock.num), u128::from(clock.den));
    let decimal = DecimalDuration::parse(&probe.format.duration)?;
    let container_ns = u128::from(decimal.ns) * denominator;
    let printed_half_unit = u128::from(decimal.precision_ns) * denominator / 2;
    let measured_ms = u64::try_from(
        (container_ns / denominator)
            .checked_add(500_000)
            .ok_or_else(|| bad("container duration overflowed"))?
            / 1_000_000,
    )
    .map_err(|_| bad("movie duration overflows milliseconds"))?;
    if measured_ms == 0 || measured_ms != duration_ms {
        return Err(bad(
            "packet container duration disagrees with decoded media facts",
        ));
    }
    if native.start_pts_ns == 0
        || native.last_pts_ns < native.start_pts_ns
        || native.last_cadence_ns == 0
    {
        return Err(bad(
            "MovieFileOutput start or final sample timing is missing",
        ));
    }
    let mut first_pts = u128::MAX;
    let mut last_pts = 0_u128;
    let mut penultimate_pts = None;
    let mut last_duration = 0_u128;
    let mut max_end = 0_u128;
    let mut packet_count = 0_u64;
    loop {
        let mut row = Vec::with_capacity(MAX_PACKET_ROW_BYTES as usize);
        let read = (&mut packet_rows)
            .take(MAX_PACKET_ROW_BYTES + 1)
            .read_until(b'\n', &mut row)
            .map_err(|cause| bad(&format!("read packet proof: {cause}")))?;
        if read == 0 {
            break;
        }
        if read as u64 > MAX_PACKET_ROW_BYTES || row.last() != Some(&b'\n') {
            return Err(bad("packet proof contains an oversized or incomplete row"));
        }
        let row = std::str::from_utf8(&row[..row.len() - 1])
            .map_err(|_| bad("packet proof is not UTF-8"))?;
        let (pts, duration) = row
            .split_once(',')
            .ok_or_else(|| bad("packet proof row is malformed"))?;
        let pts = pts
            .parse::<i64>()
            .map_err(|_| bad("packet PTS is malformed"))?;
        let duration = duration
            .parse::<i64>()
            .map_err(|_| bad("packet duration is malformed"))?;
        packet_count = packet_count
            .checked_add(1)
            .ok_or_else(|| bad("packet count overflowed"))?;
        if packet_count > decoded_frames {
            return Err(bad("video packet count exceeds decoded frame count"));
        }
        if pts < 0 || duration <= 0 {
            return Err(bad("video packet has negative PTS or no duration"));
        }
        let pts = to_scaled(pts as u128, numerator)?;
        let duration = to_scaled(duration as u128, numerator)?;
        let end = pts
            .checked_add(duration)
            .ok_or_else(|| bad("packet end overflowed"))?;
        if duration == 0
            || end
                > container_ns
                    .checked_add(printed_half_unit)
                    .ok_or_else(|| bad("container duration overflowed"))?
        {
            return Err(bad("video packet exceeds the container duration"));
        }
        first_pts = first_pts.min(pts);
        if pts > last_pts {
            penultimate_pts = Some(last_pts);
            last_pts = pts;
            last_duration = duration;
        } else if pts == last_pts {
            last_duration = duration;
        } else if penultimate_pts.is_none_or(|previous| pts > previous) {
            penultimate_pts = Some(pts);
        }
        max_end = max_end.max(end);
    }
    if packet_count != decoded_frames {
        return Err(bad("video packet count disagrees with decoded frame count"));
    }
    if first_pts != 0 || last_duration == 0 || last_pts.checked_add(last_duration) != Some(max_end)
    {
        return Err(bad(
            "movie packets do not begin at zero with a final sample",
        ));
    }
    // One track tick accounts for CMTime->nanosecond truncation and ffprobe's
    // integer time base. At Stop, the encoded movie can contain one final
    // packet beyond the last delivered MovieFileOutput sample callback.
    // Admit that gap only in the encoded-ahead direction and only for one
    // measured frame interval. For variable frame cadence, the callback may
    // instead coincide with the actual penultimate packet PTS; that proves
    // exactly one later encoded packet without inventing a timing tolerance.
    // Conversely, a final callback at the encoded
    // packet's end is the first sample outside that half-open movie interval.
    // A callback beyond the packet end remains an unencoded tail and fails.
    let tick_ns = to_scaled(1, numerator)?.max(denominator);
    let native_elapsed = u128::from(native.last_pts_ns - native.start_pts_ns) * denominator;
    let encoded_ahead_limit = last_duration
        .checked_add(tick_ns)
        .ok_or_else(|| bad("encoded sample cadence overflows"))?;
    let matches_penultimate =
        penultimate_pts.is_some_and(|pts| native_elapsed.abs_diff(pts) <= tick_ns);
    if (last_pts >= native_elapsed
        && last_pts - native_elapsed > encoded_ahead_limit
        && !matches_penultimate)
        || (native_elapsed > max_end && native_elapsed - max_end > tick_ns)
    {
        let native_elapsed = native_elapsed / denominator;
        let last_pts = last_pts / denominator;
        let last_duration = last_duration / denominator;
        let max_end = max_end / denominator;
        let container_ns = container_ns / denominator;
        let tick_ns = tick_ns / denominator;
        let penultimate_pts = penultimate_pts.map(|v| v / denominator);
        let penultimate = penultimate_pts.map_or_else(|| "null".to_owned(), |pts| pts.to_string());
        let penultimate_delta = penultimate_pts.map_or_else(
            || "null".to_owned(),
            |pts| native_elapsed.abs_diff(pts).to_string(),
        );
        return Err(bad(&format!(
            "last encoded packet is not the last MovieFileOutput sample: native_start_ns={}, native_last_ns={}, native_elapsed_ns={native_elapsed}, encoded_last_pts_ns={last_pts}, encoded_penultimate_pts_ns={penultimate}, native_penultimate_delta_ns={penultimate_delta}, encoded_last_duration_ns={last_duration}, encoded_end_ns={max_end}, container_end_ns={container_ns}, native_last_duration_ns={}, native_last_cadence_ns={}, packet_count={packet_count}, decoded_frames={decoded_frames}, track_tick_ns={tick_ns}",
            native.start_pts_ns, native.last_pts_ns, native.last_duration_ns, native.last_cadence_ns,
        )));
    }
    let cadence = u128::from(native.last_cadence_ns) * denominator;
    if last_duration
        > cadence
            .checked_mul(2)
            .and_then(|v| v.checked_add(tick_ns))
            .ok_or_else(|| bad("native sample cadence overflows"))?
    {
        return Err(bad(
            "final packet duration exceeds native movie sample cadence",
        ));
    }
    if native.last_duration_ns != 0
        && last_duration
            > (u128::from(native.last_duration_ns) * denominator)
                .checked_mul(2)
                .and_then(|v| v.checked_add(tick_ns))
                .ok_or_else(|| bad("native sample duration overflows"))?
    {
        return Err(bad(
            "final packet duration exceeds native movie sample duration",
        ));
    }
    // The container may end after the final packet because AVFoundation writes
    // a movie time range. Bound that tail by one measured encoded packet; a
    // truncated or extended file cannot pass just by changing its duration.
    if max_end
        > container_ns
            .checked_add(printed_half_unit)
            .ok_or_else(|| bad("container duration overflows"))?
        || container_ns.saturating_sub(max_end)
            > last_duration
                .checked_add(tick_ns)
                .and_then(|v| v.checked_add(printed_half_unit))
                .ok_or_else(|| bad("container end bound overflows"))?
    {
        return Err(bad(
            "movie container end is not bounded by its final packet",
        ));
    }
    let packet_duration_ms = u64::try_from(
        max_end
            .checked_add(500_000 * denominator)
            .ok_or_else(|| bad("encoded movie interval overflowed"))?
            / (1_000_000 * denominator),
    )
    .map_err(|_| bad("encoded movie interval overflows milliseconds"))?;
    if packet_duration_ms == 0 {
        return Err(bad("encoded movie interval is empty"));
    }
    Ok(VerifiedMovieTiming {
        duration_ms: packet_duration_ms,
        start_pts_ns: native.start_pts_ns,
    })
}

fn to_scaled(ticks: u128, numerator: u128) -> Result<u128> {
    ticks
        .checked_mul(numerator)
        .and_then(|v| v.checked_mul(1_000_000_000))
        .ok_or_else(|| bad("movie time base overflowed"))
}

fn bad(cause: &str) -> RecordError {
    RecordError::new(
        error_codes::CAPTURE,
        "verify macOS camera movie clock",
        cause,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractional_tick_packet_end_accepts_only_printed_decimal_rounding() {
        let mut timing = native();
        timing.last_pts_ns = timing.start_pts_ns + 1_666_666;
        timing.last_cadence_ns = 1_666_667;
        timing.last_duration_ns = 1_666_667;
        let rows = b"0,1\n1,1\n";
        assert!(verify_probe(
            &metadata("0.003333", "1/600"),
            rows.as_slice(),
            timing,
            3,
            2
        )
        .is_ok());
        assert!(verify_probe(
            &metadata("0.003332", "1/600"),
            rows.as_slice(),
            timing,
            3,
            2
        )
        .is_err());
        assert!(verify_probe(
            &metadata("0.003333000", "1/600"),
            rows.as_slice(),
            timing,
            3,
            2
        )
        .is_err());
        assert!(verify_probe(
            &metadata("0.003333", "1/600"),
            rows.as_slice(),
            timing,
            3,
            3
        )
        .is_err());
        timing.last_pts_ns += 10_000_000;
        assert!(verify_probe(
            &metadata("0.003333", "1/600"),
            rows.as_slice(),
            timing,
            3,
            2
        )
        .is_err());
    }

    #[test]
    fn retained_private_movie_uses_strict_edit_proof_after_packet_boundary_refusal() {
        let Ok(path) = std::env::var("SHELLX_CAMERA_EDIT_LIST_FIXTURE") else {
            return;
        };
        let native = NativeMovieTiming {
            start_pts_ns: 100_000_000_000,
            last_pts_ns: 108_101_400_000,
            last_duration_ns: 16_660_000,
            last_cadence_ns: 16_670_000,
            callback_count: 0,
        };
        let timing = verify_movie_timing("ffprobe", Path::new(&path), native, 7_134, 429).unwrap();
        assert_eq!(timing.duration_ms, 7_134);
        assert_eq!(timing.start_pts_ns, 100_967_860_000);
    }

    fn native() -> NativeMovieTiming {
        NativeMovieTiming {
            start_pts_ns: 34_421_447_440_000,
            last_pts_ns: 34_429_431_540_000,
            last_duration_ns: 0,
            last_cadence_ns: 8_330_000,
            callback_count: 0,
        }
    }

    fn metadata(duration: &str, time_base: &str) -> Vec<u8> {
        format!(
            r#"{{"streams":[{{"time_base":"{time_base}","duration_ts":0}}],"format":{{"duration":"{duration}"}}}}"#
        )
        .into_bytes()
    }

    fn fixture(
        duration: &str,
        last_pts: i64,
        last_duration: i64,
        frames: u64,
    ) -> Result<VerifiedMovieTiming> {
        let rows = format!("0,833\n{last_pts},{last_duration}\n");
        verify_probe(
            &metadata(duration, "1/100000"),
            rows.as_bytes(),
            native(),
            DecimalDuration::parse(duration)?.ns / 1_000_000,
            frames,
        )
    }

    #[test]
    fn movie_start_excludes_warmup_and_data_output_offset() {
        let result = fixture("7.998060", 798410, 833, 2).unwrap();
        assert_eq!(result.start_pts_ns, native().start_pts_ns);
        assert_eq!(result.duration_ms, 7_992);
        // DataOutput's first accepted sample was 100.03ms after movie start,
        // and its 7.966s interval was 32ms shorter than the MP4 container.
        assert_ne!(7_966, result.duration_ms);
    }

    #[test]
    fn out_of_order_movie_samples_still_use_the_encoded_endpoint() {
        // The packet probe uses presentation time, not row/arrival order.
        // Retaining its greatest callback PTS matches the encoded endpoint.
        let rows = b"0,833\n798410,833\n797577,833\n";
        let facts = metadata("7.998060", "1/100000");
        let verified = verify_probe(&facts, &rows[..], native(), 7_998, 3).unwrap();
        assert_eq!(verified.duration_ms, 7_992);
        let mut unencoded_tail = native();
        unencoded_tail.last_pts_ns = unencoded_tail.start_pts_ns + 7_992_450_000;
        let error = verify_probe(&facts, &rows[..], unencoded_tail, 7_998, 3).unwrap_err();
        assert_eq!(error.message, "verify macOS camera movie clock");
        assert_eq!(
            error.cause,
            "last encoded packet is not the last MovieFileOutput sample: native_start_ns=34421447440000, native_last_ns=34429439890000, native_elapsed_ns=7992450000, encoded_last_pts_ns=7984100000, encoded_penultimate_pts_ns=7975770000, native_penultimate_delta_ns=16680000, encoded_last_duration_ns=8330000, encoded_end_ns=7992430000, container_end_ns=7998060000, native_last_duration_ns=0, native_last_cadence_ns=8330000, packet_count=3, decoded_frames=3, track_tick_ns=10000"
        );
    }

    #[test]
    fn installed_mac_clock_boundary_retains_packet_duration_without_admitting_a_lost_tail() {
        // r1790529837456-791804960a73 retained only a 16.68ms PTS gap.
        // A 16.67ms final packet is within one track tick of that callback;
        // a 16.66ms final packet ends two ticks early and remains rejected.
        let mut native = native();
        native.start_pts_ns = 160_888_177_990_000;
        native.last_pts_ns = 160_892_794_290_000;
        native.last_cadence_ns = 16_670_000;
        native.last_duration_ns = 16_670_000;
        let metadata = metadata("4.616300", "1/100000");
        assert!(verify_probe(
            &metadata,
            b"0,1667\n459962,1667\n".as_slice(),
            native,
            4_616,
            2
        )
        .is_ok());
        let error = verify_probe(
            &metadata,
            b"0,1667\n459962,1666\n".as_slice(),
            native,
            4_616,
            2,
        )
        .unwrap_err();
        assert!(error.cause.contains("encoded_last_duration_ns=16660000"));
        assert!(error.cause.contains("encoded_penultimate_pts_ns=0"));
        assert!(error
            .cause
            .contains("native_penultimate_delta_ns=4616300000"));
        assert!(error.cause.contains("encoded_end_ns=4616280000"));
        assert!(error.cause.contains("container_end_ns=4616300000"));
        assert!(error.cause.contains("native_last_cadence_ns=16670000"));
        assert!(error.cause.contains("packet_count=2, decoded_frames=2"));
    }

    #[test]
    fn installed_mac_encoded_ahead_failure_retains_penultimate_identity() {
        // r12: the encoded endpoint leads the native callback by 16.92ms.
        // A preceding PTS 250us from the callback cannot prove one extra packet.
        let mut native = native();
        native.start_pts_ns = 165_370_560_600_000;
        native.last_pts_ns = 165_375_560_960_000;
        native.last_duration_ns = 16_670_000;
        native.last_cadence_ns = 16_670_000;
        let facts = metadata("5.033950", "1/100000");
        let rows = b"0,1667\n500061,1667\n501728,1667\n";
        let error = verify_probe(&facts, rows.as_slice(), native, 5_034, 3).unwrap_err();
        assert!(error.cause.contains("native_elapsed_ns=5000360000"));
        assert!(error.cause.contains("encoded_last_pts_ns=5017280000"));
        assert!(error
            .cause
            .contains("encoded_penultimate_pts_ns=5000610000"));
        assert!(error.cause.contains("native_penultimate_delta_ns=250000"));
        assert!(error.cause.contains("encoded_last_duration_ns=16670000"));
        assert!(error.cause.contains("encoded_end_ns=5033950000"));

        // The existing narrow admission still requires an actual packet PTS
        // at the native callback, within one track tick.
        let matching_rows = b"0,1667\n500036,1667\n501728,1667\n";
        assert!(verify_probe(&facts, matching_rows.as_slice(), native, 5_034, 3).is_ok());

        // Independent count and container guards must reject before clock
        // diagnostics could suggest that malformed media is admissible.
        let count = verify_probe(&facts, rows.as_slice(), native, 5_034, 4).unwrap_err();
        assert_eq!(
            count.cause,
            "video packet count disagrees with decoded frame count"
        );
        let short_container = metadata("5.033940", "1/100000");
        let container =
            verify_probe(&short_container, rows.as_slice(), native, 5_034, 3).unwrap_err();
        assert_eq!(
            container.cause,
            "video packet exceeds the container duration"
        );
    }

    #[test]
    fn single_packet_clock_failure_reports_null_penultimate() {
        let mut native = native();
        native.last_pts_ns = native.start_pts_ns + 16_680_000;
        native.last_duration_ns = 16_660_000;
        native.last_cadence_ns = 16_660_000;
        let error = verify_probe(
            &metadata("0.016660", "1/100000"),
            b"0,1666\n".as_slice(),
            native,
            17,
            1,
        )
        .unwrap_err();
        assert!(error.cause.contains("encoded_penultimate_pts_ns=null"));
        assert!(error.cause.contains("native_penultimate_delta_ns=null"));
    }

    #[test]
    fn native_stop_callback_at_encoded_packet_end_is_a_valid_boundary() {
        // Installed Mac receipt r1790480971388-89159d76211d: the last
        // callback was 16.65ms after the final packet PTS, within that
        // packet's 16.67ms encoded presentation interval.
        let metadata = metadata("6.933320", "1/100000");
        let rows = b"0,1667\n691665,1667\n";
        let mut native = native();
        native.start_pts_ns = 112_076_742_590_000;
        native.last_pts_ns = 112_083_675_890_000;
        native.last_cadence_ns = 16_670_000;
        native.last_duration_ns = 16_670_000;
        let verified = verify_probe(&metadata, &rows[..], native, 6_933, 2).unwrap();
        assert_eq!(verified.duration_ms, 6_933);

        // The one-track-tick rounding allowance is exact, not a free frame.
        native.last_pts_ns = native.start_pts_ns + 6_933_330_000;
        assert!(verify_probe(&metadata, &rows[..], native, 6_933, 2).is_ok());
        native.last_pts_ns += 10_000;
        assert!(verify_probe(&metadata, &rows[..], native, 6_933, 2).is_err());
    }

    #[test]
    fn one_variable_cadence_packet_after_native_callback_is_bounded_by_packet_identity() {
        // Installed Mac core r1790481152640-96535e238846 reported this
        // 17.079999ms lead. Its failed stage is not retained, so the rows
        // model the narrow case where the callback names the penultimate
        // encoded PTS; a different actual packet PTS must still fail.
        let metadata = metadata("6.866110", "1/100000");
        let rows = b"0,1667\n684944,1667\n683236,1667\n";
        let mut native = native();
        native.start_pts_ns = 112_277_249_459_999;
        native.last_pts_ns = 112_284_081_820_000;
        native.last_cadence_ns = 17_080_000;
        native.last_duration_ns = 16_670_000;
        let verified = verify_probe(&metadata, &rows[..], native, 6_866, 3).unwrap();
        assert_eq!(verified.duration_ms, 6_866);

        // The callback has to identify the actual preceding packet, including
        // when packet rows arrive out of presentation order.
        native.last_pts_ns -= 30_000;
        assert!(verify_probe(&metadata, &rows[..], native, 6_866, 3).is_err());
        native.last_pts_ns = native.start_pts_ns + 6_815_280_001;
        assert!(verify_probe(&metadata, &rows[..], native, 6_866, 3).is_err());
    }

    #[test]
    fn final_encoded_packet_can_follow_last_native_callback_by_one_frame() {
        // Installed Mac Stop receipt r1790475959802-43c845d19073: the file's
        // final PTS led the callback by 16.66ms, one encoded packet. Its encoded
        // packets and container had already passed the independent decode.
        let metadata = metadata("6.868037", "1/300000");
        let rows = b"0,5000\n2055411,5000\n";
        let mut native = native();
        native.start_pts_ns = 107_051_778_280_000;
        native.last_pts_ns = 107_058_612_990_000;
        native.last_cadence_ns = 16_670_000;
        native.last_duration_ns = 16_670_000;
        let verified = verify_probe(&metadata, &rows[..], native, 6_868, 2).unwrap();
        assert_eq!(verified.duration_ms, 6_868);

        // A second missing frame is not a valid one-packet Stop boundary.
        native.last_pts_ns -= 16_670_000;
        assert!(verify_probe(&metadata, &rows[..], native, 6_868, 2).is_err());
        // A callback beyond the one-tick encoded-end boundary is a lost tail.
        native.last_pts_ns = native.start_pts_ns + 6_868_040_000;
        // Exact packet end plus one rational tick is the existing boundary.
        assert!(verify_probe(&metadata, &rows[..], native, 6_868, 2).is_ok());
        native.last_pts_ns += 1;
        assert!(verify_probe(&metadata, &rows[..], native, 6_868, 2).is_err());
    }

    #[test]
    fn packet_duration_and_bad_or_truncated_output_fail_closed() {
        assert!(fixture("7.998060", 798410, 0, 2).is_err());
        assert!(fixture("7.990000", 798410, 833, 2).is_err());
        assert!(fixture("8.050000", 798410, 833, 2).is_err());
        assert!(fixture("7.998060", 798410, 833, 3).is_err());
        assert!(fixture("7.998060", 798000, 833, 2).is_err());
        assert!(verify_probe(b"{\"streams\":[", "0,833\n".as_bytes(), native(), 7_998, 2).is_err());
        assert!(verify_probe(
            &metadata("7.998060", "1/100000"),
            "0,833\n798410,".as_bytes(),
            native(),
            7_998,
            2
        )
        .is_err());
        assert!(packet_file_limit(0).is_err());
        assert!(packet_file_limit(u64::MAX).is_err());
    }

    #[test]
    fn movie_probe_budget_scales_with_verified_workload() {
        let short = probe_timeout(476, 64 * 1024 * 1024).unwrap();
        let thirty_minutes_4k = probe_timeout(54_000, 5 * 1024 * 1024 * 1024).unwrap();
        assert_eq!(short, Duration::from_secs(67));
        assert_eq!(thirty_minutes_4k, Duration::from_secs(410));
        assert!(thirty_minutes_4k > short);
        assert!(probe_timeout(0, 1).is_err());
        assert!(probe_timeout(1, 0).is_err());
        assert!(probe_timeout(u64::MAX, u64::MAX).is_err());
    }

    #[test]
    fn long_recording_streams_more_than_eight_megabytes_of_packets() {
        use std::io::{BufWriter, Seek, SeekFrom, Write};
        let mut file = tempfile::tempfile().unwrap();
        let frame_count = 1_000_000_u64;
        {
            let mut writer = BufWriter::new(&mut file);
            for pts in 0..frame_count {
                writeln!(writer, "{pts},1").unwrap();
            }
            writer.flush().unwrap();
        }
        assert!(file.metadata().unwrap().len() > 8 * 1024 * 1024);
        assert!(file.metadata().unwrap().len() <= packet_file_limit(frame_count).unwrap());
        file.seek(SeekFrom::Start(0)).unwrap();
        let native = NativeMovieTiming {
            start_pts_ns: 34_421_447_440_000,
            last_pts_ns: 34_421_447_440_000 + (frame_count - 1) * 1_000_000,
            last_duration_ns: 1_000_000,
            last_cadence_ns: 1_000_000,
            callback_count: 0,
        };
        let result = verify_probe(
            &metadata("1000.000000", "1/1000"),
            BufReader::new(&file),
            native,
            1_000_000,
            frame_count,
        )
        .unwrap();
        assert_eq!(result.duration_ms, 1_000_000);
    }
}

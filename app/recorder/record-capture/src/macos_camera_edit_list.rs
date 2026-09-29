//! Validate the presented MovieFileOutput range against its unedited packets.
//! ffprobe applies a movie edit list by shifting packet PTS and marking a
//! small number of B-frame dependency packets discard. The two packet views
//! must identify the same file bytes at one exact presentation offset.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use cut_media::ffmpeg::{run_owned_command, OwnedProcessControl};
use record_core::{error_codes, RecordError, Result};
use serde::Deserialize;

use crate::macos_camera_movie_timing::{NativeMovieTiming, VerifiedMovieTiming};

#[derive(Debug)]
struct Packet {
    pts: u64,
    duration: u64,
    pos: u64,
    discard: bool,
}

fn bad(cause: &str) -> RecordError {
    RecordError::new(
        error_codes::CAPTURE,
        "verify macOS camera movie clock",
        cause,
    )
}

#[derive(Deserialize)]
struct RawProbe {
    streams: Vec<RawStream>,
    format: RawFormat,
}

#[derive(Deserialize)]
struct RawStream {
    time_base: String,
    nb_read_frames: String,
    nb_read_packets: String,
}

#[derive(Deserialize)]
struct RawFormat {
    duration: String,
}

pub(super) fn verify_movie_edit_list(
    ffprobe: &str,
    path: &Path,
    native: NativeMovieTiming,
    presented_duration_ns: u64,
    presented_decoded_frames: u64,
    timeout: Duration,
) -> Result<VerifiedMovieTiming> {
    let mut metadata = Command::new(ffprobe);
    metadata
        .args([
            "-v",
            "error",
            "-ignore_editlist",
            "1",
            "-select_streams",
            "v:0",
            "-count_frames",
            "-count_packets",
            "-show_entries",
            "stream=time_base,nb_read_frames,nb_read_packets:format=duration",
            "-of",
            "json",
        ])
        .arg(path);
    let output = run_owned_command(
        &mut metadata,
        &OwnedProcessControl::bounded(timeout, || false).with_output_cap(64 * 1024),
        "probe unedited macOS camera movie",
    )
    .map_err(|cause| bad(&format!("unedited movie probe failed: {cause}")))?;
    if !output.status.success() || output.stdout.len() >= 64 * 1024 {
        return Err(bad(
            "unedited movie metadata probe failed or exceeded its bound",
        ));
    }
    let probe: RawProbe = serde_json::from_slice(&output.stdout)
        .map_err(|_| bad("unedited movie metadata is malformed"))?;
    let stream = probe
        .streams
        .first()
        .ok_or_else(|| bad("unedited video stream is missing"))?;
    let (num, den) = stream
        .time_base
        .split_once('/')
        .ok_or_else(|| bad("unedited time base is invalid"))?;
    let num: u64 = num
        .parse()
        .map_err(|_| bad("unedited time base is invalid"))?;
    let den: u64 = den
        .parse()
        .map_err(|_| bad("unedited time base is invalid"))?;
    if num == 0
        || den == 0
        || (1_000_000_000_u64
            .checked_mul(num)
            .ok_or_else(|| bad("unedited time base overflowed"))?
            % den)
            != 0
    {
        return Err(bad("unedited time base has a fractional nanosecond tick"));
    }
    let tick_ns = 1_000_000_000_u64 * num / den;
    let raw_duration_ns = parse_seconds_ns(&probe.format.duration)?;
    if !raw_duration_ns.is_multiple_of(tick_ns) || !presented_duration_ns.is_multiple_of(tick_ns) {
        return Err(bad("movie edit duration is not aligned to video ticks"));
    }
    let raw_frames: u64 = stream
        .nb_read_frames
        .parse()
        .map_err(|_| bad("unedited frame count is invalid"))?;
    let raw_packets: u64 = stream
        .nb_read_packets
        .parse()
        .map_err(|_| bad("unedited packet count is invalid"))?;
    if raw_frames == 0 || raw_frames != raw_packets || raw_frames <= presented_decoded_frames {
        return Err(bad("unedited movie decode and packet counts disagree"));
    }
    let raw_file = packet_proof(ffprobe, path, true, raw_frames, timeout)?;
    let edited_file = packet_proof(ffprobe, path, false, raw_frames, timeout)?;
    verify_edit_list(
        BufReader::new(raw_file.as_file()),
        BufReader::new(edited_file.as_file()),
        raw_duration_ns / tick_ns,
        presented_duration_ns / tick_ns,
        tick_ns,
        raw_frames,
        presented_decoded_frames,
        native,
    )
}

fn parse_seconds_ns(value: &str) -> Result<u64> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if fraction.len() > 9
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(bad("unedited duration is invalid"));
    }
    let seconds: u64 = whole
        .parse()
        .map_err(|_| bad("unedited duration is invalid"))?;
    let sub: u64 = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<u64>()
            .map_err(|_| bad("unedited duration is invalid"))?
            * 10_u64.pow((9 - fraction.len()) as u32)
    };
    seconds
        .checked_mul(1_000_000_000)
        .and_then(|n| n.checked_add(sub))
        .ok_or_else(|| bad("unedited duration overflows"))
}

fn packet_proof(
    ffprobe: &str,
    path: &Path,
    ignore_editlist: bool,
    raw_frames: u64,
    timeout: Duration,
) -> Result<tempfile::NamedTempFile> {
    let parent = path
        .parent()
        .ok_or_else(|| bad("camera staging parent is missing"))?;
    let file = tempfile::NamedTempFile::new_in(parent)
        .map_err(|cause| bad(&format!("reserve private edit-list proof: {cause}")))?;
    let before = file
        .as_file()
        .metadata()
        .map_err(|cause| bad(&format!("inspect private edit-list proof: {cause}")))?;
    if !before.is_file() || before.permissions().mode() & 0o777 != 0o600 {
        return Err(bad("private edit-list proof is not a plain 0600 file"));
    }
    let limit = raw_frames
        .checked_mul(128)
        .and_then(|v| v.checked_add(4096))
        .ok_or_else(|| bad("edit-list packet proof bound overflowed"))?;
    let mut command = Command::new(ffprobe);
    command
        .args([
            "-v",
            "error",
            "-ignore_editlist",
            if ignore_editlist { "1" } else { "0" },
            "-select_streams",
            "v:0",
            "-show_packets",
            "-show_entries",
            "packet=pts,duration,pos,flags",
            "-of",
            "csv=p=0",
            "-o",
        ])
        .arg(file.path())
        .arg(path);
    let growth_path = file.path().to_path_buf();
    let control = OwnedProcessControl::bounded(timeout, move || {
        fs::metadata(&growth_path).is_ok_and(|metadata| metadata.len() > limit)
    })
    .with_output_cap(64 * 1024);
    let output = run_owned_command(&mut command, &control, "probe macOS camera edit packets")
        .map_err(|cause| bad(&format!("edit-list packet probe failed: {cause}")))?;
    if !output.status.success() || !output.stdout.is_empty() {
        return Err(bad(
            "edit-list packet probe failed or wrote unexpected stdout",
        ));
    }
    let after = fs::symlink_metadata(file.path())
        .map_err(|cause| bad(&format!("inspect private edit-list proof: {cause}")))?;
    if !after.file_type().is_file()
        || after.dev() != before.dev()
        || after.ino() != before.ino()
        || after.len() > limit
    {
        return Err(bad(
            "edit-list packet proof was replaced or exceeded its bound",
        ));
    }
    Ok(file)
}

fn next_packet<R: BufRead>(rows: &mut R) -> Result<Option<Packet>> {
    let mut line = Vec::with_capacity(96);
    let read = (&mut *rows)
        .take(129)
        .read_until(b'\n', &mut line)
        .map_err(|cause| bad(&format!("read edit-list packet proof: {cause}")))?;
    if read == 0 {
        return Ok(None);
    }
    if read > 128 || line.last() != Some(&b'\n') {
        return Err(bad("edit-list packet row is oversized or incomplete"));
    }
    let text = std::str::from_utf8(&line[..line.len() - 1])
        .map_err(|_| bad("edit-list packet row is not UTF-8"))?;
    let mut fields = text.split(',');
    let number = |field: Option<&str>| -> Result<u64> {
        field
            .ok_or_else(|| bad("edit-list packet row is incomplete"))?
            .parse()
            .map_err(|_| bad("edit-list packet row has an invalid number"))
    };
    let pts = number(fields.next())?;
    let duration = number(fields.next())?;
    let pos = number(fields.next())?;
    let flags = fields
        .next()
        .ok_or_else(|| bad("edit-list packet flags are missing"))?;
    if fields.next().is_some() || flags.len() != 3 || !flags.is_ascii() || duration == 0 {
        return Err(bad("edit-list packet row is malformed"));
    }
    Ok(Some(Packet {
        pts,
        duration,
        pos,
        discard: flags.as_bytes()[1] == b'D',
    }))
}

#[expect(
    clippy::too_many_arguments,
    reason = "independent edit/decode/native proof inputs"
)]
pub(super) fn verify_edit_list<R: BufRead, E: BufRead>(
    mut raw_rows: R,
    mut edited_rows: E,
    raw_duration_ticks: u64,
    presented_duration_ticks: u64,
    tick_ns: u64,
    raw_decoded_frames: u64,
    presented_decoded_frames: u64,
    native: NativeMovieTiming,
) -> Result<VerifiedMovieTiming> {
    if raw_duration_ticks <= presented_duration_ticks
        || raw_decoded_frames <= presented_decoded_frames
        || tick_ns == 0
    {
        return Err(bad("MovieFileOutput edit bounds are absent or invalid"));
    }
    let mut raw_count = 0_u64;
    let mut edited_count = 0_u64;
    let mut visible_count = 0_u64;
    let mut raw = next_packet(&mut raw_rows)?;
    let mut raw_last_pos = None;
    let mut edited_last_pos = None;
    let mut offset = None::<u64>;
    let mut skipped_before_first_max_pts = None::<u64>;
    let mut last_visible_pts = 0_u64;
    let mut last_visible_duration = 0_u64;
    let mut first_visible = None;
    while let Some(edited) = next_packet(&mut edited_rows)? {
        edited_count += 1;
        if edited_count > raw_decoded_frames {
            return Err(bad("edited packet count exceeds the full movie"));
        }
        if edited_last_pos.is_some_and(|previous| edited.pos <= previous) {
            return Err(bad("edited packet file positions are not increasing"));
        }
        edited_last_pos = Some(edited.pos);
        loop {
            let source = raw
                .as_ref()
                .ok_or_else(|| bad("edited packet is absent from full movie"))?;
            if source.pos >= edited.pos {
                break;
            }
            if raw_last_pos.is_some_and(|previous| source.pos <= previous) {
                return Err(bad("full-movie packet file positions are not increasing"));
            }
            if let Some(edit_start) = offset {
                let edit_end = edit_start
                    .checked_add(presented_duration_ticks)
                    .ok_or_else(|| bad("edit-list end overflows ticks"))?;
                if source.pts >= edit_start && source.pts < edit_end {
                    return Err(bad("a packet inside the edit interval was skipped"));
                }
            } else {
                skipped_before_first_max_pts = Some(
                    skipped_before_first_max_pts.map_or(source.pts, |last| last.max(source.pts)),
                );
            }
            raw_last_pos = Some(source.pos);
            raw_count += 1;
            raw = next_packet(&mut raw_rows)?;
        }
        let source = raw
            .as_ref()
            .ok_or_else(|| bad("edited packet is absent from full movie"))?;
        if source.pos != edited.pos || source.duration != edited.duration {
            return Err(bad("edited packet identity differs from full movie"));
        }
        let delta = source
            .pts
            .checked_sub(edited.pts)
            .ok_or_else(|| bad("edited packet moves before full-movie time"))?;
        if delta == 0 || offset.is_some_and(|previous| previous != delta) {
            return Err(bad("edited packets do not share one media-time offset"));
        }
        if skipped_before_first_max_pts.is_some_and(|last| last >= delta) {
            return Err(bad("a skipped initial packet overlaps the edit interval"));
        }
        offset = Some(delta);
        if edited.discard {
            if edited.pts < presented_duration_ticks {
                return Err(bad("a displayed-time packet is marked discarded"));
            }
        } else {
            if edited.pts >= presented_duration_ticks {
                return Err(bad("visible packet begins outside the edit interval"));
            }
            first_visible.get_or_insert(edited.pts);
            if edited.pts > last_visible_pts {
                last_visible_pts = edited.pts;
                last_visible_duration = edited.duration;
            }
            visible_count += 1;
        }
        raw_last_pos = Some(source.pos);
        raw_count += 1;
        raw = next_packet(&mut raw_rows)?;
    }
    while let Some(source) = raw {
        if raw_last_pos.is_some_and(|previous| source.pos <= previous) {
            return Err(bad("full-movie packet file positions are not increasing"));
        }
        let edit_start = offset.ok_or_else(|| bad("edit-list has no packets"))?;
        let edit_end = edit_start
            .checked_add(presented_duration_ticks)
            .ok_or_else(|| bad("edit-list end overflows ticks"))?;
        if source.pts >= edit_start && source.pts < edit_end {
            return Err(bad("a packet inside the edit interval was skipped"));
        }
        raw_last_pos = Some(source.pos);
        raw_count += 1;
        raw = next_packet(&mut raw_rows)?;
    }
    if raw_count != raw_decoded_frames || visible_count != presented_decoded_frames {
        return Err(bad(
            "edit-list packet count disagrees with full or presented decode",
        ));
    }
    let offset = offset.ok_or_else(|| bad("edit-list has no packets"))?;
    if first_visible != Some(0)
        || offset >= raw_duration_ticks
        || presented_duration_ticks > raw_duration_ticks - offset
        || last_visible_duration == 0
        || presented_duration_ticks - last_visible_pts
            > last_visible_duration
                .checked_add(1)
                .ok_or_else(|| bad("last visible packet duration overflows"))?
    {
        return Err(bad(
            "edit-list presentation bounds disagree with movie packets",
        ));
    }
    let offset_ns = offset
        .checked_mul(tick_ns)
        .ok_or_else(|| bad("edit-list offset overflows nanoseconds"))?;
    let edited_last_ns = last_visible_pts
        .checked_mul(tick_ns)
        .ok_or_else(|| bad("last visible packet overflows nanoseconds"))?;
    if native.start_pts_ns == 0 || native.last_cadence_ns == 0 {
        return Err(bad(
            "MovieFileOutput start or final sample timing is missing",
        ));
    }
    let visible_duration_ns = last_visible_duration
        .checked_mul(tick_ns)
        .ok_or_else(|| bad("last visible packet duration overflows nanoseconds"))?;
    let final_duration_limit = native
        .last_cadence_ns
        .checked_mul(2)
        .and_then(|value| value.checked_add(tick_ns))
        .ok_or_else(|| bad("native cadence limit overflows"))?;
    if visible_duration_ns > final_duration_limit
        || (native.last_duration_ns != 0
            && visible_duration_ns
                > native
                    .last_duration_ns
                    .checked_mul(2)
                    .and_then(|value| value.checked_add(tick_ns))
                    .ok_or_else(|| bad("native sample duration limit overflows"))?)
    {
        return Err(bad("final visible packet exceeds native sample cadence"));
    }
    let native_elapsed = native
        .last_pts_ns
        .checked_sub(native.start_pts_ns)
        .ok_or_else(|| bad("MovieFileOutput callback clock is invalid"))?;
    let callback_after_edit = native_elapsed
        .checked_sub(offset_ns)
        .ok_or_else(|| bad("MovieFileOutput callback precedes edit start"))?;
    let cadence = native.last_cadence_ns;
    let callback_limit = cadence
        .checked_add(tick_ns)
        .ok_or_else(|| bad("native callback cadence limit overflows"))?;
    if callback_after_edit.abs_diff(edited_last_ns) > callback_limit {
        return Err(bad(
            "MovieFileOutput callback disagrees with edited final frame",
        ));
    }
    let presented_ns = presented_duration_ticks
        .checked_mul(tick_ns)
        .ok_or_else(|| bad("edit duration overflows nanoseconds"))?;
    Ok(VerifiedMovieTiming {
        duration_ms: presented_ns
            .checked_add(500_000)
            .ok_or_else(|| bad("edit duration rounding overflows"))?
            / 1_000_000,
        start_pts_ns: native
            .start_pts_ns
            .checked_add(offset_ns)
            .ok_or_else(|| bad("edited movie start overflows native clock"))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native() -> NativeMovieTiming {
        NativeMovieTiming {
            start_pts_ns: 100_000_000_000,
            last_pts_ns: 108_101_400_000,
            last_duration_ns: 16_660_000,
            last_cadence_ns: 16_670_000,
        }
    }

    #[test]
    fn exact_mac_edit_bounds_and_discard_dependencies_are_admitted() {
        let raw = b"96786,1666,100,K__\n808669,1666,200,___\n811784,1667,300,___\n815117,1649,400,___\n810135,1651,500,___\n";
        let edited = b"0,1666,100,K__\n711883,1666,200,___\n714998,1667,300,_D_\n718331,1649,400,_D_\n713349,1651,500,___\n";
        let verified = verify_edit_list(
            raw.as_slice(),
            edited.as_slice(),
            815102,
            713363,
            10_000,
            5,
            3,
            native(),
        )
        .unwrap();
        assert_eq!(verified.duration_ms, 7_134);
        assert_eq!(verified.start_pts_ns, 100_967_860_000);
    }

    fn three_dependency_packet_proof(
        raw: &str,
        edited: &str,
        raw_frames: u64,
        presented_frames: u64,
        timing: NativeMovieTiming,
    ) -> Result<VerifiedMovieTiming> {
        verify_edit_list(
            raw.as_bytes(),
            edited.as_bytes(),
            820000,
            713363,
            10_000,
            raw_frames,
            presented_frames,
            timing,
        )
    }

    #[test]
    fn three_discarded_packets_after_edit_end_are_admitted() {
        let raw = "96786,1666,100,K__\n808669,1666,200,___\n811784,1667,300,___\n815117,1649,400,___\n816784,1666,450,___\n810135,1651,500,___\n";
        let edited = "0,1666,100,K__\n711883,1666,200,___\n714998,1667,300,_D_\n718331,1649,400,_D_\n719998,1666,450,_D_\n713349,1651,500,___\n";
        let timing = three_dependency_packet_proof(raw, edited, 6, 3, native()).unwrap();
        assert_eq!(timing.duration_ms, 7_134);
        assert_eq!(timing.start_pts_ns, 100_967_860_000);
    }

    #[test]
    fn three_discard_proof_still_rejects_bad_flags_and_incomplete_or_wrong_packets() {
        let raw = "96786,1666,100,K__\n808669,1666,200,___\n811784,1667,300,___\n815117,1649,400,___\n816784,1666,450,___\n810135,1651,500,___\n";
        let edited = "0,1666,100,K__\n711883,1666,200,___\n714998,1667,300,_D_\n718331,1649,400,_D_\n719998,1666,450,_D_\n713349,1651,500,___\n";
        let outside_unflagged = edited.replace("719998,1666,450,_D_", "719998,1666,450,___");
        assert!(
            three_dependency_packet_proof(raw, &outside_unflagged, 6, 3, native())
                .unwrap_err()
                .cause
                .contains("visible packet begins outside")
        );
        let inside_discarded = edited.replace("711883,1666,200,___", "711883,1666,200,_D_");
        assert!(
            three_dependency_packet_proof(raw, &inside_discarded, 6, 3, native())
                .unwrap_err()
                .cause
                .contains("displayed-time packet is marked discarded")
        );
        let missing_visible = edited.replace("711883,1666,200,___\n", "");
        assert!(
            three_dependency_packet_proof(raw, &missing_visible, 6, 3, native())
                .unwrap_err()
                .cause
                .contains("inside the edit interval was skipped")
        );
        let missing_raw = raw.replace("810135,1651,500,___\n", "");
        assert!(
            three_dependency_packet_proof(&missing_raw, edited, 6, 3, native())
                .unwrap_err()
                .cause
                .contains("edited packet is absent from full movie")
        );
        let wrong_identity = edited.replace("719998,1666,450,_D_", "719998,1667,450,_D_");
        assert!(
            three_dependency_packet_proof(raw, &wrong_identity, 6, 3, native())
                .unwrap_err()
                .cause
                .contains("identity differs from full movie")
        );
        let mut late_callback = native();
        late_callback.last_pts_ns += 40_000_000;
        assert!(
            three_dependency_packet_proof(raw, edited, 6, 3, late_callback)
                .unwrap_err()
                .cause
                .contains("callback disagrees")
        );
    }

    #[test]
    fn missing_edit_or_outside_visible_frame_is_rejected() {
        let raw = b"0,1666,100,K__\n";
        let edited = b"0,1666,100,K__\n";
        assert!(verify_edit_list(
            raw.as_slice(),
            edited.as_slice(),
            1666,
            1666,
            10_000,
            1,
            1,
            native(),
        )
        .is_err());
        let raw = b"0,1666,50,K__\n96786,1666,100,K__\n810150,1666,200,___\n";
        let edited = b"0,1666,100,K__\n713364,1666,200,___\n";
        let error = verify_edit_list(
            raw.as_slice(),
            edited.as_slice(),
            815102,
            713363,
            10_000,
            3,
            2,
            native(),
        )
        .unwrap_err();
        assert!(error.cause.contains("visible packet begins outside"));
    }

    #[test]
    fn altered_packet_identity_and_unexplained_visible_tail_are_rejected() {
        let raw = b"0,1666,50,K__\n96786,1666,100,K__\n810135,1651,200,___\n";
        let wrong_offset = b"0,1666,100,K__\n713348,1651,200,___\n";
        let error = verify_edit_list(
            raw.as_slice(),
            wrong_offset.as_slice(),
            815102,
            713363,
            10_000,
            3,
            2,
            native(),
        )
        .unwrap_err();
        assert!(error.cause.contains("one media-time offset"));

        let extra_visible = b"0,1666,100,K__\n713349,1651,200,___\n";
        let error = verify_edit_list(
            raw.as_slice(),
            extra_visible.as_slice(),
            815102,
            713363,
            10_000,
            3,
            1,
            native(),
        )
        .unwrap_err();
        assert!(error.cause.contains("presented decode"));
    }

    #[test]
    fn truncated_full_decode_and_native_clock_mismatch_are_rejected() {
        let raw = b"0,1666,50,K__\n96786,1666,100,K__\n810135,1651,200,___\n";
        let edited = b"0,1666,100,K__\n713349,1651,200,___\n";
        let error = verify_edit_list(
            raw.as_slice(),
            edited.as_slice(),
            815102,
            713363,
            10_000,
            4,
            2,
            native(),
        )
        .unwrap_err();
        assert!(error.cause.contains("full or presented decode"));
        let mut late = native();
        late.last_pts_ns += 40_000_000;
        let error = verify_edit_list(
            raw.as_slice(),
            edited.as_slice(),
            815102,
            713363,
            10_000,
            3,
            2,
            late,
        )
        .unwrap_err();
        assert!(error.cause.contains("callback disagrees"));
    }

    #[test]
    fn packets_skipped_inside_the_presented_interval_are_rejected() {
        let raw = b"0,1666,50,K__\n96786,1666,100,K__\n500000,1666,150,___\n810135,1651,200,___\n";
        let edited = b"0,1666,100,K__\n713349,1651,200,___\n";
        let error = verify_edit_list(
            raw.as_slice(),
            edited.as_slice(),
            815102,
            713363,
            10_000,
            4,
            2,
            native(),
        )
        .unwrap_err();
        assert!(error.cause.contains("inside the edit interval was skipped"));

        let raw = b"0,1666,50,K__\n100000,1666,75,___\n96786,1666,100,K__\n810135,1651,200,___\n";
        let error = verify_edit_list(
            raw.as_slice(),
            edited.as_slice(),
            815102,
            713363,
            10_000,
            4,
            2,
            native(),
        )
        .unwrap_err();
        assert!(error.cause.contains("skipped initial packet overlaps"));
    }

    #[test]
    fn absent_or_incompatible_native_sample_timing_is_rejected() {
        let raw = b"0,1666,50,K__\n96786,1666,100,K__\n810135,1651,200,___\n";
        let edited = b"0,1666,100,K__\n713349,1651,200,___\n";
        let mut zero_start = native();
        zero_start.start_pts_ns = 0;
        let error = verify_edit_list(
            raw.as_slice(),
            edited.as_slice(),
            815102,
            713363,
            10_000,
            3,
            2,
            zero_start,
        )
        .unwrap_err();
        assert!(error
            .cause
            .contains("start or final sample timing is missing"));

        let mut short_cadence = native();
        short_cadence.last_cadence_ns = 1_000_000;
        short_cadence.last_duration_ns = 1_000_000;
        let error = verify_edit_list(
            raw.as_slice(),
            edited.as_slice(),
            815102,
            713363,
            10_000,
            3,
            2,
            short_cadence,
        )
        .unwrap_err();
        assert!(error.cause.contains("exceeds native sample cadence"));

        let mut overflow_cadence = native();
        overflow_cadence.last_cadence_ns = u64::MAX;
        let error = verify_edit_list(
            raw.as_slice(),
            edited.as_slice(),
            815102,
            713363,
            10_000,
            3,
            2,
            overflow_cadence,
        )
        .unwrap_err();
        assert!(error.cause.contains("native cadence limit overflows"));

        let large_duration = format!("0,1666,100,K__\n713349,{},200,___\n", u64::MAX);
        let raw_large_duration = format!(
            "0,1666,50,K__\n96786,1666,100,K__\n810135,{},200,___\n",
            u64::MAX
        );
        let error = verify_edit_list(
            raw_large_duration.as_bytes(),
            large_duration.as_bytes(),
            815102,
            713363,
            10_000,
            3,
            2,
            native(),
        )
        .unwrap_err();
        assert!(error
            .cause
            .contains("last visible packet duration overflows"));
    }

    #[test]
    fn retained_private_movie_can_prove_the_exact_edit_when_supplied() {
        let Ok(path) = std::env::var("SHELLX_CAMERA_EDIT_LIST_FIXTURE") else {
            return;
        };
        let timing = verify_movie_edit_list(
            "ffprobe",
            Path::new(&path),
            native(),
            7_133_630_000,
            429,
            Duration::from_secs(60),
        )
        .unwrap();
        assert_eq!(timing.duration_ms, 7_134);
        assert_eq!(timing.start_pts_ns, 100_967_860_000);
    }
}

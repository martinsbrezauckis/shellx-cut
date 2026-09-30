//! Validate the presented MovieFileOutput range against its unedited packets.
//! ffprobe applies a movie edit list by shifting packet PTS and marking a
//! small number of B-frame dependency packets discard. The two packet views
//! must identify the same file bytes at one exact presentation offset.

use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use cut_media::ffmpeg::{run_owned_command, OwnedProcessControl};
use record_core::{error_codes, RecordError, Result};
use serde::{Deserialize, Serialize};

use crate::macos_camera_clock::{DecimalDuration, TimeBase};
use crate::macos_camera_movie_timing::{
    retain_failed_metadata, retain_failed_probe, NativeMovieTiming, VerifiedMovieTiming,
};

const CLOCK_DIAGNOSTIC_PACKET_TAIL: usize = 6;

#[derive(Debug, Clone, Serialize)]
struct Packet {
    pts: u64,
    duration: u64,
    pos: u64,
    flags: String,
    discard: bool,
}

fn remember_packet(tail: &mut VecDeque<Packet>, packet: &Packet) {
    if tail.len() == CLOCK_DIAGNOSTIC_PACKET_TAIL {
        tail.pop_front();
    }
    tail.push_back(packet.clone());
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
    duration_ts: u64,
    nb_read_frames: String,
    nb_read_packets: String,
}

#[derive(Deserialize)]
struct RawFormat {
    duration: String,
}

#[expect(
    clippy::too_many_arguments,
    reason = "independent native, exact presented clock, decode and bounded probe inputs"
)]
pub(super) fn verify_movie_edit_list(
    ffprobe: &str,
    path: &Path,
    native: NativeMovieTiming,
    presented_time_base: &str,
    presented_duration_ticks: u64,
    presented_duration: &str,
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
            "stream=time_base,duration_ts,nb_read_frames,nb_read_packets:format=duration",
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
    let early = |failure: RecordError| {
        let parsed_clock = TimeBase::parse(&stream.time_base).ok();
        let diagnostic = serde_json::json!({
            "time_base_num": parsed_clock.map(|c| c.num),
            "time_base_den": parsed_clock.map(|c| c.den),
            "raw_time_base": stream.time_base.chars().take(80).collect::<String>(),
            "presented_time_base": presented_time_base.chars().take(80).collect::<String>(),
            "raw_duration_ticks": stream.duration_ts,
            "presented_duration_ticks": presented_duration_ticks,
            "raw_duration": probe.format.duration.chars().take(40).collect::<String>(),
            "presented_duration": presented_duration.chars().take(40).collect::<String>(),
            "raw_frames": stream.nb_read_frames.chars().take(24).collect::<String>(),
            "raw_packets": stream.nb_read_packets.chars().take(24).collect::<String>(),
            "raw_frame_count": stream.nb_read_frames.parse::<u64>().ok(),
            "raw_packet_count": stream.nb_read_packets.parse::<u64>().ok(),
            "presented_decoded_frames": presented_decoded_frames,
            "native": native,
        });
        bad(&format!("{}: {diagnostic}", failure.cause))
    };
    let clock = TimeBase::parse(&stream.time_base).map_err(&early)?;
    let presented_clock = TimeBase::parse(presented_time_base).map_err(&early)?;
    if u128::from(clock.num) * u128::from(presented_clock.den)
        != u128::from(presented_clock.num) * u128::from(clock.den)
    {
        return Err(early(bad("unedited and presented time bases disagree")));
    }
    if !DecimalDuration::parse(&probe.format.duration)
        .map_err(&early)?
        .matches(clock, stream.duration_ts)
        .map_err(&early)?
        || !DecimalDuration::parse(presented_duration)
            .map_err(&early)?
            .matches(clock, presented_duration_ticks)
            .map_err(&early)?
    {
        return Err(early(bad(
            "movie decimal duration disagrees with exact video ticks",
        )));
    }
    let raw_frames: u64 = stream
        .nb_read_frames
        .parse()
        .map_err(|_| early(bad("unedited frame count is invalid")))?;
    let raw_packets: u64 = stream
        .nb_read_packets
        .parse()
        .map_err(|_| early(bad("unedited packet count is invalid")))?;
    if raw_frames == 0 || raw_frames != raw_packets || raw_frames <= presented_decoded_frames {
        return Err(early(bad(
            "unedited movie decode and packet counts disagree",
        )));
    }
    let raw_file = packet_proof(ffprobe, path, true, raw_frames, timeout)?;
    let edited_file = packet_proof(ffprobe, path, false, raw_frames, timeout)?;
    verify_edit_list(
        BufReader::new(raw_file.as_file()),
        BufReader::new(edited_file.as_file()),
        stream.duration_ts,
        presented_duration_ticks,
        clock,
        raw_frames,
        presented_decoded_frames,
        native,
    )
    .map_err(|failure| {
        let failure = retain_failed_probe(raw_file, early(failure));
        let failure = retain_failed_probe(edited_file, failure);
        retain_failed_metadata(path, "camera-raw-metadata-", &output.stdout, failure)
    })
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
    let file = tempfile::Builder::new()
        .prefix(if ignore_editlist {
            "camera-raw-packets-"
        } else {
            "camera-edited-packets-"
        })
        .tempfile_in(parent)
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
        flags: flags.to_owned(),
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
    clock: TimeBase,
    raw_decoded_frames: u64,
    presented_decoded_frames: u64,
    native: NativeMovieTiming,
) -> Result<VerifiedMovieTiming> {
    if raw_duration_ticks <= presented_duration_ticks
        || raw_decoded_frames <= presented_decoded_frames
        || clock.num == 0
        || clock.den == 0
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
    let mut last_visible_packet = None::<Packet>;
    let mut first_visible = None;
    let mut raw_tail = VecDeque::with_capacity(CLOCK_DIAGNOSTIC_PACKET_TAIL);
    let mut edited_tail = VecDeque::with_capacity(CLOCK_DIAGNOSTIC_PACKET_TAIL);
    while let Some(edited) = next_packet(&mut edited_rows)? {
        remember_packet(&mut edited_tail, &edited);
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
            remember_packet(&mut raw_tail, source);
            raw_count += 1;
            raw = next_packet(&mut raw_rows)?;
        }
        let source = raw
            .as_ref()
            .ok_or_else(|| bad("edited packet is absent from full movie"))?;
        // An edit view can report stts durations by local demux index rather
        // than source sample index. Match the same file position and immutable
        // key/corruption flags; use the full view's duration for every bound.
        if source.pos != edited.pos
            || source.flags.as_bytes()[0] != edited.flags.as_bytes()[0]
            || source.flags.as_bytes()[2] != edited.flags.as_bytes()[2]
        {
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
                last_visible_duration = source.duration;
                last_visible_packet = Some(edited.clone());
            }
            visible_count += 1;
        }
        raw_last_pos = Some(source.pos);
        remember_packet(&mut raw_tail, source);
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
        remember_packet(&mut raw_tail, &source);
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
    let edited_last_ns = clock.floor_ns(last_visible_pts)?;
    let tick_scaled = clock.scaled(1)?;
    let tick_ns = clock.floor_ns(1)?;
    if native.start_pts_ns == 0 || native.last_cadence_ns == 0 {
        return Err(bad(
            "MovieFileOutput start or final sample timing is missing",
        ));
    }
    let visible_duration_ns = clock.floor_ns(last_visible_duration)?;
    let final_duration_limit = native
        .last_cadence_ns
        .checked_mul(2)
        .ok_or_else(|| bad("native cadence limit overflows"))?;
    let final_duration_limit_scaled = clock
        .ns_scaled(final_duration_limit)
        .checked_add(tick_scaled)
        .ok_or_else(|| bad("native cadence limit overflows"))?;
    let native_duration_limit_scaled = clock
        .ns_scaled(
            native
                .last_duration_ns
                .checked_mul(2)
                .ok_or_else(|| bad("native sample duration limit overflows"))?,
        )
        .checked_add(tick_scaled)
        .ok_or_else(|| bad("native sample duration limit overflows"))?;
    if clock.scaled(last_visible_duration)? > final_duration_limit_scaled
        || (native.last_duration_ns != 0
            && clock.scaled(last_visible_duration)? > native_duration_limit_scaled)
    {
        return Err(bad("final visible packet exceeds native sample cadence"));
    }
    let native_elapsed = native
        .last_pts_ns
        .checked_sub(native.start_pts_ns)
        .ok_or_else(|| bad("MovieFileOutput callback clock is invalid"))?;
    // A real writer completion is an independent upper fence for visible
    // images. The final frame's display duration can extend beyond closure;
    // its matched source duration and cadence were independently checked above.
    if native.stop_pts_ns != 0 || native.finish_clock_ns != 0 {
        native.verify_finish_bound(
            clock.scaled(last_visible_pts)?,
            clock.den,
            tick_scaled + u128::from(clock.den - 1),
        )?;
    }
    let native_elapsed_scaled = clock.ns_scaled(native_elapsed);
    // Apple's didStartRecording startPTS names the first written buffer in
    // AVCaptureSession.synchronizationClock. It is the native origin for
    // presented time zero. The raw/edited offset above maps encoded media
    // preroll to presentation time; applying it to startPTS maps it twice.
    // https://developer.apple.com/documentation/avfoundation/avcapturefileoutputrecordingdelegate/fileoutput(_:didstartrecordingto:startpts:from:)
    let callback_after_edit = native_elapsed;
    let callback_limit_scaled = clock
        .ns_scaled(native.last_cadence_ns)
        .checked_add(tick_scaled)
        .ok_or_else(|| bad("native callback cadence limit overflows"))?;
    let callback_limit = native
        .last_cadence_ns
        .checked_add(tick_ns)
        .ok_or_else(|| bad("native callback cadence limit overflows"))?;
    // CMTime timestamps are truncated independently to ns: the elapsed
    // subtraction differs from its exact value by less than one ns.
    let callback_delta_scaled = native_elapsed_scaled.abs_diff(clock.scaled(last_visible_pts)?);
    // Stop uses the completion fence above. Retain the original callback
    // relation for spontaneous/device-loss finishes without a Stop callback.
    let endpoint_limit_scaled = callback_limit_scaled;
    if native.stop_pts_ns == 0
        && callback_delta_scaled
            > endpoint_limit_scaled
                .checked_add(u128::from(clock.den - 1))
                .ok_or_else(|| bad("native callback cadence limit overflows"))?
    {
        // Full failed probe files and movie stay in capture staging; retain
        // bounded facts here for record.log and the terminal failure receipt.
        let diagnostic = serde_json::json!({
            "predicate": "abs(native_last_included_elapsed_ns - edited_last_ns) > endpoint_limit_ns",
            "coordinates": "startPTS is native presentation zero; raw edit offset maps packet views only",
            "endpoint_limit_ns": callback_limit,
            "native": {
                "start_pts_ns": native.start_pts_ns,
                "last_pts_ns": native.last_pts_ns,
                "last_duration_ns": native.last_duration_ns,
                "last_cadence_ns": native.last_cadence_ns,
                "callback_count": native.callback_count,
                "stop_pts_ns": native.stop_pts_ns,
                "stop_cadence_ns": native.stop_cadence_ns,
                "observed_last_pts_ns": native.observed_last_pts_ns,
                "stop_request_clock_ns": native.stop_request_clock_ns,
                "finish_clock_ns": native.finish_clock_ns,
                "elapsed_ns": native_elapsed,
            },
            "movie": {
                "time_base_ns_per_tick": tick_ns,
                "time_base_num": clock.num,
                "time_base_den": clock.den,
                "last_visible_duration_ns": visible_duration_ns,
                "last_visible_source_duration_ticks": last_visible_duration,
                "raw_duration_ticks": raw_duration_ticks,
                "presented_duration_ticks": presented_duration_ticks,
                "edit_offset_ticks": offset,
                "raw_decoded_frames": raw_decoded_frames,
                "raw_packet_count": raw_count,
                "presented_decoded_frames": presented_decoded_frames,
                "edited_packet_count": edited_count,
                "visible_packet_count": visible_count,
                "edited_last_ns": edited_last_ns,
                "callback_after_edit_ns": callback_after_edit,
                "callback_limit_ns": callback_limit,
                "callback_delta_ns": callback_after_edit.abs_diff(edited_last_ns),
                "last_visible_packet": last_visible_packet,
            },
            "raw_packet_tail": raw_tail,
            "edited_packet_tail": edited_tail,
        });
        return Err(bad(&format!(
            "MovieFileOutput callback disagrees with edited final frame: {diagnostic}"
        )));
    }
    Ok(VerifiedMovieTiming {
        duration_ms: clock.rounded_ms(presented_duration_ticks)?,
        start_pts_ns: native.start_pts_ns,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_pipeline_keeps_exact_ticks_and_early_numeric_failure_facts() {
        let dir = tempfile::tempdir().unwrap();
        // Execute a checked-in immutable fixture: a concurrently spawned test
        // child must never inherit a writable handle to this executable inode.
        let probe =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/mac_camera_exact_probe.sh");
        let mut timing = native();
        timing.last_pts_ns = timing.start_pts_ns + 5_000_000;
        timing.last_cadence_ns = 1_666_667;
        timing.last_duration_ns = 1_666_667;
        let movie = dir.path().join("camera.mp4");
        let result = verify_movie_edit_list(
            probe.to_str().unwrap(),
            &movie,
            timing,
            "1/600",
            3,
            "0.005000",
            3,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(result.duration_ms, 5);
        assert_eq!(result.start_pts_ns, timing.start_pts_ns);
        let failure = verify_movie_edit_list(
            probe.to_str().unwrap(),
            &movie,
            timing,
            "1/600",
            3,
            "0.005001",
            3,
            Duration::from_secs(5),
        )
        .unwrap_err();
        let json = failure
            .cause
            .strip_prefix("movie decimal duration disagrees with exact video ticks: ")
            .unwrap();
        let facts: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(facts["time_base_num"], 1);
        assert_eq!(facts["time_base_den"], 600);
        assert_eq!(facts["raw_duration_ticks"], 5);
        assert_eq!(facts["presented_duration_ticks"], 3);
        assert_eq!(facts["native"]["callback_count"], 5);
        assert!(json.len() < 2_048);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        // A complete packet proof rejected at the native endpoint must remain
        // available in the same owned staging directory, with the movie.
        fs::write(&movie, b"owned synthetic rejected movie").unwrap();
        timing.last_pts_ns += 10_000_000;
        let failure = verify_movie_edit_list(
            probe.to_str().unwrap(),
            &movie,
            timing,
            "1/600",
            3,
            "0.005000",
            3,
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(failure.cause.contains("callback disagrees"));
        assert!(failure.cause.contains("retained probe"));
        let files: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(files.len(), 4); // movie, raw/edited rows, raw metadata
        for (prefix, expected) in [
            (
                "camera-raw-packets-",
                "0,1,50,K__\n1,1,100,K__\n2,1,200,___\n3,1,300,___\n4,1,400,___\n",
            ),
            (
                "camera-edited-packets-",
                "0,1,100,K__\n1,1,200,___\n2,1,300,___\n3,1,400,_D_\n",
            ),
        ] {
            let path = files
                .iter()
                .find(|path| {
                    path.file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .starts_with(prefix)
                })
                .unwrap();
            assert_eq!(fs::read_to_string(path).unwrap(), expected);
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let raw_metadata = files
            .iter()
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("camera-raw-metadata-")
            })
            .unwrap();
        let facts: serde_json::Value =
            serde_json::from_slice(&fs::read(raw_metadata).unwrap()).unwrap();
        assert_eq!(facts["streams"][0]["nb_read_packets"], "5");
        assert_eq!(fs::read(&movie).unwrap(), b"owned synthetic rejected movie");
    }

    #[test]
    fn fractional_nanosecond_ticks_preserve_exact_edit_and_packet_identity() {
        // Synthetic clocks, not the unretained native e126 time base.
        for den in [600, 24, 3_000_000_000] {
            let clock = TimeBase { num: 1, den };
            let raw = "0,1,50,K__\n1,1,100,K__\n2,1,200,___\n3,1,300,___\n";
            let edited = "0,1,100,K__\n1,1,200,___\n2,1,300,_D_\n";
            let mut timing = native();
            timing.last_pts_ns = timing.start_pts_ns + clock.floor_ns(2).unwrap();
            timing.last_cadence_ns = clock.floor_ns(1).unwrap().max(1);
            timing.last_duration_ns = timing.last_cadence_ns;
            let result =
                verify_edit_list(raw.as_bytes(), edited.as_bytes(), 4, 2, clock, 4, 2, timing)
                    .unwrap();
            assert_eq!(result.start_pts_ns, timing.start_pts_ns);
            assert_eq!(result.duration_ms, clock.rounded_ms(2).unwrap());
            for (raw_rows, edited_rows, raw_count, visible_count) in [
                (raw.to_owned(), edited.replace("1,1,200", "0,1,200"), 4, 2),
                (raw.to_owned(), edited.replace("1,1,200", "1,1,201"), 4, 2),
                (raw.to_owned(), edited.to_owned(), 5, 2),
                (raw.to_owned(), edited.to_owned(), 4, 3),
                (raw.replace("2,1,200", "2,1,99"), edited.to_owned(), 4, 2),
            ] {
                assert!(verify_edit_list(
                    raw_rows.as_bytes(),
                    edited_rows.as_bytes(),
                    4,
                    2,
                    clock,
                    raw_count,
                    visible_count,
                    timing
                )
                .is_err());
            }
            timing.last_pts_ns += timing.last_cadence_ns * 3 + 2;
            assert!(
                verify_edit_list(raw.as_bytes(), edited.as_bytes(), 4, 2, clock, 4, 2, timing)
                    .unwrap_err()
                    .cause
                    .contains("callback disagrees")
            );
        }
    }

    fn native() -> NativeMovieTiming {
        NativeMovieTiming {
            start_pts_ns: 100_000_000_000,
            last_pts_ns: 107_133_490_000,
            last_duration_ns: 16_660_000,
            last_cadence_ns: 16_670_000,
            callback_count: 5,
            ..NativeMovieTiming::default()
        }
    }

    #[test]
    fn nonzero_preroll_keeps_native_presentation_origin_and_frozen_stop_endpoint() {
        // Synthetic full/edited movie views with 949.45ms compression preroll.
        // The native first-written timestamp already names presentation zero.
        let raw = "0,5000,50,K__\n284836,5000,100,K__\n299836,5001,200,___\n304837,5000,300,_D_\n";
        let edited = "0,5000,100,K__\n15000,5001,200,___\n20001,5000,300,_D_\n";
        let clock = TimeBase {
            num: 1,
            den: 300_000,
        };
        let mut timing = NativeMovieTiming {
            start_pts_ns: 100_000_000_000,
            last_pts_ns: 100_050_000_000,
            last_duration_ns: 16_670_000,
            // Natural VFR: prior interval and next excluded interval differ.
            last_cadence_ns: 16_540_001,
            stop_pts_ns: 100_066_670_000,
            stop_cadence_ns: 16_670_000,
            // Arbitrarily later callback is diagnostic, never the endpoint.
            observed_last_pts_ns: 100_150_000_000,
            callback_count: 15,
            stop_request_clock_ns: 100_060_000_000,
            finish_clock_ns: 100_120_000_000,
        };
        let verify = |native| {
            verify_edit_list(
                raw.as_bytes(),
                edited.as_bytes(),
                309837,
                20001,
                clock,
                4,
                2,
                native,
            )
        };
        let verified = verify(timing).unwrap();
        assert_eq!(verified.start_pts_ns, timing.start_pts_ns);
        assert_eq!(verified.duration_ms, 67);
        timing.last_pts_ns = timing.observed_last_pts_ns;
        assert!(verify(timing)
            .unwrap_err()
            .cause
            .contains("completion clock precedes"));
        timing.last_pts_ns = 100_050_000_000;
        timing.stop_pts_ns += 33_333_333;
        timing.stop_cadence_ns += 33_333_333;
        assert!(verify(timing).is_ok());
        let missing_visible = edited.replace("15000,5001,200,___\n", "");
        assert!(verify_edit_list(
            raw.as_bytes(),
            missing_visible.as_bytes(),
            309837,
            20001,
            clock,
            4,
            1,
            timing
        )
        .unwrap_err()
        .cause
        .contains("inside the edit interval was skipped"));
        timing.stop_cadence_ns += 1;
        assert!(verify(timing)
            .unwrap_err()
            .cause
            .contains("Stop boundary does not follow"));
        timing.stop_pts_ns = timing.last_pts_ns;
        assert!(verify(timing)
            .unwrap_err()
            .cause
            .contains("Stop boundary does not follow"));
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
            TimeBase {
                num: 1,
                den: 100_000,
            },
            5,
            3,
            native(),
        )
        .unwrap();
        assert_eq!(verified.duration_ms, 7_134);
        assert_eq!(verified.start_pts_ns, 100_000_000_000);
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
            TimeBase {
                num: 1,
                den: 100_000,
            },
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
        assert_eq!(timing.start_pts_ns, 100_000_000_000);
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
        let wrong_identity = edited.replace("719998,1666,450,_D_", "719998,1666,450,KD_");
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
            TimeBase {
                num: 1,
                den: 100_000
            },
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
            TimeBase {
                num: 1,
                den: 100_000,
            },
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
            TimeBase {
                num: 1,
                den: 100_000,
            },
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
            TimeBase {
                num: 1,
                den: 100_000,
            },
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
            TimeBase {
                num: 1,
                den: 100_000,
            },
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
            TimeBase {
                num: 1,
                den: 100_000,
            },
            3,
            2,
            late,
        )
        .unwrap_err();
        assert!(error.cause.contains("callback disagrees"));
    }

    #[test]
    fn rejected_callback_clock_retains_bounded_movie_and_packet_facts() {
        let raw = b"96786,1666,100,K__\n808669,1666,200,___\n811784,1667,300,___\n815117,1649,400,___\n810135,1651,500,___\n";
        let edited = b"0,1666,100,K__\n711883,1666,200,___\n714998,1667,300,_D_\n718331,1649,400,_D_\n713349,1651,500,___\n";
        let mut late = native();
        late.last_pts_ns += 40_000_000;
        let error = verify_edit_list(
            raw.as_slice(),
            edited.as_slice(),
            815102,
            713363,
            TimeBase {
                num: 1,
                den: 100_000,
            },
            5,
            3,
            late,
        )
        .unwrap_err();
        let detail = error
            .cause
            .strip_prefix("MovieFileOutput callback disagrees with edited final frame: ")
            .expect("the clock mismatch still rejects the movie");
        let facts: serde_json::Value = serde_json::from_str(detail).unwrap();
        assert_eq!(facts["native"]["start_pts_ns"], 100_000_000_000_u64);
        assert_eq!(facts["native"]["last_pts_ns"], late.last_pts_ns);
        assert_eq!(facts["native"]["callback_count"], 5);
        assert_eq!(facts["movie"]["time_base_ns_per_tick"], 10_000);
        assert_eq!(facts["movie"]["edit_offset_ticks"], 96786);
        assert_eq!(facts["movie"]["raw_packet_count"], 5);
        assert_eq!(facts["movie"]["presented_decoded_frames"], 3);
        assert_eq!(facts["movie"]["last_visible_packet"]["pos"], 500);
        assert_eq!(facts["edited_packet_tail"][2]["flags"], "_D_");
        assert_eq!(facts["raw_packet_tail"].as_array().unwrap().len(), 5);
        assert!(
            facts["movie"]["callback_delta_ns"].as_u64().unwrap()
                > facts["movie"]["callback_limit_ns"].as_u64().unwrap()
        );
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
            TimeBase {
                num: 1,
                den: 100_000,
            },
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
            TimeBase {
                num: 1,
                den: 100_000,
            },
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
            TimeBase {
                num: 1,
                den: 100_000,
            },
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
            TimeBase {
                num: 1,
                den: 100_000,
            },
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
            TimeBase {
                num: 1,
                den: 100_000,
            },
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
            TimeBase {
                num: 1,
                den: 100_000,
            },
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
    #[ignore = "requires a retained real movie via SHELLX_CAMERA_EDIT_LIST_FIXTURE"]
    fn retained_private_movie_can_prove_the_exact_edit_when_supplied() {
        let path = std::env::var("SHELLX_CAMERA_EDIT_LIST_FIXTURE")
            .expect("a retained real movie must be supplied explicitly");
        let timing = verify_movie_edit_list(
            "ffprobe",
            Path::new(&path),
            native(),
            "1/100000",
            713363,
            "7.133630",
            429,
            Duration::from_secs(60),
        )
        .unwrap();
        assert_eq!(timing.duration_ms, 7_134);
        assert_eq!(timing.start_pts_ns, 100_000_000_000);
    }
}

#[cfg(test)]
#[path = "macos_camera_stop_regression.rs"]
mod retained_stop_tests;

//! Synthetic picture and audio reference clocks through native checkpoint stitching.

use super::super::{clock_now_ns, NativeSegment};
use super::frame;
use crate::{checkpoint::Checkpoints, CheckpointConfig};
use record_recovery::{CaptureStart, CheckpointFacts, ManifestOwner};
use std::{f64::consts::TAU, path::Path, process::Command};

const AUDIO_HZ: u32 = 48_000;
const FLASH_SECOND: u32 = 15;
const TAKE_SECONDS: u32 = 16;

fn tool(name: &str, fallback: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fallback.into())
}

fn matching_wave(path: &Path) -> usize {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: AUDIO_HZ,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut wave = hound::WavWriter::create(path, spec).unwrap();
    let first = FLASH_SECOND * AUDIO_HZ;
    let last = first + AUDIO_HZ / 100;
    for sample in 0..TAKE_SECONDS * AUDIO_HZ {
        let value = if (first..last).contains(&sample) {
            let phase = f64::from(sample - first) * TAU * 1_000.0 / f64::from(AUDIO_HZ);
            (phase.sin() * 12_000.0).round() as i16
        } else {
            0
        };
        wave.write_sample(value).unwrap();
    }
    wave.finalize().unwrap();
    let mut reader = hound::WavReader::open(path).unwrap();
    reader
        .samples::<i16>()
        .position(|sample| sample.unwrap() != 0)
        .expect("generated beep must have a nonzero sample")
}

fn decoded_gray_frames(source: &Path, ffmpeg: &str) -> Vec<u8> {
    let output = Command::new(ffmpeg)
        .args(["-v", "error", "-i"])
        .arg(source)
        .args([
            "-an",
            "-sn",
            "-dn",
            "-vf",
            "scale=1:1:flags=area,format=gray",
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "gray",
            "-",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "synthetic reference decode failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn synthetic_beep_and_picture_flash_share_the_checkpoint_seam_at_30_and_60_fps() {
    let ffmpeg = tool("SHELLX_RECORD_FFMPEG", "ffmpeg");
    let ffprobe = tool("SHELLX_RECORD_FFPROBE", "ffprobe");
    for fps in [30_u32, 60] {
        let dir = tempfile::tempdir().unwrap();
        let mut start = CaptureStart::new(format!("synthetic-av-{fps}"), 15_000);
        start.output_cadence = Some(record_core::FrameRate::new(u64::from(fps), 1).unwrap());
        ManifestOwner::begin(dir.path(), start).unwrap();
        let config = CheckpointConfig {
            manifest_dir: dir.path().to_string_lossy().into_owned(),
            interval_ms: 15_000,
        };
        let mut checkpoints = Checkpoints::open(Some(&config)).unwrap().unwrap();
        let base = clock_now_ns();
        for (sequence, first_frame, end_frame) in
            [(0_u64, 0_u32, 15 * fps), (1_u64, 15 * fps, 16 * fps)]
        {
            let start_ms = u64::from(first_frame) * 1_000 / u64::from(fps);
            let end_ms = u64::from(end_frame) * 1_000 / u64::from(fps);
            let (reserved_sequence, staging) = checkpoints.begin(start_ms).unwrap();
            assert_eq!(reserved_sequence, sequence);
            let writer = NativeSegment::new(&staging, 256, 144, fps).unwrap();
            for number in first_frame..end_frame {
                let pts = base + i64::from(number) * 1_000_000_000 / i64::from(fps);
                let color = if number == FLASH_SECOND * fps {
                    65_535
                } else {
                    0
                };
                writer.append(&frame(pts, fps, color)).unwrap();
            }
            let end_pts = base + i64::from(end_frame) * 1_000_000_000 / i64::from(fps);
            writer.finish(end_pts).unwrap();
            assert_eq!(writer.stats().2, u64::from(end_frame - first_frame));
            checkpoints
                .publish_without_decode(
                    sequence,
                    &staging,
                    CheckpointFacts {
                        start_ms,
                        end_ms,
                        event_offset_ms: start_ms,
                        audio_offset_ms: None,
                    },
                )
                .unwrap();
        }
        let (source, media, notes) = checkpoints.stitch(&ffmpeg, &ffprobe, "source.mp4").unwrap();
        assert!(notes.is_empty(), "{fps} fps: {notes:?}");
        assert_eq!(media.decoded_video_frames, u64::from(TAKE_SECONDS * fps));
        let gray = decoded_gray_frames(&source, &ffmpeg);
        assert_eq!(gray.len(), (TAKE_SECONDS * fps) as usize);
        let bright_frames: Vec<_> = gray
            .iter()
            .enumerate()
            .filter_map(|(index, luma)| (*luma > 180).then_some(index))
            .collect();
        assert_eq!(
            bright_frames,
            vec![(FLASH_SECOND * fps) as usize],
            "{fps} fps: flash moved or disappeared at the checkpoint seam"
        );
        let first_audio = matching_wave(&dir.path().join("synthetic-beep.wav"));
        let flash_ms = bright_frames[0] as f64 * 1_000.0 / f64::from(fps);
        let beep_ms = first_audio as f64 * 1_000.0 / f64::from(AUDIO_HZ);
        let offset_ms = (flash_ms - beep_ms).abs();
        assert!(
            offset_ms <= 10.0,
            "{fps} fps: synthetic A/V reference offset {offset_ms:.3} ms"
        );
        println!(
            "synthetic {fps} fps: {} decoded frames, flash {flash_ms:.3} ms, beep {beep_ms:.3} ms, offset {offset_ms:.3} ms",
            gray.len()
        );
    }
}

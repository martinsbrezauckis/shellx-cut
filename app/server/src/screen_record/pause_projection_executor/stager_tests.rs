use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use record_core::FrameRate;
use record_recovery::{
    ExactFrameGridDuration, FrameGridStitchPlan, FrameGridStitchSpan, RunAwareStitchSpan,
};
use sha2::{Digest, Sha256};
use tempfile::tempdir;

use super::super::types::{
    FfmpegPauseProjectionArtifactVerifier, PauseProjectionArtifactVerifier,
    VerifiedPauseProjectionSource,
};
use super::{
    filter_graph, FfmpegFrameGridPauseProjectionSourceStager, FrameGridPauseProjectionSourceStager,
};

fn rate() -> FrameRate {
    FrameRate::new(30, 1).unwrap()
}

fn source(
    path: PathBuf,
    run_sequence: u64,
    checkpoint_sequence: u64,
) -> VerifiedPauseProjectionSource {
    let bytes = fs::read(&path).unwrap();
    VerifiedPauseProjectionSource {
        run_sequence,
        checkpoint_sequence,
        path,
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        duration_ms: 100,
        decoded_video_frames: 3,
        avg_frame_rate: Some(rate()),
        r_frame_rate: Some(rate()),
    }
}

fn source_span(run_sequence: u64, checkpoint_sequence: u64, offset_ms: u64) -> RunAwareStitchSpan {
    RunAwareStitchSpan::Source {
        run_sequence,
        checkpoint_sequence,
        artifact: format!("screen/run-{run_sequence}-{checkpoint_sequence}.mp4"),
        sha256: "a".repeat(64),
        logical_offset_ms: offset_ms,
        source_duration_ms: 100,
    }
}

fn grid() -> FrameGridStitchPlan {
    FrameGridStitchPlan {
        output_frame_rate: rate(),
        output_frame_count: 6,
        expected_duration_ms: ExactFrameGridDuration {
            numerator: 200,
            denominator: 1,
        },
        spans: vec![
            FrameGridStitchSpan {
                span: source_span(0, 0, 0),
                start_frame: 0,
                end_frame: 3,
            },
            FrameGridStitchSpan {
                span: source_span(1, 1, 100),
                start_frame: 3,
                end_frame: 6,
            },
        ],
    }
}

fn grid_with_middle_padding() -> FrameGridStitchPlan {
    FrameGridStitchPlan {
        output_frame_rate: rate(),
        output_frame_count: 9,
        expected_duration_ms: ExactFrameGridDuration {
            numerator: 300,
            denominator: 1,
        },
        spans: vec![
            FrameGridStitchSpan {
                span: source_span(0, 0, 0),
                start_frame: 0,
                end_frame: 3,
            },
            FrameGridStitchSpan {
                span: RunAwareStitchSpan::EncoderGapPadding {
                    run_sequence: 0,
                    logical_start_ms: 100,
                    logical_end_ms: 200,
                },
                start_frame: 3,
                end_frame: 6,
            },
            FrameGridStitchSpan {
                span: source_span(0, 1, 200),
                start_frame: 6,
                end_frame: 9,
            },
        ],
    }
}

fn grid_with_outer_padding() -> FrameGridStitchPlan {
    FrameGridStitchPlan {
        output_frame_rate: rate(),
        output_frame_count: 9,
        expected_duration_ms: ExactFrameGridDuration {
            numerator: 300,
            denominator: 1,
        },
        spans: vec![
            FrameGridStitchSpan {
                span: RunAwareStitchSpan::EncoderGapPadding {
                    run_sequence: 0,
                    logical_start_ms: 0,
                    logical_end_ms: 100,
                },
                start_frame: 0,
                end_frame: 3,
            },
            FrameGridStitchSpan {
                span: source_span(0, 0, 100),
                start_frame: 3,
                end_frame: 6,
            },
            FrameGridStitchSpan {
                span: RunAwareStitchSpan::EncoderGapPadding {
                    run_sequence: 0,
                    logical_start_ms: 200,
                    logical_end_ms: 300,
                },
                start_frame: 6,
                end_frame: 9,
            },
        ],
    }
}

fn make_source(path: &Path, color: &str) {
    let status = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s=16x16:r=30"),
            "-frames:v",
            "3",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-r",
            "30",
            "-y",
        ])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

fn decoded_pixel(path: &Path, frame: usize) -> [u8; 3] {
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let offset = frame * 16 * 16 * 3;
    output.stdout[offset..offset + 3].try_into().unwrap()
}

#[test]
fn assembles_only_compact_grid_frames_from_private_snapshots() {
    let root = tempdir().unwrap();
    let first = root.path().join("first.mp4");
    let second = root.path().join("second.mp4");
    let output = root.path().join("source-stage.mp4");
    make_source(&first, "red");
    make_source(&second, "blue");
    let sources = vec![source(first, 0, 0), source(second, 1, 1)];

    FfmpegFrameGridPauseProjectionSourceStager::new("ffmpeg", "ffprobe")
        .stage_frame_grid(root.path(), &sources, &grid(), &output)
        .unwrap();

    let facts = FfmpegPauseProjectionArtifactVerifier::new("ffmpeg", "ffprobe")
        .verify(&output)
        .unwrap();
    assert_eq!(facts.decoded_video_frames, 6);
    assert_eq!(facts.duration_ms, 200);
    assert_eq!(facts.avg_frame_rate, Some(rate()));
    assert_eq!(facts.r_frame_rate, Some(rate()));
    assert!(!facts.has_audio);
    assert!(
        !fs::read_dir(root.path())
            .unwrap()
            .flatten()
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .contains("pause-source-input")),
        "input snapshots are private and removed after staging"
    );
}

#[test]
fn assigns_an_in_run_gap_once_and_keeps_later_source_frames() {
    let root = tempdir().unwrap();
    let first = root.path().join("first.mp4");
    let second = root.path().join("second.mp4");
    let output = root.path().join("source-stage.mp4");
    make_source(&first, "red");
    make_source(&second, "blue");
    let sources = vec![source(first, 0, 0), source(second, 0, 1)];
    let plan = grid_with_middle_padding();

    let filter = filter_graph(&sources, &plan).unwrap();
    assert!(filter
        .contains("[0:v]setpts=PTS-STARTPTS,fps=fps=30/1:round=near,tpad=stop_mode=clone:stop=-1,trim=end_frame=3,setpts=N/(30/1*TB),tpad=start_mode=clone:start=0:stop_mode=clone:stop=3"));
    assert!(filter
        .contains("[1:v]setpts=PTS-STARTPTS,fps=fps=30/1:round=near,tpad=stop_mode=clone:stop=-1,trim=end_frame=3,setpts=N/(30/1*TB),tpad=start_mode=clone:start=0:stop_mode=clone:stop=0"));
    assert!(filter.contains("[s0][s1]concat=n=2:v=1:a=0"));

    FfmpegFrameGridPauseProjectionSourceStager::new("ffmpeg", "ffprobe")
        .stage_frame_grid(root.path(), &sources, &plan, &output)
        .unwrap();
    let facts = FfmpegPauseProjectionArtifactVerifier::new("ffmpeg", "ffprobe")
        .verify(&output)
        .unwrap();
    assert_eq!(facts.decoded_video_frames, 9);
    assert!(decoded_pixel(&output, 5)[0] > 200);
    assert!(decoded_pixel(&output, 6)[2] > 200);
}

#[test]
fn assigns_outer_padding_only_to_the_first_and_last_source() {
    let root = tempdir().unwrap();
    let path = root.path().join("first.mp4");
    fs::write(&path, b"first").unwrap();
    let source = source(path, 0, 0);
    let filter = filter_graph(&[source], &grid_with_outer_padding()).unwrap();
    assert!(filter.contains("tpad=start_mode=clone:start=3:stop_mode=clone:stop=3"));
}

#[test]
fn refuses_padding_that_crosses_a_sealed_run_boundary() {
    let root = tempdir().unwrap();
    let path = root.path().join("first.mp4");
    fs::write(&path, b"first").unwrap();
    let source = source(path, 0, 0);
    let mut plan = grid_with_outer_padding();
    plan.spans[0].span = RunAwareStitchSpan::EncoderGapPadding {
        run_sequence: 1,
        logical_start_ms: 0,
        logical_end_ms: 100,
    };
    assert!(filter_graph(&[source], &plan).is_err());
}

#[test]
fn rejects_hash_drift_before_ffmpeg_can_receive_the_source() {
    let root = tempdir().unwrap();
    let first = root.path().join("first.mp4");
    let second = root.path().join("second.mp4");
    let output = root.path().join("source-stage.mp4");
    make_source(&first, "red");
    make_source(&second, "blue");
    let sources = vec![source(first.clone(), 0, 0), source(second, 1, 1)];
    fs::write(first, b"not the sealed source").unwrap();

    assert!(
        FfmpegFrameGridPauseProjectionSourceStager::new("ffmpeg", "ffprobe")
            .stage_frame_grid(root.path(), &sources, &grid(), &output)
            .is_err()
    );
    assert!(!output.exists());
}

#[test]
fn rejects_frame_or_cadence_drift_without_constructing_an_ffmpeg_stage() {
    let root = tempdir().unwrap();
    let first = root.path().join("first.mp4");
    let second = root.path().join("second.mp4");
    fs::write(&first, b"first").unwrap();
    fs::write(&second, b"second").unwrap();
    let mut sources = vec![source(first, 0, 0), source(second, 1, 1)];
    sources[1].decoded_video_frames = 0;
    assert!(filter_graph(&sources, &grid()).is_err());

    sources[1].decoded_video_frames = 3;
    sources[1].avg_frame_rate = None;
    assert!(filter_graph(&sources, &grid()).is_err());
}

#[cfg(unix)]
#[test]
fn rejects_a_path_replaced_with_a_link_before_snapshotting() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let first = root.path().join("first.mp4");
    let second = root.path().join("second.mp4");
    let output = root.path().join("source-stage.mp4");
    make_source(&first, "red");
    make_source(&second, "blue");
    let sources = vec![source(first.clone(), 0, 0), source(second, 1, 1)];
    let target = outside.path().join("external.mp4");
    fs::write(&target, fs::read(&first).unwrap()).unwrap();
    fs::remove_file(&first).unwrap();
    symlink(target, first).unwrap();

    assert!(
        FfmpegFrameGridPauseProjectionSourceStager::new("ffmpeg", "ffprobe")
            .stage_frame_grid(root.path(), &sources, &grid(), &output)
            .is_err()
    );
    assert!(!output.exists());
}

#[test]
fn normalizes_vfr_before_five_explicit_padding_frames() {
    let root = tempdir().unwrap();
    let input = root.path().join("vfr.mp4");
    if let Some(retained) = std::env::var_os("CUT_PAUSE_RETAINED_VFR_SOURCE") {
        fs::copy(retained, &input).unwrap();
    } else {
        assert!(Command::new("ffmpeg").args([
            "-v", "error", "-f", "lavfi", "-i", "testsrc2=s=16x16:r=24",
            "-frames:v", "5", "-vf",
            r"settb=1/600,setpts=if(eq(N\,0)\,0\,if(eq(N\,1)\,30\,if(eq(N\,2)\,50\,if(eq(N\,3)\,80\,110))))",
            "-fps_mode", "vfr", "-enc_time_base", "1/600", "-c:v", "libx264", "-video_track_timescale", "600",
        ]).arg(&input).status().unwrap().success());
    }
    let verifier = FfmpegPauseProjectionArtifactVerifier::new("ffmpeg", "ffprobe");
    let facts = verifier.verify(&input).unwrap();
    assert_eq!(facts.decoded_video_frames, 5);
    let mut source = source(input, 0, 0);
    source.duration_ms = facts.duration_ms;
    source.decoded_video_frames = facts.decoded_video_frames;
    source.avg_frame_rate = facts.avg_frame_rate;
    source.r_frame_rate = facts.r_frame_rate;
    let rate = FrameRate::new(24, 1).unwrap();
    assert!(source.avg_frame_rate != Some(rate) || source.r_frame_rate != Some(rate));
    let plan = FrameGridStitchPlan {
        output_frame_rate: rate,
        output_frame_count: 10,
        expected_duration_ms: ExactFrameGridDuration {
            numerator: 1250,
            denominator: 3,
        },
        spans: vec![
            FrameGridStitchSpan {
                span: RunAwareStitchSpan::Source {
                    run_sequence: 0,
                    checkpoint_sequence: 0,
                    artifact: "vfr.mp4".into(),
                    sha256: source.sha256.clone(),
                    logical_offset_ms: 0,
                    source_duration_ms: 200,
                },
                start_frame: 0,
                end_frame: 5,
            },
            FrameGridStitchSpan {
                span: RunAwareStitchSpan::EncoderGapPadding {
                    run_sequence: 0,
                    logical_start_ms: 200,
                    logical_end_ms: 425,
                },
                start_frame: 5,
                end_frame: 10,
            },
        ],
    };
    let output = root.path().join("source.mp4");
    FfmpegFrameGridPauseProjectionSourceStager::new("ffmpeg", "ffprobe")
        .stage_frame_grid(root.path(), &[source], &plan, &output)
        .unwrap();
    let actual = verifier.verify(&output).unwrap();
    assert_eq!(actual.decoded_video_frames, 10);
    assert_eq!(actual.duration_ms, 417);
    assert_eq!(actual.avg_frame_rate, Some(rate));
    assert_eq!(actual.r_frame_rate, Some(rate));
    super::super::artifacts::validate_frame_grid_staged_source(&output, actual, &plan).unwrap();
}

//! Real software FFmpeg checks of the recorder's local-input boundaries.
use record_render::ffmpeg::{self, ProcessControl};
use std::net::TcpListener;
use std::path::Path;
use std::time::Duration;

fn video(path: &Path, color: &str) {
    assert!(std::process::Command::new(cut_media::toolpath::ffmpeg())
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:size=160x90:rate=25:duration=0.4"),
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p"
        ])
        .arg(path)
        .status()
        .unwrap()
        .success());
}

#[test]
fn raw_mux_keeps_every_captured_video_packet_when_audio_ends_first() {
    fn audio(path: &Path, duration: &str, frequency: &str) {
        assert!(std::process::Command::new(cut_media::toolpath::ffmpeg())
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                &format!("sine=frequency={frequency}:sample_rate=48000:duration={duration}"),
                "-c:a",
                "pcm_s16le",
            ])
            .arg(path)
            .status()
            .unwrap()
            .success());
    }

    fn video_packets(path: &Path) -> Vec<(f64, f64, String)> {
        let output = std::process::Command::new(cut_media::toolpath::ffprobe())
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_packets",
                "-show_data_hash",
                "sha256",
                "-show_entries",
                "packet=pts_time,dts_time,data_hash",
                "-of",
                "json",
            ])
            .arg(path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "ffprobe failed for {}",
            path.display()
        );
        let facts: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        facts["packets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|packet| {
                (
                    packet["pts_time"].as_str().unwrap().parse().unwrap(),
                    packet["dts_time"].as_str().unwrap().parse().unwrap(),
                    packet["data_hash"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    fn assert_copied_video(source: &[(f64, f64, String)], output: &Path) {
        let copied = video_packets(output);
        assert_eq!(
            copied.len(),
            source.len(),
            "{} lost video packets",
            output.display()
        );
        for (index, (before, after)) in source.iter().zip(&copied).enumerate() {
            assert_eq!(
                before.2,
                after.2,
                "video packet {index} changed in {}",
                output.display()
            );
            // MP4 muxers may rebase the stream, but must preserve packet cadence.
            assert!(
                ((before.0 - source[0].0) - (after.0 - copied[0].0)).abs() < 0.001,
                "video packet {index} PTS changed in {}",
                output.display()
            );
            assert!(
                ((before.1 - source[0].1) - (after.1 - copied[0].1)).abs() < 0.001,
                "video packet {index} DTS changed in {}",
                output.display()
            );
        }
    }

    fn assert_mixed_audio(output: &Path) {
        let decoded = std::process::Command::new(cut_media::toolpath::ffmpeg())
            .args(["-v", "error", "-i"])
            .arg(output)
            .args([
                "-map", "0:a:0", "-ar", "48000", "-ac", "1", "-f", "s16le", "-",
            ])
            .output()
            .unwrap();
        assert!(decoded.status.success(), "mixed audio must decode");
        assert_eq!(decoded.stdout.len() % 2, 0);
        let samples: Vec<f64> = decoded
            .stdout
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]) as f64)
            .collect();
        assert!(
            samples.len() >= 48000 * 38 / 100,
            "audio must span video tail"
        );
        let tone = |start_ms: usize, end_ms: usize, frequency: f64| {
            let window = &samples[start_ms * 48..end_ms * 48];
            let (real, imaginary) =
                window
                    .iter()
                    .enumerate()
                    .fold((0.0, 0.0), |(real, imaginary), (index, sample)| {
                        let phase = std::f64::consts::TAU * frequency * index as f64 / 48000.0;
                        (
                            real + sample * phase.cos(),
                            imaginary + sample * phase.sin(),
                        )
                    });
            2.0 * real.hypot(imaginary) / window.len() as f64 / 32768.0
        };
        assert!(tone(3, 20, 440.0) > 0.07, "mic must start at zero");
        assert!(tone(3, 20, 660.0) < 0.035, "system offset must survive mix");
        assert!(tone(60, 120, 440.0) > 0.07, "mic must remain full-level");
        assert!(tone(60, 120, 660.0) > 0.07, "system must remain full-level");
        assert!(tone(180, 220, 440.0) < 0.035, "mic must end naturally");
        assert!(tone(180, 220, 660.0) > 0.07, "system must continue");
        assert!(tone(310, 370, 440.0) < 0.02, "audio tail must be silent");
        assert!(tone(310, 370, 660.0) < 0.02, "audio tail must be silent");
    }

    super::align_ffmpeg_env();
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mp4");
    let mic = dir.path().join("mic.wav");
    let system = dir.path().join("system.wav");
    let long_mic = dir.path().join("long-mic.wav");
    video(&source, "blue");
    audio(&mic, "0.16", "440");
    audio(&system, "0.24", "660");
    audio(&long_mic, "0.8", "880");
    let source_packets = video_packets(&source);
    assert_eq!(
        source_packets.len(),
        10,
        "fixture must exercise the video tail"
    );

    for (name, mic_input, system_input, offset_ms) in [
        ("no-audio", None, None, None),
        ("mic-only", Some(mic.as_path()), None, None),
        ("system-only", None, Some(system.as_path()), Some(25)),
        (
            "both",
            Some(mic.as_path()),
            Some(system.as_path()),
            Some(25),
        ),
        ("longer-audio", Some(long_mic.as_path()), None, None),
        (
            "both-longer-audio",
            Some(long_mic.as_path()),
            Some(system.as_path()),
            Some(25),
        ),
    ] {
        let output = dir.path().join(format!("{name}.mp4"));
        super::mux_raw_sources(&source, mic_input, system_input, offset_ms, &output).unwrap();
        assert_copied_video(&source_packets, &output);
        if name == "both" {
            assert_mixed_audio(&output);
        }
        if name == "both-longer-audio" {
            let probe = std::process::Command::new(cut_media::toolpath::ffprobe())
                .args([
                    "-v",
                    "error",
                    "-select_streams",
                    "a:0",
                    "-show_entries",
                    "stream=duration",
                    "-of",
                    "default=noprint_wrappers=1:nokey=1",
                ])
                .arg(&output)
                .output()
                .unwrap();
            assert!(probe.status.success());
            let duration: f64 = String::from_utf8_lossy(&probe.stdout)
                .trim()
                .parse()
                .unwrap();
            assert!(
                (0.38..0.45).contains(&duration),
                "long mixed audio must end with the video, got {duration}s"
            );
        }
    }

    // The separate raw export route also stream-copies video with mic audio.
    let control = ProcessControl::bounded(Duration::from_secs(10), || false);
    let export = dir.path().join("raw-export.mp4");
    super::mux_raw_with_control(&source, Some(&mic), &export, &control).unwrap();
    assert_copied_video(&source_packets, &export);
}

#[test]
fn recorder_local_formats_block_playlists_in_probe_grab_camera_raw_and_gif() {
    super::align_ffmpeg_env();
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().canonicalize().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/private.ts", listener.local_addr().unwrap());
    let hostile = dir_path.join("disguised.mp4");
    std::fs::write(
        &hostile,
        format!("#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\n{url}\n#EXT-X-ENDLIST\n"),
    )
    .unwrap();
    let control = ProcessControl::bounded(Duration::from_secs(10), || false);
    let source = hostile.to_str().unwrap();
    assert!(ffmpeg::probe_with_control(source, &control).is_err());
    assert!(ffmpeg::grab_frame_with_control(source, 0, &control).is_err());
    let mut camera = ffmpeg::stream_square_with_control(source, 32, 25.0, &control).unwrap();
    assert!(camera.at(0).is_err());
    drop(camera);
    assert!(
        super::mux_raw_with_control(&hostile, None, &dir_path.join("raw.mp4"), &control).is_err()
    );
    assert!(
        super::gif_with_control(&hostile, &dir_path.join("out.gif"), 10, 160, &control).is_err()
    );
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "hostile media must create zero loopback connections"
    );

    // Local concat is dangerous too: protocol fencing alone permits this file
    // reference. The format policy must refuse it before decoding that media.
    let outside = tempfile::tempdir().unwrap();
    let outside_path = outside.path().canonicalize().unwrap();
    let referenced = outside_path.join("outside.mp4");
    video(&referenced, "red");
    std::fs::write(
        &hostile,
        format!("ffconcat version 1.0\nfile '{}'\n", referenced.display()),
    )
    .unwrap();
    assert!(ffmpeg::probe_with_control(source, &control).is_err());
    assert!(ffmpeg::grab_frame_with_control(source, 0, &control).is_err());
    let mut camera = ffmpeg::stream_square_with_control(source, 32, 25.0, &control).unwrap();
    assert!(camera.at(0).is_err());
    assert!(
        super::mux_raw_with_control(&hostile, None, &dir_path.join("concat.mp4"), &control)
            .is_err()
    );
}

#[test]
fn admitted_captured_and_registered_cameras_render_real_frames_and_cache_normally() {
    super::align_ffmpeg_env();
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().canonicalize().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_path = outside.path().canonicalize().unwrap();
    let source = dir_path.join("source.mp4");
    let local_camera = dir_path.join("camera.mp4");
    let external_camera = outside_path.join("registered.mp4");
    video(&source, "blue");
    video(&local_camera, "red");
    video(&external_camera, "green");
    let mut project = cut_core::Project::new("camera", Default::default());
    project.assets.insert(
        "registered".into(),
        cut_core::Asset {
            path: external_camera.to_string_lossy().into_owned(),
            hash: "synthetic".into(),
            probe: None,
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    let cache = super::screen_record_cache_dir(&dir_path).unwrap();
    let control = ProcessControl::bounded(Duration::from_secs(30), || false);
    for (name, camera) in [("captured", local_camera), ("registered", external_camera)] {
        let mut plan = super::plan_inputs::tests::camera_plan(&camera);
        plan.duration_ms = 400;
        plan.frame.enabled = false;
        super::plan_inputs::admit(&dir_path, &project, &mut plan).unwrap();
        let output = cache.join(format!("{name}.mp4"));
        let mut rendered_frames = 0;
        super::cache_output::bake_seed(&output, |stage| {
            rendered_frames = super::render_with_control(&source, &plan, stage, None, &control)?;
            Ok(())
        })
        .unwrap();
        assert_eq!(rendered_frames, 10);
        super::cache_output::bake_seed(&output, |_| panic!("warm bake rerendered")).unwrap();
        let facts = ffmpeg::probe_with_control(output.to_str().unwrap(), &control).unwrap();
        assert_eq!((facts.w, facts.h), (160, 90));
        let (width, height, frame) =
            ffmpeg::grab_frame_with_control(output.to_str().unwrap(), 0, &control).unwrap();
        assert_eq!((width, height), (160, 90));
        // The overlay must actually deliver pixels different from the blue main
        // capture; admission alone does not certify camera decoder continuity.
        assert!(frame
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[0] > pixel[2] || pixel[1] > pixel[2]));
    }
    let raw = cache.join("raw.mp4");
    super::cache_output::bake_seed(&raw, |stage| {
        super::mux_raw_with_control(&source, None, stage, &control)
    })
    .unwrap();
    let gif = dir_path.join("export.gif");
    super::gif_with_control(&raw, &gif, 10, 160, &control).unwrap();
    assert!(std::fs::metadata(gif).unwrap().len() > 0);
}

#[test]
fn recorder_raw_and_audio_mix_file_inputs_refuse_local_concat_playlists() {
    super::align_ffmpeg_env();
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().canonicalize().unwrap();
    let capture = super::screen_record_cache_dir(&dir_path)
        .unwrap()
        .join("capture");
    std::fs::create_dir(&capture).unwrap();
    let source = capture.join("source.mp4");
    video(&source, "blue");
    let source = source.canonicalize().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_path = outside.path().canonicalize().unwrap();
    let audio = outside_path.join("outside.wav");
    assert!(std::process::Command::new(cut_media::toolpath::ffmpeg())
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=0.4",
            "-c:a",
            "pcm_s16le"
        ])
        .arg(&audio)
        .status()
        .unwrap()
        .success());
    let original = std::fs::read(&audio).unwrap();
    let playlist = format!("ffconcat version 1.0\nfile '{}'\n", audio.display());
    let mic = capture.join("mic.wav");
    let system = capture.join("system.wav");
    std::fs::write(
        capture.join("system-audio.json"),
        br#"{"schema":"shellx-cut/system-audio-timing/1","first_packet_offset_ms":1}"#,
    )
    .unwrap();
    let control = ProcessControl::bounded(Duration::from_secs(10), || false);
    let output = dir_path.join("raw.mp4");
    for (mic_bytes, system_bytes) in [
        (None, playlist.as_bytes()),
        (Some(original.as_slice()), playlist.as_bytes()),
        (Some(playlist.as_bytes()), original.as_slice()),
    ] {
        if let Some(bytes) = mic_bytes {
            std::fs::write(&mic, bytes).unwrap();
        }
        std::fs::write(&system, system_bytes).unwrap();
        let prepared = super::export_audio_for_source(&dir_path, &source).unwrap();
        assert!(
            !prepared.retry_inputs().is_empty(),
            "capture audio fixture must reach preparation"
        );
        assert_eq!(prepared.system_audio_offset_ms(), 1);
        assert!(prepared.prepare(&dir_path, &control).is_err());
        assert!(super::mux_raw_sources(
            &source,
            mic_bytes.map(|_| mic.as_path()),
            Some(&system),
            Some(1),
            &output
        )
        .is_err());
    }
    assert_eq!(std::fs::read(&audio).unwrap(), original);
    assert!(!std::fs::read_dir(&dir_path).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".cut-recorder-export-audio-")));
}

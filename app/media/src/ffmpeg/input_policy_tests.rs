//! Real-tool regressions for self-contained local-media admission.

use super::*;

#[test]
fn owned_concat_rejects_a_cached_segment_containing_nested_concat() {
    let sample =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../server/assets/first-edit-sample.mp4");
    let temp = crate::atomic_output::private_test_tempdir();
    let referenced_media = temp.path().join("private.mp4");
    std::fs::copy(&sample, &referenced_media).unwrap();
    let replaced_segment = temp.path().join("seg_cached.mp4");
    std::fs::write(
        &replaced_segment,
        format!(
            "ffconcat version 1.0\n{}\n",
            // Relative regular names remain accepted by the nested concat
            // demuxer's independent safe-path check.
            concat_demuxer_file_line(Path::new("private.mp4"))
        ),
    )
    .unwrap();
    let list = temp.path().join("owned.ffcat");
    let output = temp.path().join("output.mp4");
    let args = vec![
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-i".into(),
        list.display().to_string(),
        "-c".into(),
        "copy".into(),
        output.display().to_string(),
    ];
    // The child really is an accepted concat demuxer without entry-specific
    // admission, independently of its cached .mp4 filename.
    std::fs::write(
        &list,
        format!(
            "ffconcat version 1.0\n{}\n",
            concat_demuxer_file_line(&replaced_segment)
        ),
    )
    .unwrap();
    run_owned_concat_atomic_output(&args, &output)
        .expect("the unrestricted control must really decode the referenced fixture");
    std::fs::remove_file(&output).unwrap();
    std::fs::write(
        &list,
        format!(
            "ffconcat version 1.0\n{}\n",
            owned_concat_file_lines(&replaced_segment)
        ),
    )
    .unwrap();
    let error = run_owned_concat_atomic_output(&args, &output).unwrap_err();
    assert!(error.cause.contains("not on whitelist"), "{}", error.cause);
    assert!(!output.exists());
    // A normal cached/rendered MP4 remains usable through the same route.
    std::fs::write(
        &list,
        format!(
            "ffconcat version 1.0\n{}\n",
            owned_concat_file_lines(&sample)
        ),
    )
    .unwrap();
    run_owned_concat_atomic_output(&args, &output).unwrap();
}

#[test]
fn local_media_rejects_local_playlists_before_opening_segments() {
    // This is an executable FFmpeg contract test, not an extension filter:
    // the private segment is valid media and the manifest also has an MP4 name.
    let sample =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../server/assets/first-edit-sample.mp4");
    let temp = tempfile::tempdir().unwrap();
    let private = temp.path().join("private");
    let incoming = temp.path().join("incoming");
    std::fs::create_dir(&private).unwrap();
    std::fs::create_dir(&incoming).unwrap();
    let segment = private.join("unregistered.ts");
    run_ffmpeg(&[
        "-i".into(),
        sample.display().to_string(),
        "-c".into(),
        "copy".into(),
        "-f".into(),
        "mpegts".into(),
        segment.display().to_string(),
    ])
    .unwrap();
    assert!(ffprobe_json(&segment).is_ok(), "TS remains admitted");
    let manifest = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:60\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:60.0,\n{}\n#EXT-X-ENDLIST\n",
        segment.display()
    );
    for name in ["local.m3u8", "disguised.mp4"] {
        let playlist = incoming.join(name);
        std::fs::write(&playlist, &manifest).unwrap();
        // Positive exploit control: this exact synthetic manifest can read
        // media outside its own directory without the demuxer restriction.
        let control = Command::new(ffprobe_bin())
            .args([
                "-v",
                "debug",
                "-protocol_whitelist",
                LOCAL_INPUT_PROTOCOLS,
                "-f",
                "hls",
                "-show_streams",
            ])
            .arg(&playlist)
            .output()
            .unwrap();
        assert!(
            control.status.success(),
            "{}",
            String::from_utf8_lossy(&control.stderr)
        );
        assert!(String::from_utf8_lossy(&control.stderr)
            .contains(&format!("Opening '{}'", segment.display())));
        let error = ffprobe_json(&playlist).expect_err("probe must reject the demuxer");
        if name.ends_with("m3u8") {
            assert!(error.cause.contains("not on whitelist"), "{}", error.cause);
        }
        let error = run_ffmpeg(&[
            "-i".into(),
            playlist.display().to_string(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ])
        .expect_err("decode must reject the demuxer");
        if name.ends_with("m3u8") {
            assert!(error.cause.contains("not on whitelist"), "{}", error.cause);
        }
        // Debug output provides an independent no-segment-open assertion.
        let restricted = Command::new(ffprobe_bin())
            .args([
                "-v",
                "debug",
                "-protocol_whitelist",
                LOCAL_INPUT_PROTOCOLS,
                "-format_whitelist",
                LOCAL_INPUT_FORMATS,
                "-f",
                "hls",
                "-show_streams",
            ])
            .arg(&playlist)
            .output()
            .unwrap();
        assert!(!restricted.status.success());
        assert!(!String::from_utf8_lossy(&restricted.stderr)
            .contains(&format!("Opening '{}'", segment.display())));
    }
    let concat = incoming.join("disguised.mov");
    std::fs::write(
        &concat,
        format!(
            "ffconcat version 1.0\n{}\n",
            concat_demuxer_file_line(&sample)
        ),
    )
    .unwrap();
    assert!(ffprobe_json(&concat)
        .unwrap_err()
        .cause
        .contains("not on whitelist"));
    assert!(run_ffmpeg(&[
        "-i".into(),
        concat.display().to_string(),
        "-f".into(),
        "null".into(),
        "-".into()
    ])
    .is_err());
}

#[test]
fn local_media_common_containers_and_audio_remain_admitted() {
    let sample =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../server/assets/first-edit-sample.mp4");
    let temp = tempfile::tempdir().unwrap();
    for ext in ["mov", "mkv", "avi"] {
        let out = temp.path().join(format!("local.{ext}"));
        run_ffmpeg(&[
            "-i".into(),
            sample.display().to_string(),
            "-c".into(),
            "copy".into(),
            out.display().to_string(),
        ])
        .unwrap();
        assert!(ffprobe_json(&out).is_ok(), "{ext} probe");
        run_ffmpeg(&[
            "-i".into(),
            out.display().to_string(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ])
        .unwrap();
    }
    for (ext, codec) in [("wav", "pcm_s16le"), ("flac", "flac")] {
        let out = temp.path().join(format!("local.{ext}"));
        run_ffmpeg(&[
            "-i".into(),
            sample.display().to_string(),
            "-vn".into(),
            "-c:a".into(),
            codec.into(),
            out.display().to_string(),
        ])
        .unwrap();
        assert!(ffprobe_json(&out).is_ok(), "{ext} probe");
        run_ffmpeg(&[
            "-i".into(),
            out.display().to_string(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ])
        .unwrap();
    }
}

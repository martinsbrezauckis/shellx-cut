use super::*;

fn write_wav(path: &Path, frames: u32) -> (u64, String) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 1_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for _ in 0..frames {
        writer.write_sample::<i16>(1).unwrap();
    }
    writer.finalize().unwrap();
    let bytes = std::fs::read(path).unwrap();
    (
        u64::try_from(bytes.len()).unwrap(),
        format!("{:x}", Sha256::digest(bytes)),
    )
}

fn source(
    stream: RecordingStream,
    artifact: &str,
    bytes: u64,
    sha256: String,
    duration_ms: u64,
    native_ready_raw_ms: u64,
    logical_start_ms: u64,
) -> VerifiedInputAudio {
    VerifiedInputAudio::test_audio(
        stream,
        artifact,
        bytes,
        sha256,
        duration_ms,
        native_ready_raw_ms,
        0,
        logical_start_ms,
    )
}

#[test]
fn selected_audio_is_compacted_to_product_track_leaves_without_pause_padding() {
    let temp = tempfile::tempdir().unwrap();
    let mic_first = temp.path().join("mic-first.wav");
    let mic_second = temp.path().join("mic-second.wav");
    let system_first = temp.path().join("system-first.wav");
    let system_second = temp.path().join("system-second.wav");
    let (mic_first_bytes, mic_first_hash) = write_wav(&mic_first, 100);
    let (mic_second_bytes, mic_second_hash) = write_wav(&mic_second, 100);
    let (system_first_bytes, system_first_hash) = write_wav(&system_first, 90);
    let (system_second_bytes, system_second_hash) = write_wav(&system_second, 90);
    let mic_stage = temp.path().join("mic.wav");
    let system_stage = temp.path().join("system.wav");

    stage_track(
        temp.path(),
        &mic_stage,
        RecordingStream::MicrophoneAudio,
        &[
            source(
                RecordingStream::MicrophoneAudio,
                "mic-first.wav",
                mic_first_bytes,
                mic_first_hash,
                100,
                0,
                0,
            ),
            source(
                RecordingStream::MicrophoneAudio,
                "mic-second.wav",
                mic_second_bytes,
                mic_second_hash,
                100,
                0,
                100,
            ),
        ],
    )
    .unwrap();
    stage_track(
        temp.path(),
        &system_stage,
        RecordingStream::SystemAudio,
        &[
            source(
                RecordingStream::SystemAudio,
                "system-first.wav",
                system_first_bytes,
                system_first_hash,
                90,
                10,
                0,
            ),
            source(
                RecordingStream::SystemAudio,
                "system-second.wav",
                system_second_bytes,
                system_second_hash,
                90,
                10,
                100,
            ),
        ],
    )
    .unwrap();

    assert_eq!(hound::WavReader::open(mic_stage).unwrap().duration(), 200);
    assert_eq!(
        hound::WavReader::open(system_stage).unwrap().duration(),
        190
    );
}

#[test]
fn selected_tracks_publish_the_product_audio_leaves_and_system_timing() {
    let temp = tempfile::tempdir().unwrap();
    let microphone = PrivateStaging::create(temp.path(), "test-audio", MICROPHONE_FILE).unwrap();
    let system = PrivateStaging::create(temp.path(), "test-audio", SYSTEM_FILE).unwrap();
    std::fs::write(microphone.path(), b"sealed microphone").unwrap();
    std::fs::write(system.path(), b"sealed system").unwrap();
    let microphone_leaf =
        ProjectionAudioLeaf::from_local(MICROPHONE_FILE, microphone.path()).unwrap();
    let system_leaf = ProjectionAudioLeaf::from_local(SYSTEM_FILE, system.path()).unwrap();
    let contract = ProjectionAudioContract::new(
        Some(microphone_leaf.clone()),
        Some((system_leaf.clone(), Some(17))),
    )
    .unwrap();
    let prepared = PreparedPauseProjectionAudio {
        tracks: vec![
            PreparedTrack {
                leaf: microphone_leaf,
                stage: microphone,
                first_packet_offset_ms: None,
            },
            PreparedTrack {
                leaf: system_leaf,
                stage: system,
                first_packet_offset_ms: Some(17),
            },
        ],
        contract,
    };

    prepared.publish(temp.path()).unwrap();

    assert_eq!(prepared.microphone_file(), Some(MICROPHONE_FILE));
    assert_eq!(
        std::fs::read(temp.path().join(MICROPHONE_FILE)).unwrap(),
        b"sealed microphone"
    );
    assert_eq!(
        std::fs::read(temp.path().join(SYSTEM_FILE)).unwrap(),
        b"sealed system"
    );
    let timing: crate::screen_record::system_audio::SystemAudioTiming = serde_json::from_slice(
        &std::fs::read(
            temp.path()
                .join(crate::screen_record::system_audio::SYSTEM_AUDIO_TIMING_FILE),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(timing.first_packet_offset_ms, Some(17));
    prepared.verify_published(temp.path()).unwrap();

    std::fs::write(temp.path().join(MICROPHONE_FILE), b"tampered microphone").unwrap();
    assert!(prepared.verify_published(temp.path()).is_err());
    std::fs::write(temp.path().join(MICROPHONE_FILE), b"sealed microphone").unwrap();

    std::fs::write(temp.path().join(SYSTEM_FILE), b"tampered system").unwrap();
    assert!(prepared.verify_published(temp.path()).is_err());
    std::fs::write(temp.path().join(SYSTEM_FILE), b"sealed system").unwrap();

    let changed_timing = crate::screen_record::system_audio::SystemAudioTiming {
        schema: "shellx-cut/system-audio-timing/1".into(),
        first_packet_offset_ms: Some(18),
    };
    std::fs::write(
        temp.path()
            .join(crate::screen_record::system_audio::SYSTEM_AUDIO_TIMING_FILE),
        serde_json::to_vec(&changed_timing).unwrap(),
    )
    .unwrap();
    assert!(prepared.verify_published(temp.path()).is_err());
}

#[cfg(unix)]
#[test]
fn projection_wav_open_refuses_a_link_before_hashing_or_copying() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target.wav");
    let linked = temp.path().join("linked.wav");
    std::fs::write(&target, b"outside selected WAV").unwrap();
    symlink(&target, &linked).unwrap();

    assert!(open_local(&linked).is_err());
}

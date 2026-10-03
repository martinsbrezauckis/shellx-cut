use super::audio_files::{open_local, wav_facts};
use super::*;
use sha2::{Digest, Sha256};
use std::path::Path;

fn raw_wav(path: &Path) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for index in 0..603_648 * 2 {
        writer.write_sample((index % 30_000) as i16).unwrap();
    }
    writer.finalize().unwrap();
}

#[test]
fn durable_audio_factory_terminalizes_meter_without_reviving_device_loss() {
    for lost in [false, true] {
        let level = Arc::new(crate::RollingAudioLevel::new());
        let factory = RequiredMacosPauseAudioFactory::new(
            PathBuf::new(),
            MicrophoneSource::SystemDefault,
            Some(level.clone()),
        );
        if lost {
            level.mark_device_lost();
        }
        drop(factory);
        level.observe_i16(&[i16::MIN]);
        let snapshot = level.snapshot();
        assert_eq!(
            snapshot.lifecycle,
            if lost {
                crate::AudioLevelLifecycle::DeviceLost
            } else {
                crate::AudioLevelLifecycle::Stopped
            }
        );
        assert!(snapshot.stale);
        assert_eq!(snapshot.peak_dbfs, None);
    }
}

#[test]
fn publishes_only_real_pcm_inside_the_screen_boundary_without_replacing_raw() {
    let dir = tempfile::tempdir().unwrap();
    let raw = dir.path().join("raw.wav");
    let final_path = dir.path().join("sealed.wav");
    raw_wav(&raw);
    let original = std::fs::read(&raw).unwrap();
    assert_eq!(hound::WavReader::open(&raw).unwrap().duration(), 603_648);
    publish_bounded_microphone_wav(&raw, &final_path, 15, 37, 10_321).unwrap();
    let mut reader = hound::WavReader::open(&final_path).unwrap();
    assert_eq!(reader.spec().channels, 2);
    assert_eq!(reader.spec().sample_rate, 48_000);
    assert_eq!(reader.duration(), 492_912);
    for (index, sample) in reader.samples::<i16>().enumerate() {
        assert_eq!(sample.unwrap(), (index % 30_000) as i16);
    }
    assert_eq!(std::fs::read(&raw).unwrap(), original);
    let mut sealed_file = open_local(&final_path).unwrap();
    let (hash, duration_ms) = wav_facts(&mut sealed_file).unwrap();
    assert_eq!(duration_ms, 10_269);
    let sealed = std::fs::read(&final_path).unwrap();
    assert_eq!(hash, format!("{:x}", Sha256::digest(&sealed)));
    assert!(publish_bounded_microphone_wav(&raw, &final_path, 15, 37, 10_321).is_err());
    assert_eq!(std::fs::read(&final_path).unwrap(), sealed);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn refuses_missing_interval_and_symlink_without_publishing_or_leaving_staging() {
    let dir = tempfile::tempdir().unwrap();
    let raw = dir.path().join("raw.wav");
    let linked = dir.path().join("link.wav");
    let final_path = dir.path().join("sealed.wav");
    raw_wav(&raw);
    assert!(publish_bounded_microphone_wav(&raw, &final_path, 15, 37, 52).is_err());
    std::os::unix::fs::symlink(&raw, &linked).unwrap();
    assert!(publish_bounded_microphone_wav(&linked, &final_path, 15, 37, 10_321).is_err());
    assert!(!final_path.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}

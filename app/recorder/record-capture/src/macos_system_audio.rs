//! Atomic publication for macOS Core Audio's physically padded system stream.

use std::path::Path;

/// Write the real first-packet offset as leading PCM silence. The final name is
/// published only after hound writes and finalizes its header, so a callback,
/// allocation, or disk failure leaves no current `system.wav` to double-shift or
/// accidentally consume. The debug record retains the measured offset; no timing
/// sidecar is emitted because the physical padding is already on the capture clock.
#[allow(dead_code)] // Linux test builds compile the format helper without the macOS caller.
pub(crate) fn publish_padded_system_wav(
    out_dir: &Path,
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
    first_packet_offset_ms: Option<u64>,
) -> Result<(), String> {
    publish_padded_system_wav_named(
        out_dir,
        "system.wav",
        samples,
        channels,
        sample_rate,
        first_packet_offset_ms,
    )
}

/// Publish one private Core Audio run under a caller-provided fixed leaf.
///
/// Pause-aware capture cannot reuse the ordinary `system.wav`: each sealed
/// native generation must keep its own immutable input to the shared recovery
/// and projection owner. The leaf name is deliberately a simple filename so
/// callers cannot escape the prepared capture directory.
#[allow(dead_code)] // Used by the private macOS pause owner on native builds.
pub(crate) fn publish_padded_system_wav_named(
    out_dir: &Path,
    file_name: &str,
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
    first_packet_offset_ms: Option<u64>,
) -> Result<(), String> {
    publish_system_wav_named_with_offset(
        out_dir,
        file_name,
        samples,
        channels,
        sample_rate,
        first_packet_offset_ms,
    )
}

/// Publish an unpadded private pause sidecar. Its first-packet offset remains
/// an explicit sealed timing fact, so the shared pause projection can position
/// each logical run exactly once. Writing physical padding here would make the
/// projection count that offset both in placement and in PCM samples.
#[allow(dead_code)] // Used by the private macOS pause owner on native builds.
pub(crate) fn publish_system_wav_named(
    out_dir: &Path,
    file_name: &str,
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
) -> Result<(), String> {
    publish_system_wav_named_with_offset(out_dir, file_name, samples, channels, sample_rate, None)
}

fn publish_system_wav_named_with_offset(
    out_dir: &Path,
    file_name: &str,
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
    first_packet_offset_ms: Option<u64>,
) -> Result<(), String> {
    if !record_recovery::is_plain_dir(out_dir).map_err(|error| error.to_string())? {
        return Err("system.wav capture directory is not a local directory".into());
    }
    if file_name.is_empty()
        || std::path::Path::new(file_name)
            .file_name()
            .and_then(|name| name.to_str())
            != Some(file_name)
    {
        return Err("system.wav pause leaf is not a simple filename".into());
    }
    let final_path = out_dir.join(file_name);
    let (part, file) = record_recovery::create_staging_file(out_dir, "system-wav")
        .map_err(|error| format!("reserve system.wav staging: {error}"))?;
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let written = (|| -> Result<(), hound::Error> {
        let mut writer = hound::WavWriter::new(file, spec)?;
        if let Some(offset_ms) = first_packet_offset_ms {
            let silence =
                crate::mic_timing::leading_silence_samples(offset_ms, sample_rate, channels);
            for _ in 0..silence {
                writer.write_sample(0_i16)?;
            }
        }
        for &sample in samples {
            writer.write_sample((sample.clamp(-1.0, 1.0) * 32767.0) as i16)?;
        }
        writer.finalize()
    })();
    if let Err(error) = written {
        let _ = std::fs::remove_file(&part);
        return Err(format!("write/finalize system.wav: {error}"));
    }
    if let Err(error) = record_recovery::publish_new_synced(&part, &final_path) {
        let _ = std::fs::remove_file(&part);
        return Err(format!("publish system.wav: {error}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::publish_padded_system_wav;

    #[test]
    fn writes_physical_padding_then_atomically_publishes_without_replacing() {
        let temp = tempfile::tempdir().unwrap();
        publish_padded_system_wav(temp.path(), &[0.5, 0.5], 2, 48_000, Some(1)).unwrap();
        let final_path = temp.path().join("system.wav");
        assert!(std::fs::metadata(&final_path).unwrap().len() > 44);
        assert!(!temp.path().join(".system.wav.part").exists());
        std::fs::write(&final_path, b"existing-final").unwrap();
        assert!(publish_padded_system_wav(temp.path(), &[0.5, 0.5], 2, 48_000, None).is_err());
        assert_eq!(std::fs::read(&final_path).unwrap(), b"existing-final");
        assert!(!temp.path().join(".system.wav.part").exists());
    }

    #[cfg(unix)]
    #[test]
    fn padded_wav_never_follows_the_legacy_staging_link() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("outside.wav");
        std::fs::write(&target, b"outside remains untouched").unwrap();
        symlink(&target, temp.path().join(".system.wav.part")).unwrap();
        publish_padded_system_wav(temp.path(), &[0.5, 0.5], 2, 48_000, Some(1)).unwrap();
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"outside remains untouched"
        );
        assert!(temp.path().join("system.wav").is_file());
        assert!(
            std::fs::symlink_metadata(temp.path().join(".system.wav.part"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn named_pause_leaf_is_no_replace_and_cannot_escape_its_capture_directory() {
        let temp = tempfile::tempdir().unwrap();
        let leaf = "recording-system-generation-00000000000000000001.wav";
        super::publish_padded_system_wav_named(temp.path(), leaf, &[0.5, 0.5], 2, 48_000, Some(1))
            .unwrap();
        assert!(temp.path().join(leaf).is_file());
        assert!(super::publish_padded_system_wav_named(
            temp.path(),
            leaf,
            &[0.5, 0.5],
            2,
            48_000,
            None,
        )
        .is_err());
        assert!(super::publish_padded_system_wav_named(
            temp.path(),
            "../escaped.wav",
            &[0.5],
            1,
            48_000,
            None,
        )
        .is_err());
    }

    #[test]
    fn unpadded_pause_sidecar_keeps_first_packet_placement_external() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp
            .path()
            .join("recording-system-generation-00000000000000000001.wav");
        super::publish_system_wav_named(
            temp.path(),
            path.file_name().unwrap().to_str().unwrap(),
            &[0.5, 0.5],
            2,
            48_000,
        )
        .unwrap();
        let mut reader = hound::WavReader::open(path).unwrap();
        assert_eq!(reader.samples::<i16>().next().unwrap().unwrap(), 16_383);
    }
}

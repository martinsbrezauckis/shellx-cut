//! A crash-safe copy of macOS system sound, written while the recording runs.
//!
//! The Core Audio tap keeps its PCM in memory and hands it over only at Stop,
//! so a forced quit, a crash or a failed Stop lost every second of system
//! sound. Once a second the recorder copies the new samples into a hidden
//! `.system-wav-*.part` file beside the microphone's own `.part` file; the WAV
//! header is brought up to date on every write. Recovery already makes a
//! playable copy of such a file. After a normal Stop the finished `system.wav`
//! is published as before and this copy is removed.
#![cfg_attr(
    not(all(target_os = "macos", feature = "capture-macos")),
    allow(dead_code)
)]

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

pub(crate) struct SystemAudioSpool {
    out_dir: PathBuf,
    file: Option<(PathBuf, hound::WavWriter<BufWriter<File>>)>,
    /// Why the copy stopped, kept for the capture's debug record.
    failed: Option<String>,
}

/// The same sample format the final `system.wav` uses.
pub(crate) fn wav_format(channels: u32, rate: f64) -> (u16, u32) {
    let channels = channels.clamp(1, 8) as u16;
    let rate = if rate.is_finite() && (8_000.0..=384_000.0).contains(&rate) {
        rate.round() as u32
    } else {
        48_000
    };
    (channels, rate)
}

impl SystemAudioSpool {
    pub(crate) fn new(out_dir: &Path) -> Self {
        Self {
            out_dir: out_dir.to_path_buf(),
            file: None,
            failed: None,
        }
    }

    /// Append newly arrived samples. The first write starts the file with the
    /// silence before the first packet, exactly as the final `system.wav` does.
    pub(crate) fn append(
        &mut self,
        samples: &[f32],
        channels: u32,
        rate: f64,
        first_packet_offset_ms: Option<u64>,
    ) {
        if samples.is_empty() || self.failed.is_some() {
            return;
        }
        if let Err(error) = self.write(samples, channels, rate, first_packet_offset_ms) {
            self.failed = Some(error);
        }
    }

    fn write(
        &mut self,
        samples: &[f32],
        channels: u32,
        rate: f64,
        first_packet_offset_ms: Option<u64>,
    ) -> Result<(), String> {
        if self.file.is_none() {
            let (channels, sample_rate) = wav_format(channels, rate);
            let (path, file) = record_recovery::create_staging_file(&self.out_dir, "system-wav")
                .map_err(|error| format!("reserve the system sound copy: {error}"))?;
            let spec = hound::WavSpec {
                channels,
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::new(BufWriter::new(file), spec)
                .map_err(|error| format!("start the system sound copy: {error}"))?;
            if let Some(offset_ms) = first_packet_offset_ms {
                let silence =
                    crate::mic_timing::leading_silence_samples(offset_ms, sample_rate, channels);
                for _ in 0..silence {
                    writer.write_sample(0_i16).map_err(|e| e.to_string())?;
                }
            }
            self.file = Some((path, writer));
        }
        let (_, writer) = self.file.as_mut().expect("created above");
        for &sample in samples {
            writer
                .write_sample((sample.clamp(-1.0, 1.0) * 32767.0) as i16)
                .map_err(|error| format!("write the system sound copy: {error}"))?;
        }
        // Writes the RIFF sizes for what is on disk now, then flushes it.
        writer
            .flush()
            .map_err(|error| format!("flush the system sound copy: {error}"))
    }

    pub(crate) fn failure(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    /// The finished `system.wav` exists: the copy is no longer needed.
    pub(crate) fn discard(mut self) {
        if let Some((path, writer)) = self.file.take() {
            drop(writer);
            let _ = std::fs::remove_file(path);
        }
    }

    /// Keep the copy for recovery (the final file could not be published).
    pub(crate) fn keep(mut self) {
        if let Some((_, writer)) = self.file.take() {
            let _ = writer.finalize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part_in(dir: &Path) -> Option<PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".system-wav-") && name.ends_with(".part"))
            })
    }

    #[test]
    fn the_copy_on_disk_is_a_readable_wav_after_every_write_without_a_finalize() {
        let dir = tempfile::tempdir().unwrap();
        let mut spool = SystemAudioSpool::new(dir.path());
        spool.append(&[], 2, 48_000.0, None);
        assert!(
            part_in(dir.path()).is_none(),
            "no file before the first sound"
        );
        spool.append(&[0.5; 960], 2, 48_000.0, Some(10));
        spool.append(&[0.25; 960], 2, 48_000.0, Some(10));
        let part = part_in(dir.path()).unwrap();
        // As after kill -9: the writer is never finalized, the file is read as is.
        let reader = hound::WavReader::open(&part).unwrap();
        assert_eq!(reader.spec().channels, 2);
        // 10 ms of leading silence (480 frames x 2 channels) + 1920 samples.
        assert_eq!(reader.len(), 960 + 1_920);
        std::mem::forget(spool);
    }

    #[test]
    fn discard_removes_the_copy_and_keep_leaves_it_finalized() {
        let dir = tempfile::tempdir().unwrap();
        let mut spool = SystemAudioSpool::new(dir.path());
        spool.append(&[0.1; 8], 1, 44_100.0, None);
        spool.discard();
        assert!(part_in(dir.path()).is_none());
        let mut spool = SystemAudioSpool::new(dir.path());
        spool.append(&[0.1; 8], 1, 1.0, None);
        spool.keep();
        let reader = hound::WavReader::open(part_in(dir.path()).unwrap()).unwrap();
        assert_eq!((reader.spec().sample_rate, reader.len()), (48_000, 8));
    }
}

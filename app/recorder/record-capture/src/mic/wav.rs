use record_core::{error_codes, RecordError, Result};

// Leave room for the RIFF/WAVE headers before hound's u32 byte counter reaches its limit.
// The limit is frame-aligned so every published WAV remains valid for multichannel input.
pub(super) const WAV_HEADER_MARGIN_BYTES: u64 = 4_096;

#[derive(Default)]
pub(super) struct PendingMicSamples {
    pub(super) samples: Vec<i16>,
    pub(super) overflowed: bool,
}

impl PendingMicSamples {
    pub(super) fn extend<I>(&mut self, samples: I, incoming_len: usize, limit: usize)
    where
        I: IntoIterator<Item = i16>,
    {
        let remaining = limit.saturating_sub(self.samples.len());
        if incoming_len > remaining {
            self.overflowed = true;
        }
        self.samples.extend(samples.into_iter().take(remaining));
    }
}

pub(super) fn wav_i16_sample_capacity(channels: u16) -> Result<u64> {
    let frame_bytes = u64::from(channels).checked_mul(2).ok_or_else(|| {
        RecordError::new(
            error_codes::CAPTURE,
            "microphone format",
            "microphone frame size overflowed",
        )
    })?;
    if frame_bytes == 0 {
        return Err(RecordError::new(
            error_codes::CAPTURE,
            "microphone format",
            "microphone reported zero channels",
        ));
    }
    let data_bytes = (u64::from(u32::MAX) - WAV_HEADER_MARGIN_BYTES) / frame_bytes * frame_bytes;
    Ok(data_bytes / 2)
}

pub(super) fn should_publish_microphone(samples_written: bool) -> bool {
    samples_written
}

pub(super) fn discard_unpublished_staging(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

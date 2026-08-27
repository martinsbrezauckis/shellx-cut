use record_core::{error_codes, RecordError, Result};

pub(super) fn decode_process_loopback_packet(
    data: Option<&[u8]>,
    frames: u32,
    silent: bool,
) -> Result<Vec<i16>> {
    const CHANNELS: usize = 2;
    const BLOCK_ALIGN: usize = CHANNELS * std::mem::size_of::<i16>();
    let sample_count = usize::try_from(frames)
        .ok()
        .and_then(|count| count.checked_mul(CHANNELS))
        .ok_or_else(|| {
            RecordError::new(
                error_codes::CAPTURE,
                "decode system audio",
                "WASAPI packet size overflowed",
            )
        })?;
    if silent {
        return Ok(vec![0; sample_count]);
    }
    let expected = usize::try_from(frames)
        .ok()
        .and_then(|count| count.checked_mul(BLOCK_ALIGN))
        .ok_or_else(|| {
            RecordError::new(
                error_codes::CAPTURE,
                "decode system audio",
                "WASAPI packet byte size overflowed",
            )
        })?;
    let data = data
        .filter(|bytes| bytes.len() >= expected)
        .ok_or_else(|| {
            RecordError::new(
                error_codes::CAPTURE,
                "decode system audio",
                "WASAPI returned a truncated audio packet",
            )
        })?;

    Ok(data[..expected]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
        .collect())
}

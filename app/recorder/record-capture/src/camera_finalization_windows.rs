//! Handle-anchored Windows Camera Capture Engine finalization.
//!
//! The writer and reader both operate on duplicates of the one `CREATE_NEW`
//! leaf held by `WindowsNoReplaceCameraStage`. No post-stop operation accepts a
//! pathname, so replacing the capture root, `camera` parent, or leaf cannot
//! redirect the hash or Media Foundation measurement.

use std::io::{Read, Seek, SeekFrom};

use record_core::{error_codes, CameraMediaFacts, RecordError, Result};
use sha2::{Digest, Sha256};
use windows::Win32::Media::MediaFoundation::{
    IMFAttributes, MFCreateSourceReaderFromByteStream, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
    MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
};

use super::{CameraMediaSeal, WindowsNoReplaceCameraStage};

/// Measure, hash, and protect the one retained `CREATE_NEW` leaf after
/// `MF_CAPTURE_ENGINE_RECORD_STOPPED`. `expected_duration_hns` is the exact
/// 100-nanosecond interval accepted from the native sample callback timeline;
/// millisecond artifact facts are only its deterministic floor projection.
pub(crate) fn finalize_windows_no_replace(
    mut stage: WindowsNoReplaceCameraStage,
    expected_duration_hns: i64,
) -> Result<CameraMediaSeal> {
    if expected_duration_hns <= 0 {
        return Err(finalization_error(
            "finalize Windows camera output",
            "native camera sample interval must be a positive 100-nanosecond span",
        ));
    }
    stage.verify_anchored_handles()?;
    let measured = measure_mp4(&stage)?;
    if measured.duration_hns != expected_duration_hns {
        return Err(finalization_error(
            "finalize Windows camera output",
            "decoded final-file sample-end span differs from the accepted native 100-nanosecond span",
        ));
    }
    let (sha256, bytes) = hash_anchored_file(&stage)?;
    let (rehashed_sha256, rehashed_bytes) = hash_anchored_file(&stage)?;
    if sha256 != rehashed_sha256 || bytes != rehashed_bytes {
        return Err(finalization_error(
            "Windows camera output changed while finalizing",
            "the retained CREATE_NEW leaf did not remain byte-identical",
        ));
    }
    stage.make_read_only()?;
    stage.verify_anchored_handles()?;
    let duration_ms = u64::try_from(measured.duration_hns)
        .ok()
        .map(|duration_hns| duration_hns / 10_000)
        .filter(|duration_ms| *duration_ms != 0)
        .ok_or_else(|| {
            finalization_error(
                "finalized Windows camera MP4 has no millisecond duration",
                "the exact 100-nanosecond sample interval must project to a non-zero shared clock span",
            )
        })?;
    let media = CameraMediaFacts {
        width: measured.width,
        height: measured.height,
        fps_num: measured.fps_num,
        fps_den: measured.fps_den,
        frame_count: measured.frame_count,
        duration_ms,
        sha256,
    };
    CameraMediaSeal::validate_finalizer_inputs(&stage.artifact_id, &stage.video, &media, bytes)?;
    let seal = CameraMediaSeal::from_finalizer(
        stage.artifact_id.clone(),
        stage.video.clone(),
        media,
        bytes,
    );
    stage.mark_finalized();
    Ok(seal)
}

fn hash_anchored_file(stage: &WindowsNoReplaceCameraStage) -> Result<(String, u64)> {
    stage.verify_anchored_handles()?;
    let mut file = stage.duplicate_file()?;
    let expected_bytes = file
        .metadata()
        .map_err(|error| {
            finalization_error("inspect anchored Windows camera leaf", &error.to_string())
        })?
        .len();
    if expected_bytes == 0 {
        return Err(finalization_error(
            "inspect anchored Windows camera leaf",
            "Capture Engine wrote no bytes to the reserved leaf",
        ));
    }
    file.seek(SeekFrom::Start(0)).map_err(|error| {
        finalization_error("rewind anchored Windows camera leaf", &error.to_string())
    })?;
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 32 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            finalization_error("hash anchored Windows camera leaf", &error.to_string())
        })?;
        if read == 0 {
            break;
        }
        bytes = bytes.checked_add(read as u64).ok_or_else(|| {
            finalization_error(
                "hash anchored Windows camera leaf",
                "complete-file byte count overflowed",
            )
        })?;
        hasher.update(&buffer[..read]);
    }
    let actual_bytes = file
        .metadata()
        .map_err(|error| {
            finalization_error("reinspect anchored Windows camera leaf", &error.to_string())
        })?
        .len();
    if bytes != expected_bytes || actual_bytes != expected_bytes {
        return Err(finalization_error(
            "Windows camera output changed while hashing",
            "the retained CREATE_NEW leaf size changed during hashing",
        ));
    }
    stage.verify_anchored_handles()?;
    Ok((format!("{:x}", hasher.finalize()), bytes))
}

#[derive(Debug)]
struct MeasuredMp4 {
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
    frame_count: u64,
    duration_hns: i64,
}

/// Decode the closed MP4 from a byte stream over the retained leaf. The span
/// is `last sample end - first sample start` in Media Foundation's native
/// 100-nanosecond units, never wall-clock callback arrival time.
fn measure_mp4(stage: &WindowsNoReplaceCameraStage) -> Result<MeasuredMp4> {
    let (byte_stream, _reader_stream) = stage.source_reader_stream()?;
    // SAFETY: the byte stream wraps a duplicate of the one exact reserved leaf
    // and reader creation/measurement are synchronous in this finalizer.
    unsafe {
        let reader = MFCreateSourceReaderFromByteStream(&byte_stream, None::<&IMFAttributes>)
            .map_err(|error| {
                finalization_error("open anchored Windows camera MP4", &error.to_string())
            })?;
        let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let media_type = reader.GetCurrentMediaType(stream).map_err(|error| {
            finalization_error(
                "read anchored Windows camera media type",
                &error.to_string(),
            )
        })?;
        let frame_size = media_type.GetUINT64(&MF_MT_FRAME_SIZE).map_err(|error| {
            finalization_error(
                "read anchored Windows camera dimensions",
                &error.to_string(),
            )
        })?;
        let frame_rate = media_type.GetUINT64(&MF_MT_FRAME_RATE).map_err(|error| {
            finalization_error("read anchored Windows camera FPS", &error.to_string())
        })?;
        let width = (frame_size >> 32) as u32;
        let height = frame_size as u32;
        let fps_num = (frame_rate >> 32) as u32;
        let fps_den = frame_rate as u32;
        if width == 0 || height == 0 || fps_num == 0 || fps_den == 0 {
            return Err(finalization_error(
                "anchored Windows camera MP4 has incomplete media facts",
                "Media Foundation must report non-zero dimensions and frame rate",
            ));
        }
        let mut frame_count = 0_u64;
        let mut first_start_hns: Option<i64> = None;
        let mut final_end_hns: Option<i64> = None;
        loop {
            let mut flags = 0_u32;
            let mut timestamp_hns = 0_i64;
            let mut sample = None;
            reader
                .ReadSample(
                    stream,
                    0,
                    None,
                    Some(&mut flags),
                    Some(&mut timestamp_hns),
                    Some(&mut sample),
                )
                .map_err(|error| {
                    finalization_error("decode anchored Windows camera MP4", &error.to_string())
                })?;
            if let Some(sample) = sample {
                let duration_hns = sample.GetSampleDuration().map_err(|error| {
                    finalization_error(
                        "read anchored Windows camera frame duration",
                        &error.to_string(),
                    )
                })?;
                if timestamp_hns < 0 || duration_hns <= 0 {
                    return Err(finalization_error(
                        "anchored Windows camera MP4 has invalid frame timing",
                        "decoded samples need non-negative starts and positive durations",
                    ));
                }
                let end_hns = timestamp_hns.checked_add(duration_hns).ok_or_else(|| {
                    finalization_error(
                        "anchored Windows camera frame timing overflowed",
                        "decoded sample end exceeded the supported Media Foundation range",
                    )
                })?;
                first_start_hns =
                    Some(first_start_hns.map_or(timestamp_hns, |first| first.min(timestamp_hns)));
                final_end_hns = Some(final_end_hns.map_or(end_hns, |last| last.max(end_hns)));
                frame_count = frame_count.checked_add(1).ok_or_else(|| {
                    finalization_error(
                        "anchored Windows camera frame count overflowed",
                        "too many decoded samples",
                    )
                })?;
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
        }
        let duration_hns = first_start_hns
            .zip(final_end_hns)
            .and_then(|(first, last)| last.checked_sub(first))
            .filter(|duration| *duration > 0)
            .ok_or_else(|| {
                finalization_error(
                    "anchored Windows camera MP4 has no decoded duration",
                    "decoded samples must establish a positive first-start to last-end span",
                )
            })?;
        Ok(MeasuredMp4 {
            width,
            height,
            fps_num,
            fps_den,
            frame_count,
            duration_hns,
        })
    }
}

fn finalization_error(message: &str, cause: &str) -> RecordError {
    RecordError::new(error_codes::CAPTURE, message, cause)
}

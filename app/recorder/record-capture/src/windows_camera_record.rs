//! Capture Engine record-sink configuration and output ownership.

use std::path::Path;

use record_core::Result;
use sha2::{Digest, Sha256};
use windows::core::Interface;
use windows::Win32::Media::MediaFoundation::{
    IMFAttributes, IMFByteStream, IMFCaptureEngine, IMFCaptureRecordSink, MFCreateMediaType,
    MFTranscodeContainerType_MPEG4, MFVideoFormat_H264,
    MF_CAPTURE_ENGINE_PREFERRED_SOURCE_STREAM_FOR_VIDEO_RECORD, MF_CAPTURE_ENGINE_SINK_TYPE_RECORD,
    MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_SUBTYPE,
};
use windows::Win32::System::Com::IStream;

use super::{camera_error, windows_error};
use crate::camera_finalization::{reserve_windows_no_replace_output, WindowsNoReplaceCameraStage};

pub(super) struct PreparedRecord {
    pub(super) output: IMFByteStream,
    pub(super) writer_stream: IStream,
    pub(super) stage: WindowsNoReplaceCameraStage,
}

pub(super) fn configure_record(
    capture_directory: &Path,
    capture_id: &str,
    engine: &IMFCaptureEngine,
) -> Result<PreparedRecord> {
    let artifact_id = camera_artifact_id(capture_id);
    let video = format!("camera/{artifact_id}.mp4");
    // SAFETY: all types belong to the initialized Capture Engine; the output
    // stream itself wraps only the reserved CREATE_NEW leaf handle.
    unsafe {
        let record: IMFCaptureRecordSink = engine
            .GetSink(MF_CAPTURE_ENGINE_SINK_TYPE_RECORD)
            .map_err(|error| windows_error("open Windows camera record sink", error))?
            .cast()
            .map_err(|error| windows_error("cast Windows camera record sink", error))?;
        record
            .RemoveAllStreams()
            .map_err(|error| windows_error("reset Windows camera record sink", error))?;
        let source = engine
            .GetSource()
            .map_err(|error| windows_error("open Windows camera record source", error))?;
        let native_type = source
            .GetCurrentDeviceMediaType(MF_CAPTURE_ENGINE_PREFERRED_SOURCE_STREAM_FOR_VIDEO_RECORD.0)
            .map_err(|error| windows_error("read Windows camera record media type", error))?;
        let output_type = MFCreateMediaType()
            .map_err(|error| windows_error("create Windows camera output media type", error))?;
        native_type
            .CopyAllItems(&output_type)
            .map_err(|error| windows_error("copy Windows camera output media type", error))?;
        output_type
            .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)
            .map_err(|error| windows_error("select Windows camera H.264 output", error))?;
        output_type
            .SetUINT32(&MF_MT_AVG_BITRATE, output_bitrate(&native_type)?)
            .map_err(|error| windows_error("set Windows camera output bitrate", error))?;
        let mut sink_index = 0_u32;
        record
            .AddStream(
                MF_CAPTURE_ENGINE_PREFERRED_SOURCE_STREAM_FOR_VIDEO_RECORD.0,
                &output_type,
                None::<&IMFAttributes>,
                Some(&mut sink_index),
            )
            .map_err(|error| windows_error("connect Windows camera record stream", error))?;
        let (output, writer_stream, stage) =
            reserve_windows_no_replace_output(capture_directory, artifact_id, video)?;
        if let Err(error) = record.SetOutputByteStream(&output, &MFTranscodeContainerType_MPEG4) {
            release_unbound_output(output, writer_stream, stage);
            return Err(windows_error(
                "bind handle-anchored Windows camera output",
                error,
            ));
        }
        Ok(PreparedRecord {
            output,
            writer_stream,
            stage,
        })
    }
}

fn release_unbound_output(
    output: IMFByteStream,
    writer_stream: IStream,
    stage: WindowsNoReplaceCameraStage,
) {
    let _ = unsafe { output.Close() };
    drop(output);
    drop(writer_stream);
    drop(stage);
}

fn output_bitrate(
    media_type: &windows::Win32::Media::MediaFoundation::IMFMediaType,
) -> Result<u32> {
    // SAFETY: packed media type attributes belong to the initialized source.
    unsafe {
        let size = media_type
            .GetUINT64(&MF_MT_FRAME_SIZE)
            .map_err(|error| windows_error("read Windows camera output dimensions", error))?;
        let rate = media_type
            .GetUINT64(&MF_MT_FRAME_RATE)
            .map_err(|error| windows_error("read Windows camera output FPS", error))?;
        let pixels = (size >> 32).checked_mul(size & u64::from(u32::MAX));
        pixels
            .and_then(|value| value.checked_mul(rate >> 32))
            .and_then(|value| value.checked_div((rate & u64::from(u32::MAX)).max(1)))
            .and_then(|value| value.checked_div(3))
            .filter(|value| *value != 0)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                camera_error(
                    "configure Windows camera output bitrate",
                    "camera media type has unsupported dimensions or frame rate",
                )
            })
    }
}

fn camera_artifact_id(capture_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"shellx-cut/windows-camera-artifact/v1\0");
    hasher.update(capture_id.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("camera_{}", &digest[..32])
}

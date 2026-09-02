//! Public-safe projections for the process-local native source-preview owner.

use base64::Engine as _;
use cut_core::{error_codes, CutError, VerbResult};
use record_capture::source_preview::{
    SourcePreviewFrame, SourcePreviewRequest, SourcePreviewStatus, MAX_SOURCE_PREVIEW_FRAME_BYTES,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::source_preview;
use crate::dispatch::parse_args;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartArgs {
    source: record_capture::source_preview::SourcePreviewSource,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

pub(crate) fn capability_handler(args: Value) -> Result<VerbResult, CutError> {
    let _: EmptyArgs = parse_args(args)?;
    Ok(VerbResult::ok(json!(source_preview::capability())))
}

pub(crate) fn start_handler(args: Value) -> Result<VerbResult, CutError> {
    let args: StartArgs = parse_args(args)?;
    let status = source_preview::start(SourcePreviewRequest {
        source: args.source,
        camera_id: None,
    })?;
    Ok(action_result("start", status))
}

pub(crate) fn status_handler(args: Value) -> Result<VerbResult, CutError> {
    let _: EmptyArgs = parse_args(args)?;
    let (status, _) = source_preview::snapshot()?;
    Ok(VerbResult::ok(status_value(status)))
}

pub(crate) fn frame_handler(args: Value) -> Result<VerbResult, CutError> {
    let _: EmptyArgs = parse_args(args)?;
    let (status, frame) = source_preview::snapshot()?;
    Ok(VerbResult::ok(json!({
        "status": status_value(status.clone()),
        "frame": frame_value(status, frame)?,
    })))
}

pub(crate) fn pause_handler(args: Value) -> Result<VerbResult, CutError> {
    let _: EmptyArgs = parse_args(args)?;
    Ok(action_result("pause", source_preview::pause()?))
}

pub(crate) fn resume_handler(args: Value) -> Result<VerbResult, CutError> {
    let _: EmptyArgs = parse_args(args)?;
    Ok(action_result("resume", source_preview::resume()?))
}

pub(crate) fn hide_handler(args: Value) -> Result<VerbResult, CutError> {
    let _: EmptyArgs = parse_args(args)?;
    Ok(action_result("hide", source_preview::hide()?))
}

pub(crate) fn stop_handler(args: Value) -> Result<VerbResult, CutError> {
    let _: EmptyArgs = parse_args(args)?;
    Ok(action_result("stop", source_preview::stop()?))
}

fn action_result(action: &str, status: SourcePreviewStatus) -> VerbResult {
    VerbResult::ok(json!({ "action": action, "status": status_value(status) }))
}

/// Public projection deliberately omits source identities and all private regions.
fn status_value(status: SourcePreviewStatus) -> Value {
    json!({
        "state": status.state,
        "recursion": status.recursion,
        "has_frame": status.has_frame,
        "generation": status.generation,
    })
}

fn frame_value(
    status: SourcePreviewStatus,
    frame: Option<SourcePreviewFrame>,
) -> Result<Value, CutError> {
    let Some(frame) = frame else {
        return Ok(Value::Null);
    };
    let generation = status.generation.ok_or_else(|| {
        CutError::new(
            error_codes::CONFLICT,
            "native preview frame no longer has an active generation",
            "the preview owner released or replaced the source before the frame could be returned",
        )
    })?;
    let bytes = frame.encoded();
    if bytes.is_empty() || bytes.len() > MAX_SOURCE_PREVIEW_FRAME_BYTES {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "native preview frame exceeds the public memory bound",
            "only 1 byte to 4 MiB BMP frames may leave the native preview owner",
        ));
    }
    if bytes.len() < 2 || &bytes[..2] != b"BM" {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "native preview frame is not a BMP",
            "only bounded BMP bytes may leave the native preview owner",
        ));
    }
    Ok(json!({
        "mime": "image/bmp",
        "bytes": bytes.len(),
        "generation": generation,
        "captured_at_ms": frame.captured_at_ms(),
        "base64": base64::engine::general_purpose::STANDARD.encode(bytes),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use record_capture::source_preview::{SourcePreviewRecursion, SourcePreviewState};

    fn status(generation: Option<u64>) -> SourcePreviewStatus {
        SourcePreviewStatus {
            state: SourcePreviewState::Ready,
            recursion: SourcePreviewRecursion::None,
            request: None,
            generation,
            has_frame: true,
        }
    }

    #[test]
    fn public_status_omits_the_private_source_identity() {
        let value = status_value(status(Some(3)));
        assert_eq!(value["generation"], 3);
        assert!(value.get("request").is_none());
        assert!(value.get("monitor_id").is_none());
    }

    #[test]
    fn frame_projection_is_bounded_bmp_base64_with_its_generation() {
        let frame = SourcePreviewFrame::new(42, vec![b'B', b'M', 1]).unwrap();
        let value = frame_value(status(Some(9)), Some(frame)).unwrap();
        assert_eq!(value["mime"], "image/bmp");
        assert_eq!(value["bytes"], 3);
        assert_eq!(value["generation"], 9);
        assert_eq!(value["captured_at_ms"], 42);
        assert_eq!(value["base64"], "Qk0B");
    }

    #[test]
    fn frame_without_a_matching_generation_fails_closed() {
        let frame = SourcePreviewFrame::new(42, vec![1]).unwrap();
        let error = frame_value(status(None), Some(frame)).unwrap_err();
        assert_eq!(error.code, error_codes::CONFLICT);
    }

    #[test]
    fn non_bmp_mailbox_bytes_cannot_cross_the_public_preview_boundary() {
        let frame = SourcePreviewFrame::new(42, vec![1, 2]).unwrap();
        let error = frame_value(status(Some(9)), Some(frame)).unwrap_err();
        assert_eq!(error.code, error_codes::INVALID_ARGS);
    }
}

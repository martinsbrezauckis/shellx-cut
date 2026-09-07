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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedLeaseArgs {
    expected_generation: u64,
    expected_lease_nonce: String,
}

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
    let snapshot = source_preview::snapshot()?;
    Ok(VerbResult::ok(status_value(
        snapshot.status,
        snapshot.unavailable_reason,
        snapshot.lease_nonce.as_deref(),
    )))
}

pub(crate) fn frame_handler(args: Value) -> Result<VerbResult, CutError> {
    let _: EmptyArgs = parse_args(args)?;
    let snapshot = source_preview::snapshot()?;
    Ok(VerbResult::ok(json!({
        "status": status_value(snapshot.status.clone(), snapshot.unavailable_reason, snapshot.lease_nonce.as_deref()),
        "frame": frame_value(snapshot.status, snapshot.frame)?,
    })))
}

pub(crate) fn pause_handler(args: Value) -> Result<VerbResult, CutError> {
    let (generation, nonce) = expected_lease(args)?;
    Ok(action_result(
        "pause",
        source_preview::pause(generation, nonce)?,
    ))
}

pub(crate) fn resume_handler(args: Value) -> Result<VerbResult, CutError> {
    let (generation, nonce) = expected_lease(args)?;
    Ok(action_result(
        "resume",
        source_preview::resume(generation, nonce)?,
    ))
}

pub(crate) fn hide_handler(args: Value) -> Result<VerbResult, CutError> {
    let (generation, nonce) = expected_lease(args)?;
    Ok(action_result(
        "hide",
        source_preview::hide(generation, nonce)?,
    ))
}

pub(crate) fn stop_handler(args: Value) -> Result<VerbResult, CutError> {
    let (generation, nonce) = expected_lease(args)?;
    Ok(action_result(
        "stop",
        source_preview::stop(generation, nonce)?,
    ))
}

fn expected_lease(args: Value) -> Result<(u64, String), CutError> {
    let args: ExpectedLeaseArgs = parse_args(args)?;
    if args.expected_generation == 0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "expected_generation must be a positive native preview generation",
            "generation zero is never issued by the preview lifecycle",
        ));
    }
    if args.expected_lease_nonce.len() != 32
        || !args
            .expected_lease_nonce
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "expected_lease_nonce must be the opaque 128-bit preview lease nonce",
            "use the exact lease_nonce returned by preview_start, preview_status, or preview_frame",
        ));
    }
    Ok((args.expected_generation, args.expected_lease_nonce))
}

fn action_result(action: &str, result: source_preview::PreviewAction) -> VerbResult {
    VerbResult::ok(
        json!({ "action": action, "status": status_value(result.status, None, result.lease_nonce.as_deref()) }),
    )
}

/// Public projection deliberately omits source identities and all private regions.
fn status_value(
    status: SourcePreviewStatus,
    unavailable_reason: Option<&str>,
    lease_nonce: Option<&str>,
) -> Value {
    let mut value = json!({
        "state": status.state,
        "recursion": status.recursion,
        "has_frame": status.has_frame,
        "generation": status.generation,
    });
    if matches!(
        status.state,
        record_capture::source_preview::SourcePreviewState::Unavailable
    ) {
        if let Some(reason) = unavailable_reason {
            value["unavailable_reason"] = Value::String(reason.into());
        }
    }
    if status.generation.is_some() {
        if let Some(lease_nonce) = lease_nonce {
            value["lease_nonce"] = Value::String(lease_nonce.into());
        }
    }
    value
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

    fn unavailable_status() -> SourcePreviewStatus {
        SourcePreviewStatus {
            state: SourcePreviewState::Unavailable,
            recursion: SourcePreviewRecursion::None,
            request: None,
            generation: None,
            has_frame: false,
        }
    }

    #[test]
    fn public_status_omits_the_private_source_identity() {
        let value = status_value(
            status(Some(3)),
            None,
            Some("00112233445566778899aabbccddeeff"),
        );
        assert_eq!(value["generation"], 3);
        assert_eq!(value["lease_nonce"], "00112233445566778899aabbccddeeff");
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

    #[test]
    fn unresolved_release_reason_is_exposed_only_with_the_terminal_status() {
        let value = status_value(unavailable_status(), Some("restart ShellX Cut"), None);
        assert_eq!(value["unavailable_reason"], "restart ShellX Cut");
        assert!(status_value(
            status(Some(3)),
            Some("restart ShellX Cut"),
            Some("00112233445566778899aabbccddeeff")
        )
        .get("unavailable_reason")
        .is_none());
    }

    #[test]
    fn lifecycle_nonce_must_match_the_lowercase_schema_contract() {
        let error = expected_lease(json!({
            "expected_generation": 1,
            "expected_lease_nonce": "00112233445566778899AABBCCDDEEFF",
        }))
        .unwrap_err();

        assert_eq!(error.code, error_codes::INVALID_ARGS);
    }
}

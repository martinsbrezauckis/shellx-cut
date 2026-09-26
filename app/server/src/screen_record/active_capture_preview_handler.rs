//! Read-only projection of pixels tapped from the exact active recording.

use base64::Engine as _;
use cut_core::{error_codes, CutError, VerbResult};
use record_capture::source_preview::MAX_SOURCE_PREVIEW_FRAME_BYTES;
use serde::Deserialize;
use serde_json::{json, Value};

use super::capture_registry;
use crate::dispatch::parse_args;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    capture_id: String,
}

pub(crate) fn live_frame_handler(args: Value) -> Result<VerbResult, CutError> {
    let args: Args = parse_args(args)?;
    let control =
        capture_registry::active_capture_control(&args.capture_id).ok_or_else(not_found)?;
    let snapshot = control.active_preview().snapshot();
    // A terminalized/replaced reservation cannot return its former pixels even
    // if this read raced worker cleanup after the first registry lookup.
    if !capture_registry::active_capture_control(&args.capture_id)
        .is_some_and(|current| current.same_reservation(&control))
    {
        return Err(not_found());
    }
    let frame = match snapshot.frame {
        Some(frame) if matches!(snapshot.state, "ready" | "stale") => {
            let bytes = frame.encoded();
            if bytes.is_empty()
                || bytes.len() > MAX_SOURCE_PREVIEW_FRAME_BYTES
                || !bytes.starts_with(b"BM")
            {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "active recording preview frame is outside its memory bound",
                    "only bounded BMP bytes from the current capture may be returned",
                ));
            }
            json!({
                "mime": "image/bmp",
                "bytes": bytes.len(),
                "generation": snapshot.generation,
                "captured_at_ms": frame.captured_at_ms(),
                "base64": base64::engine::general_purpose::STANDARD.encode(bytes),
            })
        }
        _ => Value::Null,
    };
    Ok(VerbResult::ok(json!({
        "capture_id": args.capture_id,
        "state": snapshot.state,
        "reason": snapshot.reason,
        "generation": snapshot.generation,
        "frame_age_ms": snapshot.frame_age_ms,
        "last_sample_cost_ms": snapshot.last_sample_cost_ms,
        "max_sample_cost_ms": snapshot.max_sample_cost_ms,
        "recursion": snapshot.recursion.as_str(),
        "controller_exclusion": snapshot.controller_exclusion.as_str(),
        "frame": frame,
    })))
}

fn not_found() -> CutError {
    CutError::new(
        error_codes::NOT_FOUND,
        "screen recording is not active in this cutd process",
        "the capture may have finalized or this cutd process may have restarted",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen_record::capture_session_control::CaptureSessionControl;

    #[tokio::test]
    async fn exact_capture_read_is_empty_until_native_frame_and_stale_ids_are_rejected() {
        let _lock = capture_registry::capture_test_lock().lock().await;
        let root = tempfile::tempdir().unwrap();
        let id = format!("cap_live_frame_{}", std::process::id());
        let control = CaptureSessionControl::new(None, false, false, false);
        let reservation = capture_registry::reserve_capture_after_preview_release(
            id.clone(),
            control.clone(),
            Some(root.path().to_path_buf()),
            || Ok(()),
        )
        .unwrap();
        control.active_preview().enable();
        let first = control.active_preview().begin_segment().unwrap();
        let result = live_frame_handler(json!({"capture_id": id}))
            .unwrap()
            .result
            .unwrap();
        assert_eq!(result["state"], "awaiting_frame");
        assert_eq!(result["generation"], first);
        assert!(result["frame"].is_null());
        let second = control.active_preview().begin_segment().unwrap();
        assert!(second > first);
        let result = live_frame_handler(json!({"capture_id": id}))
            .unwrap()
            .result
            .unwrap();
        assert_eq!(result["generation"], second);
        assert!(result["frame"].is_null());
        assert_eq!(
            live_frame_handler(json!({"capture_id": "other-capture"}))
                .unwrap_err()
                .code,
            error_codes::NOT_FOUND
        );
        control.terminalize().unwrap();
        let result = live_frame_handler(json!({"capture_id": id}))
            .unwrap()
            .result
            .unwrap();
        assert_eq!(result["state"], "terminal");
        assert!(result["frame"].is_null());
        drop(reservation);
        assert_eq!(
            live_frame_handler(json!({"capture_id": id}))
                .unwrap_err()
                .code,
            error_codes::NOT_FOUND
        );
    }
}

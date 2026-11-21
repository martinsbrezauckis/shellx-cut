use super::*;

pub(super) fn is_ordinary_marker(dir: &std::path::Path, capture_id: &str) -> bool {
    let Ok(marker) = crate::screen_record::capture_file(dir, capture_id, ".capture.json") else {
        return false;
    };
    let Ok(metadata) = std::fs::metadata(&marker) else {
        return false;
    };
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return false;
    }
    std::fs::read(&marker)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|value| value.get("pause").is_none_or(Value::is_null))
}

// A Stop before the first screen frame may need to report a typed native
// start failure. Keep that path synchronous, as it was before save jobs.
pub(super) async fn has_first_screen_frame(capture_id: &str) -> bool {
    let Ok(active) = crate::screen_record::active_capture::active_handler(json!({})).await else {
        return false;
    };
    active.result.is_some_and(|value| {
        value["capture_id"] == capture_id && value["first_screen_frame_unix_ms"].as_u64().is_some()
    })
}

use cut_core::{error_codes, CutError};

pub(crate) fn validate_capture_id(capture_id: &str) -> Result<(), CutError> {
    let valid = !capture_id.is_empty()
        && capture_id.len() <= 128
        && capture_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        });
    valid.then_some(()).ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "capture_id is not valid",
            "capture_id must be the filesystem-safe id returned by screen_record.start",
        )
        .with_suggested_action("pass the exact capture_id from screen_record.start")
    })
}

use record_core::{error_codes, RecordError};

pub(crate) fn finalization_error(message: &str, cause: &str) -> RecordError {
    RecordError::new(error_codes::CAPTURE, message, cause)
}

//! Server-boundary capture cadence construction.
//!
//! Final media facts are deliberately assembled by `CaptureOutput` from the
//! existing record-recovery verification path; this module never starts a probe.

use record_core::CaptureCadence;

pub(crate) fn from_server_fps(value: f64) -> Result<CaptureCadence, cut_core::CutError> {
    CaptureCadence::from_server_fps(value).map_err(|error| {
        cut_core::CutError::new(
            cut_core::error_codes::INVALID_ARGS,
            "fps must be a finite decimal that can be represented exactly",
            error.to_string(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_decimal_preserves_fractional_request_and_backend_policy() {
        let cadence = from_server_fps(29.97).unwrap();
        assert_eq!(cadence.requested.num, 2_997);
        assert_eq!(cadence.requested.den, 100);
        assert_eq!(cadence.backend_requested.num, 30);
        assert_eq!(cadence.backend_requested.den, 1);
    }
}

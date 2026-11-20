//! Closing a WGC checkpoint encoder that never received a sample.

type Error = Box<dyn std::error::Error + Send + Sync>;

/// A checkpoint reopened just before Stop (for example after a rollover that
/// waited on a busy machine) can close before WGC delivered its first frame.
/// Media Foundation then ends the transcode with MF_E_SINK_NO_SAMPLES_PROCESSED.
/// That is an empty segment, not a failed recording: closing succeeds, and the
/// checkpoint verifier holds the prior checkpoint's last picture over the span.
/// Any other error, or the same code after a sample was offered, still fails.
pub(crate) fn close_result(offered: bool, error: Error) -> Result<Option<()>, Error> {
    let empty = !offered
        && error
            .downcast_ref::<windows::core::Error>()
            .is_some_and(|error| {
                error.code()
                    == windows::Win32::Media::MediaFoundation::MF_E_SINK_NO_SAMPLES_PROCESSED
            });
    if empty {
        Ok(None)
    } else {
        Err(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_segment_that_never_got_a_sample_is_empty() {
        use windows::Win32::Media::MediaFoundation::MF_E_SINK_NO_SAMPLES_PROCESSED;
        let empty = || Error::from(windows::core::Error::from(MF_E_SINK_NO_SAMPLES_PROCESSED));
        assert!(matches!(close_result(false, empty()), Ok(None)));
        assert!(close_result(true, empty()).is_err());
        let other = Error::from(windows::core::Error::from(
            windows::Win32::Foundation::E_FAIL,
        ));
        assert!(close_result(false, other).is_err());
        assert!(close_result(false, "WGC transcoder thread panicked".into()).is_err());
    }
}

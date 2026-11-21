//! One setup boundary shared by the file writer and the in-memory preflight.

use std::time::{Duration, Instant};
use windows::Media::Core::MediaStreamSource;
use windows::Media::MediaProperties::MediaEncodingProfile;
use windows::Media::Transcoding::{MediaTranscoder, PrepareTranscodeResult};
use windows::Storage::Streams::IRandomAccessStream;

#[derive(Debug)]
struct SetupFailure(String);
impl std::fmt::Display for SetupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", crate::VIDEO_ENCODER_UNAVAILABLE, self.0)
    }
}
impl std::error::Error for SetupFailure {}

type Error = Box<dyn std::error::Error + Send + Sync>;
fn failure(error: impl std::fmt::Display) -> Error {
    Box::new(SetupFailure(error.to_string()))
}

pub(crate) fn prepare(
    source: &MediaStreamSource,
    stream: &IRandomAccessStream,
    profile: &MediaEncodingProfile,
    timeout: Option<Duration>,
) -> Result<(MediaTranscoder, PrepareTranscodeResult), Error> {
    let transcoder = MediaTranscoder::new().map_err(failure)?;
    transcoder
        .SetHardwareAccelerationEnabled(true)
        .map_err(failure)?;
    let operation = transcoder
        .PrepareMediaStreamSourceTranscodeAsync(source, stream, profile)
        .map_err(failure)?;
    let prepared = if let Some(timeout) = timeout {
        let deadline = Instant::now() + timeout;
        // WinRT AsyncStatus::Started has value zero. Only the advisory check
        // has a deadline; the actual writer keeps its existing wait policy.
        while operation.Status().map_err(failure)?.0 == 0 {
            if Instant::now() >= deadline {
                let _ = operation.Cancel();
                return Err(failure(
                    "video encoder setup did not finish within five seconds",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        operation.GetResults().map_err(failure)?
    } else {
        operation.join().map_err(failure)?
    };
    if !prepared.CanTranscode().map_err(failure)? {
        return Err(failure(format!(
            "Media Foundation could not prepare this video profile: {:?}",
            prepared.FailureReason().map_err(failure)?
        )));
    }
    Ok((transcoder, prepared))
}

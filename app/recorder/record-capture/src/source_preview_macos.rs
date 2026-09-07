//! ScreenCaptureKit preview retaining only bounded in-memory BMP frames.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use record_core::{error_codes, RecordError, Result};
use screencapturekit::cv::CVPixelBufferLockFlags;
use screencapturekit::error::SCStreamErrorCode;
use screencapturekit::prelude::*;
use screencapturekit::stream::StreamCallbacks;

use crate::source_preview::{SourcePreviewRequest, SourcePreviewSource};
use crate::source_preview_native::{NativeSourcePreviewSession, SourcePreviewMailbox};
use crate::CaptureRegion;

const SOURCE_PREVIEW_ADMISSION_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) fn start(
    request: SourcePreviewRequest,
    generation: u64,
    region: Option<CaptureRegion>,
) -> Result<NativeSourcePreviewSession> {
    if request.camera_id.is_some() {
        return Err(unavailable(
            "selected-camera preview needs its own AVFoundation adapter",
        ));
    }
    if region.is_some() && !matches!(&request.source, SourcePreviewSource::Monitor { .. }) {
        return Err(unavailable(
            "a region preview needs an exact selected display rather than a window",
        ));
    }
    let mailbox = SourcePreviewMailbox::default();
    let stop = Arc::new(AtomicBool::new(false));
    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let mailbox_thread = mailbox.clone();
    let stop_thread = stop.clone();
    let join = thread::spawn(move || {
        let result = run(
            request,
            region,
            mailbox_thread.clone(),
            stop_thread,
            &started_tx,
        );
        if let Err(error) = &result {
            mailbox_thread.mark_unavailable();
            let _ = started_tx.send(Err(error.clone()));
        }
        result
    });
    match started_rx.recv_timeout(SOURCE_PREVIEW_ADMISSION_TIMEOUT) {
        Ok(Ok(())) => Ok(NativeSourcePreviewSession::new(
            generation,
            mailbox,
            move || {
                stop.store(true, Ordering::Release);
                join.join().map_err(|_| {
                    capture_error("join ScreenCaptureKit preview", "worker panicked")
                })??;
                Ok(())
            },
        )),
        Ok(Err(error)) => {
            let _ = join.join();
            Err(error)
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // The worker owns all SCK values. Dropping its join handle here is
            // safe: the stop flag plus disconnected channel make a late worker
            // stop any just-started stream before it can become a session.
            stop.store(true, Ordering::Release);
            Err(admission_timeout())
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = join.join();
            Err(capture_error(
                "start ScreenCaptureKit preview",
                "worker exited before admission",
            ))
        }
    }
}

fn run(
    request: SourcePreviewRequest,
    region: Option<CaptureRegion>,
    mailbox: SourcePreviewMailbox,
    stop: Arc<AtomicBool>,
    started_tx: &mpsc::SyncSender<Result<()>>,
) -> Result<()> {
    ensure_not_stopped(&stop)?;
    crate::macos::sck_init_cg();
    let content = SCShareableContent::get()
        .map_err(|error| sck_error("enumerate ScreenCaptureKit sources", error))?;
    ensure_not_stopped(&stop)?;
    let filter = match request.source {
        SourcePreviewSource::Monitor { monitor_id } => {
            let displays = content.displays();
            let display = displays
                .iter()
                .find(|display| {
                    crate::macos_monitor_target::monitor_id(display).as_deref() == Some(&monitor_id)
                })
                .ok_or_else(|| {
                    capture_error(
                        "resolve selected display",
                        "selected display is no longer available",
                    )
                })?;
            SCContentFilter::create()
                .with_display(display)
                .with_excluding_windows(&[])
                .build()
        }
        SourcePreviewSource::Window { window_id } => {
            let id = crate::window_target::parse_macos_window_id(&window_id).ok_or_else(|| {
                capture_error("resolve selected window", "selected window id is malformed")
            })?;
            let windows = content.windows();
            let window = windows
                .iter()
                .find(|window| window.window_id() == id)
                .ok_or_else(|| {
                    capture_error(
                        "resolve selected window",
                        "selected window is no longer available",
                    )
                })?;
            SCContentFilter::create().with_window(window).build()
        }
        SourcePreviewSource::Portal => return Err(unavailable("portal source selection on macOS")),
    };
    ensure_not_stopped(&stop)?;
    let mut config = SCStreamConfiguration::new();
    config
        .set_pixel_format(PixelFormat::BGRA)
        .set_queue_depth(2)
        .set_shows_cursor(false);
    let terminal_mailbox = mailbox.clone();
    let terminal_stop = stop.clone();
    let delegate = StreamCallbacks::new().on_error(move |error| {
        mark_stream_terminal(&terminal_mailbox, error);
        terminal_stop.store(true, Ordering::Release);
    });
    let mut stream = SCStream::new_with_delegate(&filter, &config, delegate);
    let frame_mailbox = mailbox.clone();
    let started = Instant::now();
    stream
        .add_output_handler(
            move |sample: screencapturekit::cm::CMSampleBuffer, output_type| {
                if output_type != SCStreamOutputType::Screen {
                    return;
                }
                let Some(buffer) = sample.image_buffer() else {
                    return;
                };
                let Ok(guard) = buffer.lock(CVPixelBufferLockFlags::READ_ONLY) else {
                    frame_mailbox.mark_unavailable();
                    return;
                };
                let accepted = frame_mailbox.publish_bgra(
                    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                    u32::try_from(guard.width()).unwrap_or(0),
                    u32::try_from(guard.height()).unwrap_or(0),
                    guard.bytes_per_row(),
                    guard.as_slice(),
                    region,
                );
                if accepted.is_err() {
                    frame_mailbox.mark_unavailable();
                }
            },
            SCStreamOutputType::Screen,
        )
        .ok_or_else(|| {
            capture_error(
                "attach ScreenCaptureKit preview callback",
                "ScreenCaptureKit rejected screen output",
            )
        })?;
    stream
        .start_capture()
        .map_err(|error| sck_error("start ScreenCaptureKit preview", error))?;
    if stop.load(Ordering::Acquire) || started_tx.send(Ok(())).is_err() {
        stop.store(true, Ordering::Release);
        stop_stream(&mut stream)?;
        return Err(admission_cancelled());
    }
    while !stop.load(Ordering::Acquire) {
        thread::sleep(Duration::from_millis(25));
    }
    stop_stream(&mut stream)
}

fn ensure_not_stopped(stop: &AtomicBool) -> Result<()> {
    if stop.load(Ordering::Acquire) {
        Err(admission_cancelled())
    } else {
        Ok(())
    }
}

fn stop_stream(stream: &mut SCStream) -> Result<()> {
    stream
        .stop_capture()
        .map_err(|error| sck_error("stop ScreenCaptureKit preview", error))
}

fn mark_stream_terminal(mailbox: &SourcePreviewMailbox, error: SCError) {
    if is_permission_denied(&error) {
        mailbox.mark_permission_denied();
    } else if is_source_lost(&error) {
        mailbox.mark_source_lost();
    } else {
        mailbox.mark_unavailable();
    }
}

fn is_permission_denied(error: &SCError) -> bool {
    matches!(
        error,
        SCError::PermissionDenied(_)
            | SCError::SCStreamError {
                code: SCStreamErrorCode::UserDeclined,
                ..
            }
    )
}

fn is_source_lost(error: &SCError) -> bool {
    matches!(
        error,
        SCError::DisplayNotFound(_)
            | SCError::WindowNotFound(_)
            | SCError::SCStreamError {
                code: SCStreamErrorCode::NoWindowList
                    | SCStreamErrorCode::NoDisplayList
                    | SCStreamErrorCode::NoCaptureSource
                    | SCStreamErrorCode::UserStopped
                    | SCStreamErrorCode::SystemStoppedStream,
                ..
            }
    )
}

fn sck_error(context: &str, error: SCError) -> RecordError {
    if is_permission_denied(&error) {
        RecordError::new(
            error_codes::CAPTURE,
            "screen-recording permission denied",
            error.to_string(),
        )
        .with_action("allow Screen Recording for ShellX Cut in macOS privacy settings, then retry")
    } else {
        capture_error(context, error)
    }
}

fn admission_timeout() -> RecordError {
    RecordError::new(
        error_codes::CAPTURE,
        "ScreenCaptureKit preview admission timed out",
        "no native preview session was admitted within 15 seconds",
    )
    .with_action("check screen-recording permission and retry source preview")
}

fn admission_cancelled() -> RecordError {
    RecordError::new(
        error_codes::CAPTURE,
        "ScreenCaptureKit preview admission was cancelled",
        "the caller released preview ownership before native admission completed",
    )
}

fn capture_error(context: &str, detail: impl std::fmt::Display) -> RecordError {
    RecordError::new(error_codes::CAPTURE, context, detail.to_string())
}

fn unavailable(detail: &str) -> RecordError {
    RecordError::new(
        error_codes::UNIMPLEMENTED,
        "native source preview is unavailable",
        detail,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_screen_capturekit_failures_keep_permission_and_source_loss_distinct() {
        assert!(is_permission_denied(&SCError::PermissionDenied(
            "TCC".into()
        )));
        assert!(is_permission_denied(&SCError::SCStreamError {
            code: SCStreamErrorCode::UserDeclined,
            message: None,
        }));
        assert!(is_source_lost(&SCError::SCStreamError {
            code: SCStreamErrorCode::NoCaptureSource,
            message: None,
        }));
        assert!(!is_source_lost(&SCError::CaptureStartFailed(
            "GPU failure".into()
        )));
    }

    #[test]
    fn timed_out_admission_has_a_bounded_actionable_error() {
        let error = admission_timeout();
        assert_eq!(error.code, error_codes::CAPTURE);
        assert!(error.message.contains("timed out"));
        assert!(error.suggested_action.is_some());
    }
}

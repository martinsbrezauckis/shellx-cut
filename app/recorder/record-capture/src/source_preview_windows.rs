//! Windows Graphics Capture preview that retains only a bounded in-memory BMP.

use std::error::Error;
use std::time::Instant;

use record_core::{error_codes, RecordError, Result};
use windows_capture::{
    capture::{Context, GraphicsCaptureApiHandler},
    frame::Frame,
    graphics_capture_api::InternalCaptureControl,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    },
    window::Window,
};

use crate::source_preview::{SourcePreviewRequest, SourcePreviewSource};
use crate::source_preview_native::{NativeSourcePreviewSession, SourcePreviewMailbox};
use crate::CaptureRegion;

#[derive(Clone)]
struct Flags {
    mailbox: SourcePreviewMailbox,
    region: Option<CaptureRegion>,
}

struct Handler {
    mailbox: SourcePreviewMailbox,
    region: Option<CaptureRegion>,
    started: Instant,
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = Flags;
    type Error = Box<dyn Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> std::result::Result<Self, Self::Error> {
        Ok(Self {
            mailbox: ctx.flags.mailbox,
            region: ctx.flags.region,
            started: Instant::now(),
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _control: InternalCaptureControl,
    ) -> std::result::Result<(), Self::Error> {
        let captured_at_ms = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let mut buffer = match frame.buffer() {
            Ok(buffer) => buffer,
            Err(error) => {
                // A WGC buffer/readback failure is an adapter failure, not proof
                // that the user-selected monitor or window disappeared.
                self.mailbox.mark_unavailable();
                return Err(Box::new(error));
            }
        };
        let result = self.mailbox.publish_bgra(
            captured_at_ms,
            buffer.width(),
            buffer.height(),
            usize::try_from(buffer.row_pitch()).unwrap_or(usize::MAX),
            buffer.as_raw_buffer(),
            self.region,
        );
        if result.is_err() {
            self.mailbox.mark_unavailable();
        }
        result.map_err(Into::into)
    }

    fn on_closed(&mut self) -> std::result::Result<(), Self::Error> {
        self.mailbox.mark_source_lost();
        Ok(())
    }
}

pub(crate) fn start(
    request: SourcePreviewRequest,
    generation: u64,
    region: Option<CaptureRegion>,
) -> Result<NativeSourcePreviewSession> {
    if request.camera_id.is_some() {
        return Err(unavailable(
            "selected-camera preview needs its own Media Foundation adapter",
        ));
    }
    if region.is_some() && !matches!(&request.source, SourcePreviewSource::Monitor { .. }) {
        return Err(unavailable(
            "a region preview needs an exact selected display rather than a window",
        ));
    }
    let mailbox = SourcePreviewMailbox::default();
    let control = match request.source {
        SourcePreviewSource::Monitor { monitor_id } => {
            let monitor = crate::windows_monitor_target::resolve_monitor(&monitor_id)
                .map_err(|detail| capture_error("resolve selected display", detail))?;
            Handler::start_free_threaded(Settings::new(
                monitor,
                CursorCaptureSettings::WithoutCursor,
                DrawBorderSettings::Default,
                SecondaryWindowSettings::Default,
                MinimumUpdateIntervalSettings::Default,
                DirtyRegionSettings::Default,
                ColorFormat::Bgra8,
                Flags {
                    mailbox: mailbox.clone(),
                    region,
                },
            ))
        }
        SourcePreviewSource::Window { window_id } => {
            let hwnd = crate::windows_picker::resolve_window(&window_id)
                .map_err(|detail| capture_error("resolve selected window", detail))?;
            Handler::start_free_threaded(Settings::new(
                Window::from_raw_hwnd(hwnd.0),
                CursorCaptureSettings::WithoutCursor,
                DrawBorderSettings::Default,
                SecondaryWindowSettings::Default,
                MinimumUpdateIntervalSettings::Default,
                DirtyRegionSettings::Default,
                ColorFormat::Bgra8,
                Flags {
                    mailbox: mailbox.clone(),
                    region,
                },
            ))
        }
        SourcePreviewSource::Portal => {
            return Err(unavailable("portal source selection on Windows"))
        }
    }
    .map_err(|error| capture_error("start Windows Graphics Capture preview", error))?;
    Ok(NativeSourcePreviewSession::new(
        generation,
        mailbox,
        move || {
            control
                .stop()
                .map_err(|error| capture_error("stop Windows Graphics Capture preview", error))
        },
    ))
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

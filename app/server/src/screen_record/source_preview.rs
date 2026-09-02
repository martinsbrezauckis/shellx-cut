//! Private server owner for the in-memory native source-preview session.

use std::sync::{Mutex, OnceLock};

use cut_core::{error_codes, CutError};
use record_capture::source_preview::{
    SourcePreviewCommand, SourcePreviewLifecycle, SourcePreviewPlatform, SourcePreviewRequest,
    SourcePreviewSource, SourcePreviewStatus,
};
use record_capture::source_preview_native::{
    NativeSourcePreviewSession, SourcePreviewCapability, SourcePreviewMailboxTerminal,
};
use record_capture::CaptureRegion;

use super::{capture_registry, record_err};

struct PreviewOwner {
    lifecycle: SourcePreviewLifecycle,
    native: Option<NativeSourcePreviewSession>,
    region: Option<CaptureRegion>,
}

impl PreviewOwner {
    fn new() -> Self {
        Self {
            lifecycle: SourcePreviewLifecycle::new(platform()),
            native: None,
            region: None,
        }
    }

    fn start(&mut self, request: SourcePreviewRequest) -> Result<SourcePreviewStatus, CutError> {
        self.start_with_region(request, None)
    }

    /// Coordinates are private native state, never a server argument or response.
    fn start_with_region(
        &mut self,
        request: SourcePreviewRequest,
        region: Option<CaptureRegion>,
    ) -> Result<SourcePreviewStatus, CutError> {
        if capture_registry::has_active_capture() {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "source preview is unavailable while a recording owns capture devices",
                "stop or finalize the active recording before starting a native preview",
            ));
        }
        if region.is_some() && !matches!(&request.source, SourcePreviewSource::Monitor { .. }) {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "a region preview requires an exact selected display",
                "window and portal previews cannot safely apply desktop coordinates",
            ));
        }
        let commands = self.lifecycle.start(request).map_err(record_err)?;
        self.region = region;
        if let Err(error) = self.apply(commands) {
            self.region = None;
            return Err(error);
        }
        Ok(self.lifecycle.status())
    }

    fn apply(&mut self, commands: Vec<SourcePreviewCommand>) -> Result<(), CutError> {
        for command in commands {
            match command {
                SourcePreviewCommand::Stop => self.stop_native()?,
                SourcePreviewCommand::Start {
                    generation,
                    request,
                } => match record_capture::source_preview_native::start(
                    request,
                    generation,
                    self.region,
                ) {
                    Ok(session) => self.native = Some(session),
                    Err(error) => {
                        self.lifecycle.unavailable();
                        return Err(record_err(error));
                    }
                },
            }
        }
        Ok(())
    }

    fn poll(&mut self) -> Result<(), CutError> {
        let Some(session) = self.native.as_ref() else {
            return Ok(());
        };
        if let Some(terminal) = session.terminal() {
            match terminal {
                SourcePreviewMailboxTerminal::SourceLost => {
                    self.lifecycle.source_lost();
                }
                SourcePreviewMailboxTerminal::Unavailable => {
                    self.lifecycle.unavailable();
                }
                SourcePreviewMailboxTerminal::PermissionDenied => {
                    self.lifecycle.permission_denied();
                }
            }
            self.stop_native()?;
            self.region = None;
            return Ok(());
        }
        if let Some(frame) = session.latest_frame() {
            self.lifecycle
                .accept_frame(session.generation, frame)
                .map_err(record_err)?;
        }
        Ok(())
    }

    /// The lifecycle owns the accepted frame: callers must not bypass its
    /// generation and 10 fps gates by reading the native mailbox directly.
    fn snapshot(
        &mut self,
    ) -> Result<
        (
            SourcePreviewStatus,
            Option<record_capture::source_preview::SourcePreviewFrame>,
        ),
        CutError,
    > {
        self.poll()?;
        Ok((
            self.lifecycle.status(),
            self.lifecycle.latest_frame().cloned(),
        ))
    }

    fn stop(&mut self) -> Result<SourcePreviewStatus, CutError> {
        let commands = self.lifecycle.stop();
        self.apply(commands)?;
        self.region = None;
        Ok(self.lifecycle.status())
    }

    fn pause(&mut self) -> Result<SourcePreviewStatus, CutError> {
        let commands = self.lifecycle.pause();
        self.apply(commands)?;
        Ok(self.lifecycle.status())
    }

    fn resume(&mut self) -> Result<SourcePreviewStatus, CutError> {
        let commands = self.lifecycle.resume().map_err(record_err)?;
        self.apply(commands)?;
        Ok(self.lifecycle.status())
    }

    fn hide(&mut self) -> Result<SourcePreviewStatus, CutError> {
        let commands = self.lifecycle.hide();
        self.apply(commands)?;
        self.region = None;
        Ok(self.lifecycle.status())
    }

    fn release_for_recording(&mut self) -> Result<(), CutError> {
        let commands = self.lifecycle.release_for_recording();
        self.apply(commands)?;
        self.region = None;
        Ok(())
    }

    fn stop_native(&mut self) -> Result<(), CutError> {
        if let Some(mut session) = self.native.take() {
            session.stop().map_err(record_err)?;
        }
        Ok(())
    }
}

static OWNER: OnceLock<Mutex<PreviewOwner>> = OnceLock::new();

fn owner() -> &'static Mutex<PreviewOwner> {
    OWNER.get_or_init(|| Mutex::new(PreviewOwner::new()))
}

pub(crate) fn capability() -> SourcePreviewCapability {
    record_capture::source_preview_native::capability()
}

pub(crate) fn start(request: SourcePreviewRequest) -> Result<SourcePreviewStatus, CutError> {
    owner()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .start(request)
}

/// Read the status and current bytes under one owner lock so a replacement
/// cannot pair a prior generation's status with a later source's pixels.
pub(crate) fn snapshot() -> Result<
    (
        SourcePreviewStatus,
        Option<record_capture::source_preview::SourcePreviewFrame>,
    ),
    CutError,
> {
    let mut owner = owner()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    owner.snapshot()
}

pub(crate) fn stop() -> Result<SourcePreviewStatus, CutError> {
    owner()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .stop()
}

pub(crate) fn pause() -> Result<SourcePreviewStatus, CutError> {
    owner()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .pause()
}

pub(crate) fn resume() -> Result<SourcePreviewStatus, CutError> {
    owner()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .resume()
}

pub(crate) fn hide() -> Result<SourcePreviewStatus, CutError> {
    owner()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .hide()
}

pub(crate) fn release_for_recording() -> Result<(), CutError> {
    owner()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .release_for_recording()
}

fn platform() -> SourcePreviewPlatform {
    #[cfg(windows)]
    return SourcePreviewPlatform::Windows;
    #[cfg(target_os = "macos")]
    return SourcePreviewPlatform::Macos;
    #[allow(unreachable_code)]
    SourcePreviewPlatform::Linux
}

#[cfg(test)]
#[path = "source_preview_tests.rs"]
mod tests;

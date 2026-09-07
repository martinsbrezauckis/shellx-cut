//! Private server owner for the in-memory native source-preview session.

use std::sync::{Mutex, OnceLock};

use cut_core::{error_codes, CutError};
use record_capture::source_preview::{
    SourcePreviewCommand, SourcePreviewLifecycle, SourcePreviewPlatform, SourcePreviewRequest,
    SourcePreviewSource,
};
use record_capture::source_preview_native::{
    NativeSourcePreviewSession, SourcePreviewCapability, SourcePreviewMailboxTerminal,
};
use record_capture::CaptureRegion;

use super::{capture_registry, record_err};

#[path = "source_preview_lease.rs"]
mod lease;
pub(crate) use lease::PreviewAction;
use lease::{PreviewLease, PreviewSnapshot, UNRESOLVED_RELEASE_REASON};

struct PreviewOwner {
    lifecycle: SourcePreviewLifecycle,
    native: Option<NativeSourcePreviewSession>,
    region: Option<CaptureRegion>,
    lease: PreviewLease,
    /// A native adapter returned a close failure after consuming its only
    /// teardown operation. Do not claim the devices again in this process.
    teardown_failed: bool,
}

impl PreviewOwner {
    fn new() -> Self {
        Self {
            lifecycle: SourcePreviewLifecycle::new(platform()),
            native: None,
            region: None,
            lease: PreviewLease::default(),
            teardown_failed: false,
        }
    }

    fn start(&mut self, request: SourcePreviewRequest) -> Result<PreviewAction, CutError> {
        self.start_with_region(request, None)
    }

    /// Coordinates are private native state, never a server argument or response.
    fn start_with_region(
        &mut self,
        request: SourcePreviewRequest,
        region: Option<CaptureRegion>,
    ) -> Result<PreviewAction, CutError> {
        self.ensure_teardown_recovered()?;
        self.lease.ensure_minted()?;
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
        Ok(self.action())
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
    fn snapshot(&mut self) -> Result<PreviewSnapshot, CutError> {
        self.poll()?;
        Ok(PreviewSnapshot {
            status: self.lifecycle.status(),
            frame: self.lifecycle.latest_frame().cloned(),
            unavailable_reason: self.teardown_failed.then_some(UNRESOLVED_RELEASE_REASON),
            lease_nonce: self.lease.nonce_for(&self.lifecycle.status()),
        })
    }

    fn stop(
        &mut self,
        expected_generation: u64,
        expected_lease_nonce: &str,
    ) -> Result<PreviewAction, CutError> {
        self.ensure_lease(expected_generation, expected_lease_nonce)?;
        let commands = self.lifecycle.stop();
        self.apply(commands)?;
        self.region = None;
        Ok(self.action())
    }

    fn pause(
        &mut self,
        expected_generation: u64,
        expected_lease_nonce: &str,
    ) -> Result<PreviewAction, CutError> {
        self.ensure_lease(expected_generation, expected_lease_nonce)?;
        let commands = self.lifecycle.pause();
        self.apply(commands)?;
        Ok(self.action())
    }

    fn resume(
        &mut self,
        expected_generation: u64,
        expected_lease_nonce: &str,
    ) -> Result<PreviewAction, CutError> {
        self.ensure_lease(expected_generation, expected_lease_nonce)?;
        let commands = self.lifecycle.resume().map_err(record_err)?;
        self.apply(commands)?;
        Ok(self.action())
    }

    fn hide(
        &mut self,
        expected_generation: u64,
        expected_lease_nonce: &str,
    ) -> Result<PreviewAction, CutError> {
        self.ensure_lease(expected_generation, expected_lease_nonce)?;
        let commands = self.lifecycle.hide();
        self.apply(commands)?;
        self.region = None;
        Ok(self.action())
    }

    fn release_for_recording(&mut self) -> Result<(), CutError> {
        self.ensure_teardown_recovered()?;
        let commands = self.lifecycle.release_for_recording();
        self.apply(commands)?;
        self.region = None;
        Ok(())
    }

    fn stop_native(&mut self) -> Result<(), CutError> {
        if let Some(mut session) = self.native.take() {
            if let Err(error) = session.stop() {
                // `NativeSourcePreviewSession::stop` consumes the one-shot
                // native stop callback. Its Drop path therefore cannot prove a
                // later retry succeeded. Keep the process fail-closed instead
                // of reporting a released preview or admitting a recording.
                self.teardown_failed = true;
                self.lifecycle.unavailable();
                self.region = None;
                return Err(record_err(error).with_suggested_action(
                    "restart ShellX Cut (or the Cut server) before starting another preview or recording",
                ));
            }
        }
        Ok(())
    }

    fn action(&self) -> PreviewAction {
        let status = self.lifecycle.status();
        PreviewAction {
            lease_nonce: self.lease.nonce_for(&status),
            status,
        }
    }

    fn ensure_lease(
        &self,
        expected_generation: u64,
        expected_lease_nonce: &str,
    ) -> Result<(), CutError> {
        self.ensure_teardown_recovered()?;
        if !self.lease.matches(expected_lease_nonce) {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "source preview lease belongs to a different Cut server",
                "the supplied opaque preview lease nonce does not match this process owner",
            )
            .with_suggested_action(
                "read screen_record.preview_status and use its current lease_nonce before sending another lifecycle action",
            ));
        }
        let actual_generation = self.lifecycle.status().generation;
        if actual_generation == Some(expected_generation) {
            return Ok(());
        }
        Err(CutError::new(
            error_codes::CONFLICT,
            "source preview generation no longer owns the native session",
            format!(
                "expected generation {expected_generation}, current generation is {}",
                actual_generation
                    .map(|generation| generation.to_string())
                    .unwrap_or_else(|| "none".into())
            ),
        )
        .with_suggested_action(
            "read screen_record.preview_status before sending another lifecycle action",
        ))
    }

    fn ensure_teardown_recovered(&self) -> Result<(), CutError> {
        if !self.teardown_failed {
            return Ok(());
        }
        Err(CutError::new(
            error_codes::CONFLICT,
            "native source preview release is unresolved",
            "a native preview close failed after its teardown operation was consumed",
        )
        .with_suggested_action(
            "restart ShellX Cut (or the Cut server) before starting another preview or recording",
        ))
    }
}

static OWNER: OnceLock<Mutex<PreviewOwner>> = OnceLock::new();

fn owner() -> &'static Mutex<PreviewOwner> {
    OWNER.get_or_init(|| Mutex::new(PreviewOwner::new()))
}

fn with_owner<T>(
    action: impl FnOnce(&mut PreviewOwner) -> Result<T, CutError>,
) -> Result<T, CutError> {
    let mut owner = owner()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    action(&mut owner)
}

pub(crate) fn capability() -> SourcePreviewCapability {
    record_capture::source_preview_native::capability()
}

pub(crate) fn start(request: SourcePreviewRequest) -> Result<PreviewAction, CutError> {
    // The gate must precede OWNER. Capture handoff takes the same order before
    // it reserves capture and locks OWNER to release preview; reversing these
    // two locks would deadlock competing preview and recording starts.
    let _admission_gate = capture_registry::preview_capture_gate();
    with_owner(|owner| owner.start(request))
}

/// Read the status and current bytes under one owner lock so a replacement
/// cannot pair a prior generation's status with a later source's pixels.
pub(crate) fn snapshot() -> Result<PreviewSnapshot, CutError> {
    with_owner(PreviewOwner::snapshot)
}

pub(crate) fn stop(
    expected_generation: u64,
    expected_lease_nonce: String,
) -> Result<PreviewAction, CutError> {
    with_owner(|owner| owner.stop(expected_generation, &expected_lease_nonce))
}

pub(crate) fn pause(
    expected_generation: u64,
    expected_lease_nonce: String,
) -> Result<PreviewAction, CutError> {
    with_owner(|owner| owner.pause(expected_generation, &expected_lease_nonce))
}

pub(crate) fn resume(
    expected_generation: u64,
    expected_lease_nonce: String,
) -> Result<PreviewAction, CutError> {
    with_owner(|owner| owner.resume(expected_generation, &expected_lease_nonce))
}

pub(crate) fn hide(
    expected_generation: u64,
    expected_lease_nonce: String,
) -> Result<PreviewAction, CutError> {
    with_owner(|owner| owner.hide(expected_generation, &expected_lease_nonce))
}

pub(crate) fn release_for_recording() -> Result<(), CutError> {
    with_owner(PreviewOwner::release_for_recording)
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

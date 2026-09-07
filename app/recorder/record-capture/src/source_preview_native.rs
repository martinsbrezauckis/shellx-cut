//! In-process native source-preview session and latest-frame mailbox.

use std::sync::{Arc, Mutex};

use record_core::{error_codes, RecordError, Result};
use serde::Serialize;

use crate::source_preview::{SourcePreviewFrame, SourcePreviewRequest};
#[cfg(all(target_os = "linux", feature = "capture-linux"))]
use crate::source_preview_bitmap::SourcePreviewPixelFormat;
use crate::CaptureRegion;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SourcePreviewCapability {
    Available {
        /// `exact` accepts only the current opaque monitor/window id; `portal`
        /// requires a fresh user choice in the Linux system picker.
        source_selection: SourcePreviewSelectionMode,
    },
    Unsupported {
        prerequisite: String,
    },
}

/// A capability fact, not a browser or host-platform inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourcePreviewSelectionMode {
    Exact,
    Portal,
}

#[derive(Clone, Debug, Default)]
pub struct SourcePreviewMailbox(Arc<Mutex<SourcePreviewMailboxState>>);

#[derive(Clone, Debug, Default)]
struct SourcePreviewMailboxState {
    frame: Option<SourcePreviewFrame>,
    terminal: Option<SourcePreviewMailboxTerminal>,
}

/// Native adapters may report only an observed source closure or an adapter
/// failure. A platform's explicit denial remains distinct from either one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourcePreviewMailboxTerminal {
    SourceLost,
    Unavailable,
    PermissionDenied,
}

#[cfg(all(target_os = "linux", feature = "capture-linux"))]
pub(crate) struct NativeSourcePreviewPixels<'a> {
    pub captured_at_ms: u64,
    pub width: u32,
    pub height: u32,
    pub format: SourcePreviewPixelFormat,
    pub stride: usize,
    pub pixels: &'a [u8],
    pub offset: usize,
}

impl SourcePreviewMailbox {
    #[cfg_attr(
        all(target_os = "linux", not(feature = "source-preview-test-support")),
        allow(
            dead_code,
            reason = "PipeWire uses the generic pixel path; BGRA callbacks belong to Windows/macOS adapters"
        )
    )]
    pub(crate) fn publish_bgra(
        &self,
        captured_at_ms: u64,
        width: u32,
        height: u32,
        stride: usize,
        pixels: &[u8],
        region: Option<CaptureRegion>,
    ) -> Result<()> {
        let frame = SourcePreviewFrame::new(
            captured_at_ms,
            crate::source_preview_bitmap::bgra_bmp(width, height, stride, pixels, region)?,
        )?;
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .frame = Some(frame);
        Ok(())
    }

    #[cfg(all(target_os = "linux", feature = "capture-linux"))]
    pub(crate) fn publish_native_pixels(
        &self,
        pixels: NativeSourcePreviewPixels<'_>,
    ) -> Result<()> {
        let frame = SourcePreviewFrame::new(
            pixels.captured_at_ms,
            crate::source_preview_bitmap::native_bmp(
                pixels.width,
                pixels.height,
                pixels.format,
                pixels.stride,
                pixels.pixels,
                pixels.offset,
                None,
            )?,
        )?;
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .frame = Some(frame);
        Ok(())
    }

    pub fn latest(&self) -> Option<SourcePreviewFrame> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .frame
            .clone()
    }

    #[cfg_attr(
        target_os = "linux",
        allow(
            dead_code,
            reason = "only Windows/macOS adapters report a formerly admitted source closing"
        )
    )]
    pub(crate) fn mark_source_lost(&self) {
        self.mark_terminal(SourcePreviewMailboxTerminal::SourceLost);
    }

    #[cfg(any(
        test,
        all(windows, feature = "capture-windows"),
        all(unix, any(feature = "capture-macos", feature = "capture-linux"))
    ))]
    pub(crate) fn mark_unavailable(&self) {
        self.mark_terminal(SourcePreviewMailboxTerminal::Unavailable);
    }

    #[cfg(any(
        test,
        feature = "source-preview-test-support",
        all(target_os = "macos", feature = "capture-macos")
    ))]
    pub(crate) fn mark_permission_denied(&self) {
        self.mark_terminal(SourcePreviewMailboxTerminal::PermissionDenied);
    }

    fn mark_terminal(&self, terminal: SourcePreviewMailboxTerminal) {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.terminal.get_or_insert(terminal);
    }

    pub fn terminal(&self) -> Option<SourcePreviewMailboxTerminal> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .terminal
    }

    pub fn source_lost(&self) -> bool {
        self.terminal() == Some(SourcePreviewMailboxTerminal::SourceLost)
    }

    pub fn unavailable(&self) -> bool {
        self.terminal() == Some(SourcePreviewMailboxTerminal::Unavailable)
    }

    pub fn permission_denied(&self) -> bool {
        self.terminal() == Some(SourcePreviewMailboxTerminal::PermissionDenied)
    }

    pub(crate) fn clear(&self) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .frame = None;
    }
}

pub struct NativeSourcePreviewSession {
    pub generation: u64,
    mailbox: SourcePreviewMailbox,
    stop: Option<Box<dyn FnOnce() -> Result<()> + Send>>,
}

impl NativeSourcePreviewSession {
    #[cfg(any(
        test,
        feature = "source-preview-test-support",
        all(windows, feature = "capture-windows"),
        all(unix, any(feature = "capture-macos", feature = "capture-linux"))
    ))]
    pub(crate) fn new(
        generation: u64,
        mailbox: SourcePreviewMailbox,
        stop: impl FnOnce() -> Result<()> + Send + 'static,
    ) -> Self {
        Self {
            generation,
            mailbox,
            stop: Some(Box::new(stop)),
        }
    }

    pub fn latest_frame(&self) -> Option<SourcePreviewFrame> {
        self.mailbox.latest()
    }

    pub fn source_lost(&self) -> bool {
        self.mailbox.source_lost()
    }

    pub fn unavailable(&self) -> bool {
        self.mailbox.unavailable()
    }

    pub fn terminal(&self) -> Option<SourcePreviewMailboxTerminal> {
        self.mailbox.terminal()
    }

    pub fn permission_denied(&self) -> bool {
        self.mailbox.permission_denied()
    }

    pub fn stop(&mut self) -> Result<()> {
        self.mailbox.clear();
        self.stop
            .take()
            .map(|stop| stop())
            .transpose()?
            .unwrap_or(());
        Ok(())
    }
}

impl Drop for NativeSourcePreviewSession {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Cross-crate server test injector. It cannot open native capture or write a
/// path; its bounded fake mailbox exists only with the explicit test-support
/// feature.
#[cfg(feature = "source-preview-test-support")]
#[doc(hidden)]
pub mod test_support {
    use super::*;

    #[doc(hidden)]
    pub struct SourcePreviewTestMailbox {
        mailbox: SourcePreviewMailbox,
    }

    impl Default for SourcePreviewTestMailbox {
        fn default() -> Self {
            Self::new()
        }
    }

    impl SourcePreviewTestMailbox {
        pub fn new() -> Self {
            Self {
                mailbox: SourcePreviewMailbox::default(),
            }
        }

        pub fn session(&self, generation: u64) -> NativeSourcePreviewSession {
            NativeSourcePreviewSession::new(generation, self.mailbox.clone(), || Ok(()))
        }

        pub fn failing_stop_session(&self, generation: u64) -> NativeSourcePreviewSession {
            NativeSourcePreviewSession::new(generation, self.mailbox.clone(), || {
                Err(RecordError::new(
                    error_codes::CAPTURE,
                    "stop native source preview",
                    "test native close failure",
                ))
            })
        }

        pub fn publish_bgra(
            &self,
            captured_at_ms: u64,
            width: u32,
            height: u32,
            stride: usize,
            pixels: &[u8],
        ) -> Result<()> {
            self.mailbox
                .publish_bgra(captured_at_ms, width, height, stride, pixels, None)
        }

        pub fn mark_permission_denied(&self) {
            self.mailbox.mark_permission_denied();
        }
    }
}

pub fn capability() -> SourcePreviewCapability {
    #[cfg(all(windows, feature = "capture-windows"))]
    {
        return SourcePreviewCapability::Available {
            source_selection: SourcePreviewSelectionMode::Exact,
        };
    }
    #[cfg(all(target_os = "macos", feature = "capture-macos"))]
    {
        return SourcePreviewCapability::Available {
            source_selection: SourcePreviewSelectionMode::Exact,
        };
    }
    #[cfg(all(target_os = "linux", feature = "capture-linux"))]
    {
        return SourcePreviewCapability::Available {
            source_selection: SourcePreviewSelectionMode::Portal,
        };
    }
    #[allow(unreachable_code)]
    SourcePreviewCapability::Unsupported {
        prerequisite: "this build needs a reviewed in-process native preview adapter; Linux requires the capture-linux portal/PipeWire preview owner".into(),
    }
}

pub fn start(
    request: SourcePreviewRequest,
    generation: u64,
    region: Option<CaptureRegion>,
) -> Result<NativeSourcePreviewSession> {
    #[cfg(all(windows, feature = "capture-windows"))]
    {
        return crate::source_preview_windows::start(request, generation, region);
    }
    #[cfg(all(target_os = "macos", feature = "capture-macos"))]
    {
        return crate::source_preview_macos::start(request, generation, region);
    }
    #[cfg(all(target_os = "linux", feature = "capture-linux"))]
    {
        return crate::source_preview_linux::start(request, generation, region);
    }
    #[allow(unreachable_code)]
    {
        let _ = (request, generation, region);
        Err(RecordError::new(
            error_codes::UNIMPLEMENTED,
            "native source preview is unavailable in this recorder build",
            "no in-process native preview adapter is compiled for this platform",
        ))
    }
}

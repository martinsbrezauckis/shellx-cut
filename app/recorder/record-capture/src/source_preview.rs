//! Memory-only native source-preview lifecycle.

use record_core::{error_codes, RecordError, Result};
use serde::{Deserialize, Serialize};

pub const MAX_SOURCE_PREVIEW_FRAME_BYTES: usize = 4 * 1024 * 1024;
pub const SOURCE_PREVIEW_MIN_FRAME_INTERVAL_MS: u64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourcePreviewPlatform {
    Windows,
    Macos,
    Linux,
}
/// No title, geometry, ordinal, or primary-display fallback is representable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourcePreviewSource {
    Monitor { monitor_id: String },
    Window { window_id: String },
    Portal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePreviewRequest {
    pub source: SourcePreviewSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera_id: Option<String>,
}

impl SourcePreviewRequest {
    /// Refuse source forms the target platform cannot truthfully fulfill.
    pub fn validate_for(&self, platform: SourcePreviewPlatform) -> Result<()> {
        match (&self.source, platform) {
            (SourcePreviewSource::Monitor { monitor_id }, SourcePreviewPlatform::Windows)
            | (SourcePreviewSource::Monitor { monitor_id }, SourcePreviewPlatform::Macos) => {
                validate_opaque_id("monitor_id", monitor_id)?;
            }
            (SourcePreviewSource::Window { window_id }, SourcePreviewPlatform::Windows)
            | (SourcePreviewSource::Window { window_id }, SourcePreviewPlatform::Macos) => {
                validate_opaque_id("window_id", window_id)?;
            }
            (SourcePreviewSource::Portal, SourcePreviewPlatform::Linux) => {}
            (SourcePreviewSource::Portal, _) => return Err(unavailable("portal source selection")),
            (_, SourcePreviewPlatform::Linux) => {
                return Err(unavailable(
                    "in-app monitor or window selection on the Linux portal backend",
                ));
            }
        }
        if let Some(camera_id) = self.camera_id.as_deref() {
            validate_opaque_id("camera_id", camera_id)?;
        }
        Ok(())
    }
}

/// `Ready` means an actual frame was retained, never merely a successful open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourcePreviewState {
    Idle,
    Starting,
    Ready,
    Paused,
    Hidden,
    PermissionRequired,
    PermissionDenied,
    SourceLost,
    Unavailable,
    Stopped,
}

/// Adapter-observed recursion disclosure, separate from frame readiness.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourcePreviewRecursion {
    #[default]
    None,
    Detected,
    Unavoidable,
}

/// `Stop` precedes a replacement native start; adapters make it idempotent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourcePreviewCommand {
    Start {
        generation: u64,
        request: SourcePreviewRequest,
    },
    Stop,
}

/// A bounded in-memory frame with no serializer or path field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePreviewFrame {
    captured_at_ms: u64,
    encoded: Vec<u8>,
}

impl SourcePreviewFrame {
    pub fn new(captured_at_ms: u64, encoded: Vec<u8>) -> Result<Self> {
        if encoded.is_empty() || encoded.len() > MAX_SOURCE_PREVIEW_FRAME_BYTES {
            return Err(invalid(
                "preview frame must contain 1 byte to 4 MiB of encoded bytes",
            ));
        }
        Ok(Self {
            captured_at_ms,
            encoded,
        })
    }
    pub const fn captured_at_ms(&self) -> u64 {
        self.captured_at_ms
    }
    pub fn encoded(&self) -> &[u8] {
        &self.encoded
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePreviewStatus {
    pub state: SourcePreviewState,
    pub recursion: SourcePreviewRecursion,
    pub request: Option<SourcePreviewRequest>,
    pub generation: Option<u64>,
    pub has_frame: bool,
}

/// Callers execute commands in order and accept frames only from their active stream.
#[derive(Debug)]
pub struct SourcePreviewLifecycle {
    platform: SourcePreviewPlatform,
    state: SourcePreviewState,
    recursion: SourcePreviewRecursion,
    request: Option<SourcePreviewRequest>,
    generation: u64,
    frame: Option<SourcePreviewFrame>,
    last_frame_at_ms: Option<u64>,
}

impl SourcePreviewLifecycle {
    pub const fn new(platform: SourcePreviewPlatform) -> Self {
        Self {
            platform,
            state: SourcePreviewState::Idle,
            recursion: SourcePreviewRecursion::None,
            request: None,
            generation: 0,
            frame: None,
            last_frame_at_ms: None,
        }
    }

    pub const fn platform(&self) -> SourcePreviewPlatform {
        self.platform
    }

    pub fn status(&self) -> SourcePreviewStatus {
        SourcePreviewStatus {
            state: self.state,
            recursion: self.recursion,
            request: self.request.clone(),
            generation: self.request.as_ref().map(|_| self.generation),
            has_frame: self.frame.is_some(),
        }
    }

    pub fn latest_frame(&self) -> Option<&SourcePreviewFrame> {
        self.frame.as_ref()
    }

    /// A replacement releases its live predecessor before the new exact start.
    pub fn start(&mut self, request: SourcePreviewRequest) -> Result<Vec<SourcePreviewCommand>> {
        request.validate_for(self.platform)?;
        let generation = self.next_generation()?;
        let mut commands = Vec::new();
        if self.holds_native_lease() {
            commands.push(SourcePreviewCommand::Stop);
        }
        self.request = Some(request.clone());
        self.generation = generation;
        self.frame = None;
        self.last_frame_at_ms = None;
        self.recursion = SourcePreviewRecursion::None;
        self.state = SourcePreviewState::Starting;
        commands.push(SourcePreviewCommand::Start {
            generation,
            request,
        });
        Ok(commands)
    }

    /// Real frames alone make this ready; `false` is a deliberate rate-cap drop.
    pub fn accept_frame(&mut self, generation: u64, frame: SourcePreviewFrame) -> Result<bool> {
        if !self.holds_native_lease() {
            return Err(invalid(
                "cannot retain a preview frame without an active preview lease",
            ));
        }
        if generation != self.generation {
            return Err(invalid(
                "preview frame belongs to a stale preview generation",
            ));
        }
        if self.last_frame_at_ms.is_some_and(|previous| {
            frame.captured_at_ms.saturating_sub(previous) < SOURCE_PREVIEW_MIN_FRAME_INTERVAL_MS
        }) {
            return Ok(false);
        }
        self.last_frame_at_ms = Some(frame.captured_at_ms);
        self.frame = Some(frame);
        self.state = SourcePreviewState::Ready;
        Ok(true)
    }

    pub fn pause(&mut self) -> Vec<SourcePreviewCommand> {
        if !self.holds_native_lease() {
            return Vec::new();
        }
        self.frame = None;
        self.last_frame_at_ms = None;
        self.state = SourcePreviewState::Paused;
        vec![SourcePreviewCommand::Stop]
    }

    pub fn resume(&mut self) -> Result<Vec<SourcePreviewCommand>> {
        if self.state != SourcePreviewState::Paused {
            return Ok(Vec::new());
        }
        let Some(request) = self.request.clone() else {
            return Ok(Vec::new());
        };
        let generation = self.next_generation()?;
        self.generation = generation;
        self.state = SourcePreviewState::Starting;
        Ok(vec![SourcePreviewCommand::Start {
            generation,
            request,
        }])
    }

    /// Visibility loss releases pixels/ownership and never auto-resumes capture.
    pub fn hide(&mut self) -> Vec<SourcePreviewCommand> {
        let commands = self.release_active_lease();
        self.request = None;
        self.recursion = SourcePreviewRecursion::None;
        self.state = SourcePreviewState::Hidden;
        commands
    }

    pub fn release_for_recording(&mut self) -> Vec<SourcePreviewCommand> {
        self.stop()
    }

    pub fn stop(&mut self) -> Vec<SourcePreviewCommand> {
        let commands = self.release_active_lease();
        self.request = None;
        self.recursion = SourcePreviewRecursion::None;
        self.state = SourcePreviewState::Stopped;
        commands
    }

    pub fn permission_required(&mut self) -> Vec<SourcePreviewCommand> {
        self.terminal(SourcePreviewState::PermissionRequired)
    }

    pub fn permission_denied(&mut self) -> Vec<SourcePreviewCommand> {
        self.terminal(SourcePreviewState::PermissionDenied)
    }

    pub fn source_lost(&mut self) -> Vec<SourcePreviewCommand> {
        self.terminal(SourcePreviewState::SourceLost)
    }

    pub fn unavailable(&mut self) -> Vec<SourcePreviewCommand> {
        self.terminal(SourcePreviewState::Unavailable)
    }

    /// Only adapters may report recursion; it never fabricates frame delivery.
    pub fn set_recursion(&mut self, recursion: SourcePreviewRecursion) {
        self.recursion = recursion;
    }

    fn terminal(&mut self, state: SourcePreviewState) -> Vec<SourcePreviewCommand> {
        let commands = self.release_active_lease();
        self.request = None;
        self.state = state;
        commands
    }

    fn release_active_lease(&mut self) -> Vec<SourcePreviewCommand> {
        self.frame = None;
        self.last_frame_at_ms = None;
        if self.holds_native_lease() {
            vec![SourcePreviewCommand::Stop]
        } else {
            Vec::new()
        }
    }

    fn holds_native_lease(&self) -> bool {
        matches!(
            self.state,
            SourcePreviewState::Starting | SourcePreviewState::Ready
        )
    }

    fn next_generation(&self) -> Result<u64> {
        self.generation
            .checked_add(1)
            .ok_or_else(|| invalid("source preview generation is exhausted"))
    }

    #[cfg(test)]
    pub(crate) fn with_generation_for_test(
        platform: SourcePreviewPlatform,
        generation: u64,
    ) -> Self {
        let mut lifecycle = Self::new(platform);
        lifecycle.generation = generation;
        lifecycle
    }
}

fn validate_opaque_id(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 4_096 {
        return Err(invalid(&format!(
            "preview {label} must be a non-empty opaque identity"
        )));
    }
    Ok(())
}

fn invalid(message: &str) -> RecordError {
    RecordError::new(
        error_codes::INVALID_ARGS,
        message,
        "invalid source preview contract",
    )
}

fn unavailable(feature: &str) -> RecordError {
    RecordError::new(
        error_codes::UNIMPLEMENTED,
        "selected preview source is unavailable on this backend",
        feature,
    )
}

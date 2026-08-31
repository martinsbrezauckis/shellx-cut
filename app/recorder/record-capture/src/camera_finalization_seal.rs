use record_core::{
    error_codes, CameraArtifact, CameraClockRange, CameraMediaFacts, CameraTerminalState,
    RecordError, Result,
};

/// Final media truth. Production code can carry and read this opaque value,
/// but only the parent finalizer module can construct one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CameraMediaSeal {
    artifact_id: String,
    video: String,
    media: CameraMediaFacts,
    bytes: u64,
}

impl CameraMediaSeal {
    /// Platform finalizers with an equivalent no-replace, close, decode, hash,
    /// and durability proof may construct the opaque seal from already-
    /// published immutable-file facts.
    pub(crate) fn verified_publication(
        artifact_id: String,
        video: String,
        media: CameraMediaFacts,
        bytes: u64,
    ) -> Result<Self> {
        Self::validate_finalizer_inputs(&artifact_id, &video, &media, bytes)?;
        Ok(Self::from_finalizer(artifact_id, video, media, bytes))
    }

    /// Validate every fallible seal fact before the irreversible no-replace
    /// link. The private constructor below is consequently infallible and is
    /// called only after anchored publication has succeeded.
    pub(super) fn validate_finalizer_inputs(
        artifact_id: &str,
        video: &str,
        media: &CameraMediaFacts,
        bytes: u64,
    ) -> Result<()> {
        if bytes == 0 {
            return Err(RecordError::new(
                error_codes::CAPTURE,
                "sealed camera evidence must have non-zero bytes",
                "camera media finalization requires a complete non-empty file",
            ));
        }
        CameraArtifact::new(
            "camera_seal",
            artifact_id,
            video,
            CameraClockRange {
                first_frame_offset_ms: 0,
                end_frame_offset_ms: media.duration_ms,
            },
            media.clone(),
            CameraTerminalState::Complete,
        )?;
        Ok(())
    }

    pub(super) fn from_finalizer(
        artifact_id: String,
        video: String,
        media: CameraMediaFacts,
        bytes: u64,
    ) -> Self {
        Self {
            artifact_id,
            video,
            media,
            bytes,
        }
    }

    pub(crate) fn artifact_id(&self) -> &str {
        &self.artifact_id
    }

    pub(crate) fn video(&self) -> &str {
        &self.video
    }

    pub(crate) fn media(&self) -> &CameraMediaFacts {
        &self.media
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    #[cfg(test)]
    pub(crate) fn fixture(
        artifact_id: String,
        video: String,
        media: CameraMediaFacts,
        bytes: u64,
    ) -> Result<Self> {
        Self::validate_finalizer_inputs(&artifact_id, &video, &media, bytes)?;
        Ok(Self::from_finalizer(artifact_id, video, media, bytes))
    }

    #[cfg(test)]
    pub(crate) fn fixture_unchecked(
        artifact_id: String,
        video: String,
        media: CameraMediaFacts,
        bytes: u64,
    ) -> Self {
        Self::from_finalizer(artifact_id, video, media, bytes)
    }
}

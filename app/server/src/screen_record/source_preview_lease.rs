//! Opaque lease identity and public lease-bearing source-preview projections.

use cut_core::{error_codes, CutError};
use record_capture::source_preview::{SourcePreviewFrame, SourcePreviewStatus};

pub(super) const UNRESOLVED_RELEASE_REASON: &str = "A previous native preview could not be released. Restart ShellX Cut (or the Cut server) before previewing or recording.";

pub(crate) struct PreviewSnapshot {
    pub status: SourcePreviewStatus,
    pub frame: Option<SourcePreviewFrame>,
    pub unavailable_reason: Option<&'static str>,
    pub lease_nonce: Option<String>,
}

#[derive(Debug)]
pub(crate) struct PreviewAction {
    pub status: SourcePreviewStatus,
    pub lease_nonce: Option<String>,
}

/// One opaque identity per process-local preview owner. It is paired with the
/// lifecycle generation so a delayed control from a restarted server cannot
/// match a newly issued generation one.
#[derive(Default)]
pub(super) struct PreviewLease {
    nonce: Option<String>,
}

impl PreviewLease {
    pub(super) fn ensure_minted(&mut self) -> Result<(), CutError> {
        if self.nonce.is_some() {
            return Ok(());
        }
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|error| {
            CutError::new(
                error_codes::IO,
                "could not mint the source preview lease nonce",
                error.to_string(),
            )
        })?;
        self.nonce = Some(hex::encode(bytes));
        Ok(())
    }

    pub(super) fn matches(&self, expected_nonce: &str) -> bool {
        self.nonce.as_deref() == Some(expected_nonce)
    }

    pub(super) fn nonce_for(&self, status: &SourcePreviewStatus) -> Option<String> {
        status
            .generation
            .is_some()
            .then(|| self.nonce.clone())
            .flatten()
    }

    #[cfg(test)]
    pub(super) fn with_nonce(nonce: &str) -> Self {
        Self {
            nonce: Some(nonce.into()),
        }
    }
}

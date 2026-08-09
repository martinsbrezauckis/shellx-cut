//! Native-encoder staging and immutable checkpoint publication.

use std::fs;
use std::path::Path;

use crate::contract::{Checkpoint, CheckpointFacts, ManifestError, MediaFacts, OpenCheckpoint};
use crate::journal::Entry;
use crate::manifest::{
    checkpoint_name, checkpoint_path, io, is_local_checkpoint_file, is_plain_dir,
    is_plain_regular_file, staging_name, ManifestOwner,
};
use crate::staging::PrivateStaging;

impl ManifestOwner {
    /// Reserve one private, absent output leaf while retaining the fixed logical
    /// journal staging name. Native encoders may reject a pre-created file.
    pub fn begin_segment(
        &mut self,
        sequence: u64,
        start_ms: u64,
    ) -> Result<std::path::PathBuf, ManifestError> {
        self.ensure_can_begin(sequence)?;
        let staging = PrivateStaging::create(
            &self.root,
            &format!("checkpoint-{sequence:06}"),
            "segment.mp4",
        )
        .map_err(|source| io(&self.root, source))?;
        self.begin_segment_with_staging(sequence, start_ms, staging)
    }

    /// Reserve the compact private stage required by the Windows Graphics
    /// Capture encoder. The logical journal entry intentionally remains the
    /// same platform-neutral checkpoint name.
    pub fn begin_windows_wgc_segment(
        &mut self,
        sequence: u64,
        start_ms: u64,
    ) -> Result<std::path::PathBuf, ManifestError> {
        self.ensure_can_begin(sequence)?;
        let staging = PrivateStaging::create_windows_wgc(&self.root)
            .map_err(|source| io(&self.root, source))?;
        self.begin_segment_with_staging(sequence, start_ms, staging)
    }

    fn begin_segment_with_staging(
        &mut self,
        sequence: u64,
        start_ms: u64,
        staging: PrivateStaging,
    ) -> Result<std::path::PathBuf, ManifestError> {
        let logical_staging = staging_name(sequence);
        if let Err(error) = self.append(&Entry::Open(OpenCheckpoint {
            sequence,
            staging: logical_staging.clone(),
            start_ms,
        })) {
            let _ = staging.cleanup();
            return Err(error);
        }
        self.manifest.openings.push(OpenCheckpoint {
            sequence,
            staging: logical_staging,
            start_ms,
        });
        let path = staging.path().to_path_buf();
        self.staging = Some(staging);
        Ok(path)
    }

    fn ensure_can_begin(&self, sequence: u64) -> Result<(), ManifestError> {
        if self.manifest.receipt.is_some() {
            return Err(ManifestError::Invalid("capture already completed".into()));
        }
        if !self.manifest.openings.is_empty() {
            return Err(ManifestError::Invalid(
                "previous checkpoint is still open or unpublished".into(),
            ));
        }
        if sequence != self.manifest.checkpoints.len() as u64 {
            return Err(ManifestError::Invalid(
                "segment sequence is not contiguous".into(),
            ));
        }
        Ok(())
    }

    /// Validate the native encoder result, then atomically publish the logical
    /// checkpoint name only when no prior checkpoint already owns it.
    pub fn publish(
        &mut self,
        sequence: u64,
        staging: &Path,
        facts: CheckpointFacts,
        media: MediaFacts,
    ) -> Result<Checkpoint, ManifestError> {
        if facts.end_ms <= facts.start_ms
            || sequence != self.manifest.checkpoints.len() as u64
            || !self
                .manifest
                .openings
                .iter()
                .any(|open| open.sequence == sequence)
        {
            return Err(ManifestError::Invalid(
                "checkpoint facts or sequence are invalid".into(),
            ));
        }
        if self.staging.as_ref().map(PrivateStaging::path) != Some(staging)
            || !is_plain_regular_file(staging)?
        {
            return Err(ManifestError::Invalid(
                "staging segment is not the expected local regular file".into(),
            ));
        }
        let meta = fs::symlink_metadata(staging).map_err(|source| io(staging, source))?;
        if meta.len() == 0 || !is_plain_dir(&self.root.join("checkpoints"))? {
            return Err(ManifestError::Invalid(
                "staging segment or checkpoint directory is unsafe".into(),
            ));
        }
        let file = checkpoint_name(sequence);
        let final_path = checkpoint_path(&self.root, sequence);
        crate::atomic::publish_new_synced(staging, &final_path)
            .map_err(|source| io(&final_path, source))?;
        if let Some(staging) = self.staging.take() {
            let _ = staging.cleanup();
        }
        if !is_local_checkpoint_file(&self.root, sequence)? {
            return Err(ManifestError::Invalid(
                "published checkpoint is not a local regular file".into(),
            ));
        }
        let checkpoint = Checkpoint {
            sequence,
            file,
            bytes: meta.len(),
            sha256: crate::manifest::sha256(&final_path)?,
            facts,
            media: Some(media),
        };
        self.append(&Entry::Checkpoint(checkpoint.clone()))?;
        self.manifest.checkpoints.push(checkpoint.clone());
        self.manifest
            .openings
            .retain(|open| open.sequence != sequence);
        Ok(checkpoint)
    }
}

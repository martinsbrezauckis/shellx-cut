//! Finalization barriers kept injectable for the Linux durability fault harness.

use std::fs::File;

use record_core::Result;

use crate::camera_finalization_anchored::AnchoredDirectory;
use crate::camera_finalization_durability::{sync_publication_directory, sync_published_file};

pub(super) struct FinalizationHooks<
    AfterPathsAnchored,
    AfterStagedHash,
    AfterLink,
    AfterPublicationVerify,
    SyncFile,
    AfterFileSync,
    SyncDirectory,
    AfterDirectorySync,
> {
    pub(super) after_paths_anchored: AfterPathsAnchored,
    pub(super) after_staged_hash: AfterStagedHash,
    pub(super) after_link: AfterLink,
    pub(super) after_publication_verify: AfterPublicationVerify,
    pub(super) sync_file: SyncFile,
    pub(super) after_file_sync: AfterFileSync,
    pub(super) sync_directory: SyncDirectory,
    pub(super) after_directory_sync: AfterDirectorySync,
}

type Noop = fn() -> Result<()>;
type FileSync = fn(&File) -> Result<()>;
type DirectorySync = fn(&AnchoredDirectory) -> Result<()>;

pub(super) type ProductionFinalizationHooks =
    FinalizationHooks<Noop, Noop, Noop, Noop, FileSync, Noop, DirectorySync, Noop>;

pub(super) fn production_hooks() -> ProductionFinalizationHooks {
    FinalizationHooks {
        after_paths_anchored: noop,
        after_staged_hash: noop,
        after_link: noop,
        after_publication_verify: noop,
        sync_file: sync_published_file,
        after_file_sync: noop,
        sync_directory: sync_publication_directory,
        after_directory_sync: noop,
    }
}

fn noop() -> Result<()> {
    Ok(())
}

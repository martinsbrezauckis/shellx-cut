//! Engine-owned fallback project creation and pinned-handle rollback for `screen_record.start`.
//!
//! When a Start begins without an open project and without `expected_project_identity`,
//! the engine creates the temporary project internally and retains a single-use, non-cloneable
//! [`FallbackProjectTransaction`]. If Start is refused, rollback inspects and removes only what
//! this Start created through pinned directory handles with no-follow on every component.

#[cfg(unix)]
#[path = "created_project_rollback/pinned_unix.rs"]
mod pinned;
#[cfg(windows)]
#[path = "created_project_rollback/pinned_windows.rs"]
mod pinned;
mod remove;
mod reply;
mod verify;
pub(crate) use reply::{refused_fallback_reply, running_take_failure_reply, CreateFailure};

use crate::screen_record::refuse_project_transition_if_active;
use crate::state::AppState;
use cut_core::{error_codes, Actor, CutError, ProjectStore};
use remove::{execute_pinned_rollback, PhaseFailure};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use verify::{
    capture_created_baseline, pick_unique_project_name, verify_ops_file, verify_project_json_file,
    verify_root_to_project_link,
};

pub(super) const STANDARD_SUBDIRS: [&str; 3] = ["receipts", "proxies", "filmstrip"];
pub(super) const START_CAPTURE_FILES: [&str; 5] = [
    ".capture.json",
    "capture.manifest.jsonl",
    "record.log",
    "recording-scene.receipt.json",
    "studio.events.jsonl",
];
pub(super) const MAX_METADATA_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RollbackOutcome {
    NotCreated,
    Discarded,
    Kept,
    Incomplete,
}

impl RollbackOutcome {
    pub(crate) fn as_reply_payload(self) -> Value {
        json!({
            "created_project_outcome": match self {
                Self::NotCreated => "not_created",
                Self::Discarded => "discarded",
                Self::Kept => "kept",
                Self::Incomplete => "incomplete",
            },
            "created_project_discarded": matches!(self, Self::Discarded),
            "created_project_kept": matches!(self, Self::Kept),
        })
    }
}

pub(crate) struct CommittedFallbackProject {
    pub(crate) name: String,
    pub(crate) identity: Value,
}

/// One-use, Start-owned transaction holding pinned handles and baseline identities
/// for a fallback recording project. Never `Clone`, `Copy`, or caller-addressed.
pub(crate) struct FallbackProjectTransaction {
    root_dir: pinned::PinnedDir,
    project_dir_name: String,
    project_dir: pinned::PinnedDir,
    creation_op: String,
    ops_identity: pinned::FileIdentity,
    project_json_identity: pinned::FileIdentity,
    sub_dir_identities: [(&'static str, pinned::DirIdentity); 3],
    capture_id: Option<String>,
    store: ProjectStore,
}

impl FallbackProjectTransaction {
    pub(crate) fn create(actor: Actor) -> Result<Self, CreateFailure> {
        let root = crate::dispatch::default_projects_dir().map_err(CreateFailure::NoFolder)?;
        Self::create_at_root(&root, actor)
    }

    pub(crate) fn create_at_root(root_path: &Path, actor: Actor) -> Result<Self, CreateFailure> {
        std::fs::create_dir_all(root_path).map_err(|error| {
            CreateFailure::NoFolder(CutError::new(
                error_codes::IO,
                "cannot create projects directory",
                error.to_string(),
            ))
        })?;
        let root_dir = pinned::open_root_dir_nofollow(root_path).map_err(|error| {
            CreateFailure::NoFolder(CutError::new(
                error_codes::IO,
                "projects root is unsafe or unavailable",
                error.to_string(),
            ))
        })?;
        let name = pick_unique_project_name(&root_dir).map_err(CreateFailure::NoFolder)?;
        let store = ProjectStore::create_with_actor(root_path, &name, None, actor)
            .map_err(CreateFailure::NoFolder)?;
        let project_dir_name = format!("{name}.cutproj");
        // From here the folder exists: any failure leaves it, unlisted.
        let (project_dir, creation_op, ops_identity, project_json_identity, sub_dir_identities) =
            capture_created_baseline(&root_dir, &project_dir_name, &store)
                .map_err(CreateFailure::FolderLeft)?;
        Ok(Self {
            root_dir,
            project_dir_name,
            project_dir,
            creation_op,
            ops_identity,
            project_json_identity,
            sub_dir_identities,
            capture_id: None,
            store,
        })
    }

    pub(crate) fn project_dir_path(&self) -> &Path {
        &self.store.dir
    }

    pub(crate) fn note_capture_dir_created(&mut self, capture_id: String) {
        self.capture_id = Some(capture_id);
    }

    pub(crate) async fn commit_after_start_success(
        self,
        state: &AppState,
    ) -> Result<CommittedFallbackProject, CutError> {
        verify_root_to_project_link(&self.root_dir, &self.project_dir_name, &self.project_dir)
            .map_err(|_| {
                CutError::new(
                    error_codes::CONFLICT,
                    "fallback recording project changed before activation",
                    "the newly created recording project directory was altered or redirected",
                )
            })?;
        let store = self.store;
        let project_name = store.project.name.clone();
        let identity = crate::dispatch::path_free_project_identity(&store)?;
        let dir_text = store.dir.to_string_lossy().into_owned();
        drop(self.project_dir);
        drop(self.root_dir);
        crate::projects_index::note_created(&project_name, &dir_text)
            .map_err(crate::projects_index::refusal)?;
        crate::dispatch::activate_project_locked(state, store).await?;
        Ok(CommittedFallbackProject {
            name: project_name,
            identity,
        })
    }

    pub(crate) async fn rollback(self, state: &AppState) -> RollbackOutcome {
        self.rollback_with_hooks(state, || {}, || {}).await
    }

    async fn rollback_with_hooks(
        self,
        state: &AppState,
        after_admit: impl FnOnce(),
        after_ops_unlink: impl FnOnce(),
    ) -> RollbackOutcome {
        let dir_path = self.store.dir.clone();
        if refuse_project_transition_if_active(state, Some(&dir_path))
            .await
            .is_err()
        {
            return self.keep_if_genuine_baseline(state).await;
        }
        let Self {
            root_dir,
            project_dir_name,
            project_dir,
            creation_op,
            ops_identity,
            project_json_identity,
            sub_dir_identities,
            capture_id,
            store,
        } = self;

        let kept_identity = project_dir.identity;
        match execute_pinned_rollback(
            &root_dir,
            &project_dir_name,
            project_dir,
            &creation_op,
            ops_identity,
            project_json_identity,
            &sub_dir_identities,
            capture_id.as_deref(),
            after_admit,
            after_ops_unlink,
        ) {
            Ok(()) => RollbackOutcome::Discarded,
            Err(PhaseFailure::BeforeBaselineUnlink) => {
                keep_intact_fallback_if_genuine(
                    state,
                    &root_dir,
                    &project_dir_name,
                    kept_identity,
                    ops_identity,
                    project_json_identity,
                    &creation_op,
                    store,
                )
                .await
            }
            Err(PhaseFailure::MidBaselineRemoval) => RollbackOutcome::Incomplete,
        }
    }

    async fn keep_if_genuine_baseline(self, state: &AppState) -> RollbackOutcome {
        let kept_identity = self.project_dir.identity;
        drop(self.project_dir);
        keep_intact_fallback_if_genuine(
            state,
            &self.root_dir,
            &self.project_dir_name,
            kept_identity,
            self.ops_identity,
            self.project_json_identity,
            &self.creation_op,
            self.store,
        )
        .await
    }
}

#[allow(clippy::too_many_arguments)]
async fn keep_intact_fallback_if_genuine(
    state: &AppState,
    root_dir: &pinned::PinnedDir,
    project_dir_name: &str,
    expected_project_dir_id: pinned::DirIdentity,
    ops_identity: pinned::FileIdentity,
    project_json_identity: pinned::FileIdentity,
    creation_op: &str,
    store: ProjectStore,
) -> RollbackOutcome {
    let genuine = (|| -> Result<(), ()> {
        pinned::verify_dir_handle(root_dir).map_err(|_| ())?;
        let proj = pinned::open_verified_child_dir_nofollow(
            root_dir,
            project_dir_name,
            expected_project_dir_id,
        )
        .map_err(|_| ())?;
        let mut ops = pinned::open_verified_child_file_nofollow(&proj, "ops.jsonl", ops_identity)
            .map_err(|_| ())?;
        verify_ops_file(&mut ops, creation_op).map_err(|_| ())?;
        let mut pj =
            pinned::open_verified_child_file_nofollow(&proj, "project.json", project_json_identity)
                .map_err(|_| ())?;
        verify_project_json_file(&mut pj).map_err(|_| ())?;
        Ok(())
    })()
    .is_ok();
    if !genuine {
        // The retained folder cannot be confirmed: do not promise it is there.
        return RollbackOutcome::Incomplete;
    }
    let name = store.project.name.clone();
    let dir = PathBuf::from(&store.dir).to_string_lossy().into_owned();
    if crate::projects_index::note_created(&name, &dir).is_err() {
        return RollbackOutcome::Incomplete;
    }
    if crate::dispatch::activate_project_locked(state, store)
        .await
        .is_err()
    {
        return RollbackOutcome::Incomplete;
    }
    RollbackOutcome::Kept
}

#[cfg(test)]
#[path = "created_project_rollback/tests.rs"]
mod tests;

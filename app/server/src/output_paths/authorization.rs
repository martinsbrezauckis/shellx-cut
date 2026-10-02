//! Immutable request-time write authority for delayed Render Queue children.
//! This is internal state, never an argument accepted from a REST caller.

use cut_core::{error_codes, CutError};
use cut_media::PathFence;
use std::{
    future::Future,
    path::{Path, PathBuf},
};

tokio::task_local! {
    static QUEUE_OUTPUT_AUTHORIZATION: OutputAuthorization;
}

#[derive(Clone)]
pub(crate) struct OutputAuthorization {
    fence: PathFence,
    default_dir: Option<PathBuf>,
}

impl OutputAuthorization {
    pub(crate) fn capture(project_dir: &Path) -> Result<Self, CutError> {
        let default_dir = super::unscoped_session_output_dir()
            .map(|dir| dir.canonicalize())
            .transpose()?;
        let fence = super::make_fence_with_default(project_dir, default_dir.as_deref())?;
        Ok(Self { fence, default_dir })
    }

    pub(crate) async fn scope<T>(self, action: impl Future<Output = T>) -> T {
        QUEUE_OUTPUT_AUTHORIZATION.scope(self, action).await
    }

    fn fence_for(&self, project_dir: &Path) -> Result<PathFence, CutError> {
        if project_dir.canonicalize()? != self.fence.project_dir() {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "queued render output authorization belongs to a different project",
                "the project changed after render.queue admission",
            ));
        }
        Ok(self.fence.clone())
    }
}

pub(super) fn scoped_fence(project_dir: &Path) -> Option<Result<PathFence, CutError>> {
    QUEUE_OUTPUT_AUTHORIZATION
        .try_with(|scope| scope.fence_for(project_dir))
        .ok()
}

pub(super) fn scoped_default_dir() -> Option<Option<PathBuf>> {
    QUEUE_OUTPUT_AUTHORIZATION
        .try_with(|scope| scope.default_dir.clone())
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output_paths::{
        fence_output_path, make_fence, set_session_output_dir, OutputPathPolicy,
    };

    #[test]
    fn snapshot_rejects_foreign_roots_projects_and_does_not_leak_to_other_tasks() {
        let _lock = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let project = tempfile::tempdir().unwrap();
        let project_root = project.path().canonicalize().unwrap();
        let chosen = tempfile::tempdir().unwrap();
        let chosen_path = chosen.path().canonicalize().unwrap();
        let later = tempfile::tempdir().unwrap();
        let foreign_project = tempfile::tempdir().unwrap();
        set_session_output_dir(Some(chosen_path.clone()));
        let authority = OutputAuthorization::capture(project_root.as_path()).unwrap();
        set_session_output_dir(Some(later.path().to_path_buf()));
        let selected = chosen_path.join("selected.mp4");
        runtime.block_on(authority.scope(async {
            assert!(fence_output_path(
                project_root.as_path(),
                selected.to_str(),
                "exports/out.mp4",
                OutputPathPolicy::MP4
            )
            .is_ok());
            assert!(fence_output_path(
                project_root.as_path(),
                later.path().join("foreign.mp4").to_str(),
                "exports/out.mp4",
                OutputPathPolicy::MP4
            )
            .is_err());
            assert!(fence_output_path(
                project_root.as_path(),
                Some("../escape.mp4"),
                "exports/out.mp4",
                OutputPathPolicy::MP4
            )
            .is_err());
            assert!(make_fence(foreign_project.path()).is_err());
            let resolved = fence_output_path(
                project_root.as_path(),
                None,
                "exports/default.mp4",
                OutputPathPolicy::MP4,
            )
            .unwrap();
            assert_eq!(resolved.parent(), Some(chosen_path.as_path()));
            let dir = project_root.as_path().to_path_buf();
            let path = selected.clone();
            assert!(tokio::spawn(async move {
                make_fence(&dir).unwrap().fence_output_path(&path).is_err()
            })
            .await
            .unwrap());
        }));
        assert_eq!(
            super::super::unscoped_session_output_dir().as_deref(),
            Some(later.path())
        );
        assert!(make_fence(project_root.as_path())
            .unwrap()
            .fence_output_path(&selected)
            .is_err());
        set_session_output_dir(None);
        let project_default = OutputAuthorization::capture(project_root.as_path()).unwrap();
        set_session_output_dir(Some(later.path().to_path_buf()));
        runtime.block_on(project_default.scope(async {
            let resolved = fence_output_path(
                project_root.as_path(),
                None,
                "exports/project-default.mp4",
                OutputPathPolicy::MP4,
            )
            .unwrap();
            assert_eq!(
                resolved.parent(),
                Some(project_root.as_path().join("exports").as_path()),
                "captured absence must not inherit a later folder"
            );
        }));
        assert_eq!(
            super::super::unscoped_session_output_dir().as_deref(),
            Some(later.path())
        );
        set_session_output_dir(None);
    }
}

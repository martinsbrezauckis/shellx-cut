//! Project identity pin for a request that performs delayed materialization.

use crate::dispatch::{no_project, path_free_project_identity};
use crate::state::AppState;
use cut_core::{error_codes, CutError};
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::sync::MutexGuard;

/// Non-cloneable evidence that a verified project transition remains held.
pub(crate) struct ProjectMaterialization<'a> {
    _transition: MutexGuard<'a, ()>,
}

#[derive(Clone)]
pub(crate) struct ProjectMaterializationPin {
    operation: &'static str,
    project_dir: PathBuf,
    project_identity: Value,
    project_revision: Option<String>,
}

impl ProjectMaterializationPin {
    /// Safe origin evidence for a response that must not direct recovery at a
    /// replacement project. The path stays server-local; callers match the
    /// same path-free identity exposed by project.state after reopening.
    pub(crate) fn origin_binding(&self) -> Value {
        json!({
            "project_identity": self.project_identity.clone(),
            "project_revision": self.project_revision.clone(),
        })
    }

    /// Snapshot the open project before a connector or adapter can yield.
    pub(crate) async fn capture(
        state: &AppState,
        operation: &'static str,
    ) -> Result<Self, CutError> {
        let project = state.project.read().await;
        let store = project.as_ref().ok_or_else(no_project)?;
        Ok(Self {
            operation,
            project_dir: store.dir.clone(),
            project_identity: path_free_project_identity(store)?,
            project_revision: store.log.current_revision()?,
        })
    }

    /// Verify the original project and hold replacement out through materialization.
    pub(crate) async fn lock_verified<'a>(
        &self,
        state: &'a AppState,
    ) -> Result<ProjectMaterialization<'a>, CutError> {
        let transition = state.project_transition.lock().await;
        let project = state.project.read().await;
        let Some(store) = project.as_ref() else {
            return Err(self.project_changed("the project closed while its connector was running"));
        };
        if store.dir != self.project_dir
            || path_free_project_identity(store)? != self.project_identity
        {
            return Err(self
                .project_changed("the open project is not the project that started the request"));
        }
        if store.log.current_revision()? != self.project_revision {
            return Err(self.project_changed(
                "the target project's revision changed while the connector was running",
            ));
        }
        drop(project);
        Ok(ProjectMaterialization {
            _transition: transition,
        })
    }

    fn project_changed(&self, cause: &str) -> CutError {
        CutError::new(
            error_codes::CONFLICT,
            format!(
                "{} target project changed before materialization",
                self.operation
            ),
            cause,
        )
        .with_suggested_action("refresh project.state and run the request again")
    }
}

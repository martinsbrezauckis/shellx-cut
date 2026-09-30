//! Admit embedded renderer media against the initiating project snapshot.
use cut_core::{error_codes, CutError, Project};
use std::path::{Path, PathBuf};

pub(crate) fn load(
    dir: &Path,
    project: &Project,
    path: &Path,
) -> Result<record_core::EditPlan, CutError> {
    let mut plan = super::polish::load_plan(path)?;
    admit(dir, project, &mut plan)?;
    Ok(plan)
}

pub(crate) fn admit(
    dir: &Path,
    project: &Project,
    plan: &mut record_core::EditPlan,
) -> Result<(), CutError> {
    let Some(camera) = plan.webcam.as_mut() else {
        return Ok(());
    };
    let path = Path::new(&camera.source);
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        dir.join(path)
    };
    if !record_recovery::is_plain_regular_file(&path).map_err(denied)? {
        return Err(denied("webcam source is not a plain local file"));
    }
    let canonical = path.canonicalize().map_err(denied)?;
    let root = dir.canonicalize().map_err(denied)?;
    let registered = project.assets.values().any(|asset| {
        let asset = Path::new(&asset.path);
        let asset = if asset.is_absolute() {
            asset.to_owned()
        } else {
            dir.join(asset)
        };
        asset.canonicalize().is_ok_and(|asset| asset == canonical)
    });
    if !canonical.starts_with(&root) && !registered {
        return Err(denied(
            "webcam source must belong to this project or an explicitly registered asset",
        ));
    }
    camera.source = canonical.to_string_lossy().into_owned();
    Ok(())
}

pub(super) fn webcam_path(plan: &record_core::EditPlan) -> Option<PathBuf> {
    plan.webcam
        .as_ref()
        .map(|camera| PathBuf::from(&camera.source))
}

fn denied(cause: impl std::fmt::Display) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "unsafe EditPlan webcam source",
        cause.to_string(),
    )
}
#[cfg(test)]
#[path = "plan_inputs_tests.rs"]
pub(super) mod tests;

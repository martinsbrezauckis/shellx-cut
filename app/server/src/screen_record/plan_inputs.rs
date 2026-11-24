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

fn canonical_plain_file(dir: &Path, raw: &str, label: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        dir.join(path)
    };
    match record_recovery::is_plain_regular_file(&path) {
        Ok(true) => path.canonicalize().map_err(|e| e.to_string()),
        Ok(false) => Err(format!("{label} is not a plain local file")),
        Err(e) => Err(e.to_string()),
    }
}

pub(crate) fn admit(
    dir: &Path,
    project: &Project,
    plan: &mut record_core::EditPlan,
) -> Result<(), CutError> {
    let root = dir.canonicalize().map_err(denied_webcam)?;
    if let Some(camera) = plan.webcam.as_mut() {
        let canonical =
            canonical_plain_file(dir, &camera.source, "webcam source").map_err(denied_webcam)?;
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
            return Err(denied_webcam(
                "webcam source must belong to this project or an explicitly registered asset",
            ));
        }
        camera.source = canonical.to_string_lossy().into_owned();
    }
    if let Some(captions) = plan.captions.as_mut() {
        let canonical = canonical_plain_file(dir, &captions.words_path, "caption transcript")
            .map_err(denied_caption)?;
        if !canonical.starts_with(&root) {
            return Err(denied_caption(
                "caption transcript must stay inside this project",
            ));
        }
        captions.words_path = canonical.to_string_lossy().into_owned();
    }
    Ok(())
}

pub(super) fn webcam_path(plan: &record_core::EditPlan) -> Option<PathBuf> {
    plan.webcam
        .as_ref()
        .map(|camera| PathBuf::from(&camera.source))
}

fn denied_webcam(cause: impl std::fmt::Display) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "unsafe EditPlan webcam source",
        cause.to_string(),
    )
    .with_suggested_action("Keep the EditPlan webcam source inside this project.")
}

fn denied_caption(cause: impl std::fmt::Display) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "unsafe EditPlan caption transcript",
        cause.to_string(),
    )
    .with_suggested_action("Keep the EditPlan caption transcript inside this project.")
}
#[cfg(test)]
#[path = "plan_inputs_tests.rs"]
pub(super) mod tests;

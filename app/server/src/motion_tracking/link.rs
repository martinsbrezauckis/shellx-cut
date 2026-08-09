use crate::dispatch::no_project;
use crate::motion_bridge::{
    current_motion_link_from_ops, ensure_motion_link_unchanged, motion_package_identity,
    motion_package_revision, safe_fragment,
};
use crate::state::AppState;
use cut_core::{error_codes, Actor, CutError};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub(super) struct LinkedSource {
    pub(super) project_dir: PathBuf,
    pub(super) package: PathBuf,
    pub(super) link: Value,
    generation: String,
    source_revision: String,
}

pub(super) async fn linked_source(state: &AppState, clip: &str) -> Result<LinkedSource, CutError> {
    let (link, generation, project_dir) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let (link, generation) =
            current_motion_link_from_ops(store.log.read_all()?.as_slice(), &store.project, clip)?;
        (link, generation, store.dir.clone())
    };
    let raw = link
        .get("sourcePath")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                "linked Motion source is unavailable",
                "relink the original package before using tracking",
            )
        })?;
    let metadata = std::fs::symlink_metadata(raw).map_err(|error| {
        CutError::new(
            error_codes::NOT_FOUND,
            "linked Motion source is missing",
            error.to_string(),
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CutError::new(
            error_codes::GUARDRAIL,
            "linked Motion source must be a real directory",
            "symlink package roots are refused",
        ));
    }
    let package = PathBuf::from(raw).canonicalize()?;
    let identity = motion_package_identity(&package)?;
    let source_revision = motion_package_revision(&package)?;
    if link.get("packageId").and_then(Value::as_str) != Some(identity.0.as_str())
        || link.get("motionId").and_then(Value::as_str) != Some(identity.1.as_str())
    {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "linked Motion source identity changed",
            "relink to the original package identity before tracking",
        ));
    }
    Ok(LinkedSource {
        project_dir,
        package,
        link,
        generation,
        source_revision,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn attach_candidate(
    state: &AppState,
    mut source: LinkedSource,
    output: PathBuf,
    receipt_id: &str,
    verb: &str,
    state_name: &str,
    tracking: Value,
    actor: Actor,
    rationale: Option<String>,
) -> Result<String, CutError> {
    ensure_source_package_unchanged(&source.package, &source.source_revision)?;
    let source_revision = motion_package_revision(&output)?;
    let link = source
        .link
        .as_object_mut()
        .expect("validated Motion link is an object");
    link.insert("sourcePath".into(), json!(output));
    link.insert("sourceRevision".into(), json!(source_revision));
    link.insert("sourceRevisionKind".into(), json!("motion-package"));
    link.insert("state".into(), json!(state_name));
    link.insert("lastReceiptId".into(), json!(receipt_id));
    link.insert("tracking".into(), tracking);
    let clip_id = source
        .link
        .get("clipId")
        .and_then(Value::as_str)
        .expect("validated Motion link clip id")
        .to_string();
    let op = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().ok_or_else(no_project)?;
        if store.dir != source.project_dir {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "project changed while Motion tracking was running",
                "the candidate package was not attached",
            ));
        }
        ensure_motion_link_unchanged(
            store.log.read_all()?.as_slice(),
            &clip_id,
            &source.generation,
        )?;
        store.record_motion_link_source_update(verb, &clip_id, source.link, actor, rationale)?
    };
    state
        .events
        .publish(crate::events::Event::OpApplied { op: op.clone() });
    Ok(op.op_id)
}

fn ensure_source_package_unchanged(package: &Path, expected: &str) -> Result<(), CutError> {
    if motion_package_revision(package)? != expected {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "linked Motion source changed while tracking was running",
            "the completed candidate package was not attached",
        ));
    }
    Ok(())
}

pub(super) fn output_package(
    project_dir: &Path,
    clip: &str,
    operation: &str,
) -> Result<PathBuf, CutError> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| CutError::new(error_codes::IO, "read system clock", error.to_string()))?
        .as_nanos();
    let relative = format!(
        "motion-sources/{}/{}-{}",
        safe_fragment(clip),
        stamp,
        safe_fragment(operation)
    );
    crate::output_paths::fence_project_directory_path(project_dir, &relative)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_revision_race_is_rejected() {
        let root = tempfile::tempdir().expect("temp package");
        std::fs::write(
            root.path().join("manifest.json"),
            r#"{"id":"pkg","motion":"motion.json"}"#,
        )
        .expect("manifest");
        std::fs::write(root.path().join("motion.json"), r#"{"id":"motion"}"#).expect("motion");
        let revision = motion_package_revision(root.path()).expect("revision");
        std::fs::write(
            root.path().join("motion.json"),
            r#"{"id":"motion","layers":[]}"#,
        )
        .expect("mutated motion");
        assert!(ensure_source_package_unchanged(root.path(), &revision).is_err());
    }
}

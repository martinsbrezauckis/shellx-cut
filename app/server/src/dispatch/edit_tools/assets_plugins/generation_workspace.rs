//! Admit scratch ancestors before any reference, prompt, or provider writes.

use cut_core::CutError;
use std::path::{Path, PathBuf};

pub(super) fn create(project: &Path, run_name: &str) -> Result<PathBuf, CutError> {
    let runs = crate::output_paths::ensure_plain_project_relative_dir(
        project,
        Path::new("cache/gen/runs"),
    )?;
    // The run folder is new for every run (item 140 sweep): its files are then
    // written in place, so an existing folder of that name is never reused.
    let run = runs.join(run_name);
    std::fs::create_dir(&run).map_err(|e| {
        CutError::new(
            cut_core::error_codes::IO,
            format!(
                "generation run folder {} could not be created",
                run.display()
            ),
            e.to_string(),
        )
        .with_suggested_action("retry the generation; a new run folder name is chosen each time")
    })?;
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn generation_workspace_refuses_imported_link_at_each_scratch_ancestor() {
        for linked in ["cache", "cache/gen", "cache/gen/runs"] {
            let project = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let link = project.path().join(linked);
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(outside.path(), &link).unwrap();
            let error = create(project.path(), "owned-run").unwrap_err();
            assert_eq!(error.code, cut_core::error_codes::INVALID_ARGS);
            assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
        }
    }

    /// Item 140 sweep: a run folder that already exists (a received project
    /// can carry one holding second names of outside files) is never reused.
    #[test]
    fn generation_workspace_never_reuses_an_existing_run_folder() {
        let project = tempfile::tempdir().unwrap();
        let outside = project.path().join("outside.txt");
        std::fs::write(&outside, "KEEP").unwrap();
        let planted = project.path().join("cache/gen/runs/owned-run");
        std::fs::create_dir_all(&planted).unwrap();
        std::fs::hard_link(&outside, planted.join("prompt.txt")).unwrap();
        let error = create(project.path(), "owned-run").unwrap_err();
        assert_eq!(error.code, cut_core::error_codes::IO);
        assert!(error.suggested_action.is_some());
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "KEEP");
    }

    #[test]
    fn generation_workspace_preserves_plain_chain_and_registered_reference_copy() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.png");
        std::fs::write(&source, b"synthetic registered image").unwrap();
        let hash = cut_core::hash_file(&source).unwrap();
        let mut project = cut_core::Project::new("test", Default::default());
        let asset: cut_core::Asset = serde_json::from_value(serde_json::json!({
            "path": source,
            "hash": hash,
            "probe": {"kind": "image"}
        }))
        .unwrap();
        project.assets.insert("ref".into(), asset);
        let refs = crate::dispatch::generated_assets::resolve_generation_references(
            &project,
            &["ref".into()],
        )
        .unwrap();
        let workspace = create(root.path(), "owned-run").unwrap();
        assert_eq!(workspace, root.path().join("cache/gen/runs/owned-run"));
        let copied = crate::dispatch::generated_assets::copy_generation_references(
            &project,
            &refs,
            root.path(),
            &workspace,
        )
        .unwrap();
        assert_eq!(
            copied,
            vec![workspace.join("reference-1.png").display().to_string()]
        );
        assert_eq!(cut_core::hash_file(Path::new(&copied[0])).unwrap(), hash);
        assert_eq!(
            create(root.path(), "next-run").unwrap(),
            root.path().join("cache/gen/runs/next-run")
        );
    }
}

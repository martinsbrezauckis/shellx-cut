//! Bottom-up pinned-handle rollback execution with pre-unlink re-verification.

use super::pinned;
use super::verify::{
    remove_start_capture_subtree, verify_admitted_tree, verify_ops_file, verify_project_json_file,
    verify_root_to_project_link,
};

pub(super) enum PhaseFailure {
    BeforeBaselineUnlink,
    MidBaselineRemoval,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_pinned_rollback(
    root_dir: &pinned::PinnedDir,
    project_dir_name: &str,
    project_dir: pinned::PinnedDir,
    creation_op: &str,
    ops_identity: pinned::FileIdentity,
    project_json_identity: pinned::FileIdentity,
    sub_dir_identities: &[(&'static str, pinned::DirIdentity); 3],
    capture_id: Option<&str>,
    after_admit: impl FnOnce(),
    after_ops_unlink: impl FnOnce(),
) -> Result<(), PhaseFailure> {
    verify_admitted_tree(
        root_dir,
        project_dir_name,
        &project_dir,
        creation_op,
        ops_identity,
        project_json_identity,
        sub_dir_identities,
        capture_id,
    )
    .map_err(|_| PhaseFailure::BeforeBaselineUnlink)?;

    after_admit();

    if let Some(cid) = capture_id {
        remove_start_capture_subtree(root_dir, project_dir_name, &project_dir, cid)
            .map_err(|_| PhaseFailure::BeforeBaselineUnlink)?;
    }

    for &(sub, sub_id) in sub_dir_identities {
        verify_root_to_project_link(root_dir, project_dir_name, &project_dir)
            .map_err(|_| PhaseFailure::BeforeBaselineUnlink)?;
        let dir = pinned::open_verified_child_dir_nofollow(&project_dir, sub, sub_id)
            .map_err(|_| PhaseFailure::BeforeBaselineUnlink)?;
        if !pinned::list_child_names(&dir)
            .map_err(|_| PhaseFailure::BeforeBaselineUnlink)?
            .is_empty()
        {
            return Err(PhaseFailure::BeforeBaselineUnlink);
        }
        drop(dir);
        pinned::unlink_verified_child_dir_at(&project_dir, sub, Some(sub_id))
            .map_err(|_| PhaseFailure::BeforeBaselineUnlink)?;
    }

    verify_root_to_project_link(root_dir, project_dir_name, &project_dir)
        .map_err(|_| PhaseFailure::BeforeBaselineUnlink)?;
    let mut ops_file =
        pinned::open_verified_child_file_nofollow(&project_dir, "ops.jsonl", ops_identity)
            .map_err(|_| PhaseFailure::BeforeBaselineUnlink)?;
    verify_ops_file(&mut ops_file, creation_op).map_err(|_| PhaseFailure::BeforeBaselineUnlink)?;
    drop(ops_file);
    let mut pj_file = pinned::open_verified_child_file_nofollow(
        &project_dir,
        "project.json",
        project_json_identity,
    )
    .map_err(|_| PhaseFailure::BeforeBaselineUnlink)?;
    verify_project_json_file(&mut pj_file).map_err(|_| PhaseFailure::BeforeBaselineUnlink)?;
    drop(pj_file);
    pinned::unlink_verified_child_file_at(&project_dir, "ops.jsonl", Some(ops_identity))
        .map_err(|_| PhaseFailure::BeforeBaselineUnlink)?;

    after_ops_unlink();

    verify_root_to_project_link(root_dir, project_dir_name, &project_dir)
        .map_err(|_| PhaseFailure::MidBaselineRemoval)?;
    pinned::unlink_verified_child_file_at(
        &project_dir,
        "project.json",
        Some(project_json_identity),
    )
    .map_err(|_| PhaseFailure::MidBaselineRemoval)?;

    verify_root_to_project_link(root_dir, project_dir_name, &project_dir)
        .map_err(|_| PhaseFailure::MidBaselineRemoval)?;
    if !pinned::list_child_names(&project_dir)
        .map_err(|_| PhaseFailure::MidBaselineRemoval)?
        .is_empty()
    {
        return Err(PhaseFailure::MidBaselineRemoval);
    }
    pinned::unlink_root_project_dir(root_dir, project_dir, project_dir_name)
        .map_err(|_| PhaseFailure::MidBaselineRemoval)?;

    Ok(())
}

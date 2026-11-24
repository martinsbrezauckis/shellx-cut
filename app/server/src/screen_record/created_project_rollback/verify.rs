//! Baseline inspection and pinned-tree verification for fallback Start projects.

use super::{pinned, MAX_METADATA_BYTES, STANDARD_SUBDIRS, START_CAPTURE_FILES};
use cut_core::{error_codes, CutError, OpRecord, Project, ProjectStore};
use std::collections::BTreeSet;

pub(super) fn capture_created_baseline(
    root_dir: &pinned::PinnedDir,
    project_dir_name: &str,
    store: &ProjectStore,
) -> Result<
    (
        pinned::PinnedDir,
        String,
        pinned::FileIdentity,
        pinned::FileIdentity,
        [(&'static str, pinned::DirIdentity); 3],
    ),
    CutError,
> {
    let io_err = |msg: &str, e: std::io::Error| CutError::new(error_codes::IO, msg, e.to_string());
    let proj_dir = pinned::open_child_dir_nofollow(root_dir, project_dir_name)
        .map_err(|e| io_err("cannot pin newly created recording project", e))?;
    let creation_op = store
        .log
        .current_revision()?
        .ok_or_else(|| CutError::new(error_codes::IO, "created project missing baseline op", ""))?;
    let mut ops_file = pinned::open_child_file_nofollow(&proj_dir, "ops.jsonl")
        .map_err(|e| io_err("cannot pin created project journal", e))?;
    verify_ops_file(&mut ops_file, &creation_op)?;
    let mut pj_file = pinned::open_child_file_nofollow(&proj_dir, "project.json")
        .map_err(|e| io_err("cannot pin created project state", e))?;
    verify_project_json_file(&mut pj_file)?;
    let mut subs = Vec::with_capacity(STANDARD_SUBDIRS.len());
    for sub in STANDARD_SUBDIRS {
        let dir = pinned::open_child_dir_nofollow(&proj_dir, sub)
            .map_err(|e| io_err("cannot pin standard project subfolder", e))?;
        if !pinned::list_child_names(&dir)
            .map_err(|e| io_err("cannot inspect standard project subfolder", e))?
            .is_empty()
        {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "newly created subfolder was not empty",
                sub,
            ));
        }
        subs.push((sub, dir.identity));
    }
    let children: BTreeSet<String> = pinned::list_child_names(&proj_dir)
        .map_err(|e| io_err("cannot inspect created project directory", e))?
        .into_iter()
        .collect();
    let expected: BTreeSet<String> = [
        "ops.jsonl",
        "project.json",
        "receipts",
        "proxies",
        "filmstrip",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    if children != expected {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "newly created project directory contents changed during creation",
            "",
        ));
    }
    Ok((
        proj_dir,
        creation_op,
        ops_file.identity,
        pj_file.identity,
        [subs[0], subs[1], subs[2]],
    ))
}

pub(super) fn pick_unique_project_name(root_dir: &pinned::PinnedDir) -> Result<String, CutError> {
    let existing: BTreeSet<String> = pinned::list_child_names(root_dir)
        .map_err(|e| {
            CutError::new(
                error_codes::IO,
                "cannot inspect projects root",
                e.to_string(),
            )
        })?
        .into_iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    for _ in 0..16 {
        let mut raw = [0u8; 4];
        getrandom::fill(&mut raw).map_err(|e| {
            CutError::new(
                error_codes::IO,
                "cannot generate project name suffix",
                e.to_string(),
            )
        })?;
        let name = format!(
            "Recording {:02x}{:02x}{:02x}{:02x}",
            raw[0], raw[1], raw[2], raw[3]
        );
        let dir_name = format!("{name}.cutproj").to_ascii_lowercase();
        if !existing.contains(&dir_name) {
            return Ok(name);
        }
    }
    Err(CutError::new(
        error_codes::CONFLICT,
        "could not pick an unused fallback recording project name",
        "existing project folders collided",
    ))
}

pub(super) fn verify_ops_file(
    file: &mut pinned::PinnedFile,
    expected_op: &str,
) -> Result<(), CutError> {
    let bytes = pinned::read_pinned_file_bounded(file, MAX_METADATA_BYTES)
        .map_err(|e| CutError::new(error_codes::IO, "cannot read ops.jsonl", e.to_string()))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| CutError::new(error_codes::CONFLICT, "ops.jsonl is not UTF-8", ""))?;
    let ops: Vec<OpRecord> = text
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()
        .map_err(|_| {
            CutError::new(
                error_codes::CONFLICT,
                "ops.jsonl changed from fallback baseline",
                "",
            )
        })?;
    if ops.len() != 1 || ops[0].verb != "project.create" || ops[0].op_id != expected_op {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "fallback project journal changed",
            "",
        ));
    }
    Ok(())
}

pub(super) fn verify_project_json_file(file: &mut pinned::PinnedFile) -> Result<(), CutError> {
    let bytes = pinned::read_pinned_file_bounded(file, MAX_METADATA_BYTES)
        .map_err(|e| CutError::new(error_codes::IO, "cannot read project.json", e.to_string()))?;
    let project: Project = serde_json::from_slice(&bytes)
        .map_err(|_| CutError::new(error_codes::CONFLICT, "project.json changed", ""))?;
    if !project.assets.is_empty() {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "fallback project gained assets",
            "",
        ));
    }
    Ok(())
}

pub(super) fn verify_root_to_project_link(
    root_dir: &pinned::PinnedDir,
    project_dir_name: &str,
    project_dir: &pinned::PinnedDir,
) -> Result<(), ()> {
    pinned::verify_dir_handle(root_dir).map_err(|_| ())?;
    pinned::verify_dir_handle(project_dir).map_err(|_| ())?;
    pinned::open_verified_child_dir_nofollow(root_dir, project_dir_name, project_dir.identity)
        .map(|_| ())
        .map_err(|_| ())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn verify_admitted_tree(
    root_dir: &pinned::PinnedDir,
    project_dir_name: &str,
    project_dir: &pinned::PinnedDir,
    creation_op: &str,
    ops_identity: pinned::FileIdentity,
    project_json_identity: pinned::FileIdentity,
    sub_dir_identities: &[(&'static str, pinned::DirIdentity); 3],
    capture_id: Option<&str>,
) -> Result<(), ()> {
    verify_root_to_project_link(root_dir, project_dir_name, project_dir)?;
    let entries: BTreeSet<String> = pinned::list_child_names(project_dir)
        .map_err(|_| ())?
        .into_iter()
        .collect();
    for required in [
        "ops.jsonl",
        "project.json",
        "receipts",
        "proxies",
        "filmstrip",
    ] {
        if !entries.contains(required) {
            return Err(());
        }
    }
    for name in &entries {
        let allowed = matches!(
            name.as_str(),
            "ops.jsonl" | "project.json" | "receipts" | "proxies" | "filmstrip"
        ) || (name == "cache" && capture_id.is_some());
        if !allowed {
            return Err(());
        }
    }
    for &(sub, sub_id) in sub_dir_identities {
        let dir =
            pinned::open_verified_child_dir_nofollow(project_dir, sub, sub_id).map_err(|_| ())?;
        if !pinned::list_child_names(&dir).map_err(|_| ())?.is_empty() {
            return Err(());
        }
    }
    if entries.contains("cache") {
        let cid = capture_id.ok_or(())?;
        let cache = pinned::open_child_dir_nofollow(project_dir, "cache").map_err(|_| ())?;
        if pinned::list_child_names(&cache).map_err(|_| ())? != ["screen_record"] {
            return Err(());
        }
        let sr = pinned::open_child_dir_nofollow(&cache, "screen_record").map_err(|_| ())?;
        if pinned::list_child_names(&sr).map_err(|_| ())? != [cid] {
            return Err(());
        }
        let cap = pinned::open_child_dir_nofollow(&sr, cid).map_err(|_| ())?;
        for entry_name in pinned::list_child_names(&cap).map_err(|_| ())? {
            if entry_name == "checkpoints" {
                let cp = pinned::open_child_dir_nofollow(&cap, "checkpoints").map_err(|_| ())?;
                if !pinned::list_child_names(&cp).map_err(|_| ())?.is_empty() {
                    return Err(());
                }
            } else if START_CAPTURE_FILES.contains(&entry_name.as_str()) {
                let _ = pinned::open_child_file_nofollow(&cap, &entry_name).map_err(|_| ())?;
            } else {
                return Err(());
            }
        }
    }
    let mut ops_file =
        pinned::open_verified_child_file_nofollow(project_dir, "ops.jsonl", ops_identity)
            .map_err(|_| ())?;
    verify_ops_file(&mut ops_file, creation_op).map_err(|_| ())?;
    let mut pj_file = pinned::open_verified_child_file_nofollow(
        project_dir,
        "project.json",
        project_json_identity,
    )
    .map_err(|_| ())?;
    verify_project_json_file(&mut pj_file).map_err(|_| ())?;
    Ok(())
}

pub(super) fn remove_start_capture_subtree(
    root_dir: &pinned::PinnedDir,
    project_dir_name: &str,
    project_dir: &pinned::PinnedDir,
    cid: &str,
) -> Result<(), ()> {
    verify_root_to_project_link(root_dir, project_dir_name, project_dir)?;
    let entries = pinned::list_child_names(project_dir).map_err(|_| ())?;
    if !entries.iter().any(|n| n == "cache") {
        return Ok(());
    }
    let cache = pinned::open_child_dir_nofollow(project_dir, "cache").map_err(|_| ())?;
    let sr = pinned::open_child_dir_nofollow(&cache, "screen_record").map_err(|_| ())?;
    let cap = pinned::open_child_dir_nofollow(&sr, cid).map_err(|_| ())?;
    let cap_entries = pinned::list_child_names(&cap).map_err(|_| ())?;
    for entry_name in cap_entries {
        if entry_name == "checkpoints" {
            let cp = pinned::open_child_dir_nofollow(&cap, "checkpoints").map_err(|_| ())?;
            if !pinned::list_child_names(&cp).map_err(|_| ())?.is_empty() {
                return Err(());
            }
            drop(cp);
            verify_root_to_project_link(root_dir, project_dir_name, project_dir)?;
            pinned::unlink_verified_child_dir_at(&cap, "checkpoints", None).map_err(|_| ())?;
        } else if START_CAPTURE_FILES.contains(&entry_name.as_str()) {
            verify_root_to_project_link(root_dir, project_dir_name, project_dir)?;
            pinned::unlink_verified_child_file_at(&cap, &entry_name, None).map_err(|_| ())?;
        } else {
            return Err(());
        }
    }
    drop(cap);
    verify_root_to_project_link(root_dir, project_dir_name, project_dir)?;
    pinned::unlink_verified_child_dir_at(&sr, cid, None).map_err(|_| ())?;
    drop(sr);
    verify_root_to_project_link(root_dir, project_dir_name, project_dir)?;
    pinned::unlink_verified_child_dir_at(&cache, "screen_record", None).map_err(|_| ())?;
    drop(cache);
    verify_root_to_project_link(root_dir, project_dir_name, project_dir)?;
    pinned::unlink_verified_child_dir_at(project_dir, "cache", None).map_err(|_| ())?;
    Ok(())
}

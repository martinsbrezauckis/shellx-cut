//! Global Library organization, removal, and project-import handlers.

use super::library_handlers::no_lib_item;
use super::*;

/// library.remove{id} — drop the item; unlink its blob if nothing else uses it.
pub(in crate::dispatch) async fn library_remove(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        id: String,
    }
    let a: Args = parse_args(args)?;
    let (removed, blob_to_remove) = crate::library::mutate_manifest(|m| {
        let removed = m.remove(&a.id);
        let blob_to_remove = removed
            .as_ref()
            .and_then(|item| item.blob.clone())
            .filter(|blob| !m.blob_referenced(blob));
        Ok((removed, blob_to_remove))
    })?;
    if let Some(blob) = &blob_to_remove {
        crate::library::remove_blob(blob);
    }
    Ok(VerbResult::ok(json!({ "removed": removed.is_some() })))
}

/// library.move{id, folder?} — move to a folder (omit/empty folder → root).
pub(in crate::dispatch) async fn library_move(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        id: String,
        folder: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let item = crate::library::mutate_manifest(|m| {
        if !m.move_to(&a.id, a.folder) {
            return Err(no_lib_item(&a.id));
        }
        Ok(m.items.iter().find(|i| i.id == a.id).cloned())
    })?;
    Ok(VerbResult::ok(json!({ "item": item })))
}

/// library.tag{id, tags} — replace the item's tag set.
pub(in crate::dispatch) async fn library_tag(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        id: String,
        tags: Vec<String>,
    }
    let a: Args = parse_args(args)?;
    let item = crate::library::mutate_manifest(|m| {
        if !m.set_tags(&a.id, a.tags) {
            return Err(no_lib_item(&a.id));
        }
        Ok(m.items.iter().find(|i| i.id == a.id).cloned())
    })?;
    Ok(VerbResult::ok(json!({ "item": item })))
}

/// library.favorite{id, on} — pin/unpin.
pub(in crate::dispatch) async fn library_favorite(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        id: String,
        on: bool,
    }
    let a: Args = parse_args(args)?;
    let item = crate::library::mutate_manifest(|m| {
        if !m.set_favorite(&a.id, a.on) {
            return Err(no_lib_item(&a.id));
        }
        Ok(m.items.iter().find(|i| i.id == a.id).cloned())
    })?;
    Ok(VerbResult::ok(json!({ "item": item })))
}

/// library.use{id} — bump the recently-used / use-count telemetry (drives sorts).
pub(in crate::dispatch) async fn library_use(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        id: String,
    }
    let a: Args = parse_args(args)?;
    let item = crate::library::mutate_manifest(|m| {
        if !m.bump_use(&a.id, crate::library::now_ms()) {
            return Err(no_lib_item(&a.id));
        }
        Ok(m.items.iter().find(|i| i.id == a.id).cloned())
    })?;
    Ok(VerbResult::ok(json!({ "item": item })))
}

/// library.add_to_project{id} — import the library asset into the OPEN project
/// (reuses media.import: hash/probe/proxy/transcribe/perception chain + auto-place),
/// then bump the library use counter. Requires an open project.
pub(in crate::dispatch) async fn library_add_to_project(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        id: String,
    }
    let a: Args = parse_args(args)?;
    let path = crate::library::with_manifest(|m| crate::library::item_media_path(m, &a.id))
        .ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!(
                    "library item '{}' not found, or its linked original is missing",
                    a.id
                ),
                "the linked source may have moved — re-add it, or use a copied (portable) item"
                    .to_string(),
            )
        })?;
    let mut result = media_import(state, json!({ "path": path.to_string_lossy() }), actor).await?;
    if let Err(e) = crate::library::mutate_manifest(|m| {
        if !m.bump_use(&a.id, crate::library::now_ms()) {
            return Err(no_lib_item(&a.id));
        }
        Ok(())
    }) {
        let mut detail = Map::new();
        detail.insert("library_id".into(), json!(a.id));
        detail.insert("cause".into(), json!(e.cause));
        result = result.with_warnings(vec![cut_core::VerbWarning {
            code: "library_use_not_recorded".into(),
            message: format!(
                "import succeeded, but library use could not be recorded: {}",
                e.message
            ),
            detail,
        }]);
    }
    Ok(result)
}

/// library.folder_add{name} — create an organization folder.
pub(in crate::dispatch) async fn library_folder_add(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        name: String,
    }
    let a: Args = parse_args(args)?;
    let (added, folders) = crate::library::mutate_manifest(|m| {
        let added = m.add_folder(&a.name);
        Ok((added, m.folders.clone()))
    })?;
    Ok(VerbResult::ok(
        json!({ "added": added, "folders": folders }),
    ))
}

/// library.folder_rename{old, new} — rename + re-point items.
pub(in crate::dispatch) async fn library_folder_rename(
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        old: String,
        new: String,
    }
    let a: Args = parse_args(args)?;
    let (renamed, folders) = crate::library::mutate_manifest(|m| {
        let renamed = m.rename_folder(&a.old, &a.new);
        Ok((renamed, m.folders.clone()))
    })?;
    Ok(VerbResult::ok(
        json!({ "renamed": renamed, "folders": folders }),
    ))
}

/// library.folder_remove{name} — remove a folder; its items move to root.
pub(in crate::dispatch) async fn library_folder_remove(
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        name: String,
    }
    let a: Args = parse_args(args)?;
    let (removed, folders) = crate::library::mutate_manifest(|m| {
        let removed = m.remove_folder(&a.name);
        Ok((removed, m.folders.clone()))
    })?;
    Ok(VerbResult::ok(
        json!({ "removed": removed, "folders": folders }),
    ))
}

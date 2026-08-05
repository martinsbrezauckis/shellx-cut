//! Global Library source, relink, and bounded-query handlers.

use super::*;

/// NOT_FOUND error for an unknown library item id.
pub(super) fn no_lib_item(id: &str) -> CutError {
    CutError::new(
        error_codes::NOT_FOUND,
        format!("no library item '{id}'"),
        "list items with library.list".to_string(),
    )
}

/// Resolve library.add's source: a `path` (a file to add) OR an `asset` id in the
/// OPEN project (→ its source path + already-computed content hash).
async fn resolve_library_source(
    state: &AppState,
    path: Option<String>,
    asset: Option<String>,
) -> Result<(PathBuf, Option<String>), CutError> {
    match (path, asset) {
        (Some(_), Some(_)) => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "library.add needs exactly one of path or asset",
                "pass a file path, or an in-project asset id, not both",
            ));
        }
        (Some(p), None) => return Ok((PathBuf::from(p), None)),
        (None, Some(aid)) => {
            let guard = state.project.read().await;
            let store = guard.as_ref().ok_or_else(no_project)?;
            let a = store.project.assets.get(&aid).ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no asset '{aid}' in the open project"),
                    "pass a path, or an asset id from project.state".to_string(),
                )
            })?;
            return Ok((PathBuf::from(&a.path), Some(a.hash.clone())));
        }
        (None, None) => {}
    }
    Err(CutError::new(
        error_codes::INVALID_ARGS,
        "library.add needs path or asset".to_string(),
        "pass a file path, or an in-project asset id".to_string(),
    ))
}

/// library.add{path?|asset?, name?, tags?, folder?, copy?, source?} — validate +
/// classify (ffprobe) + content-hash, then LINK (default) or COPY into the library.
pub(in crate::dispatch) async fn library_add(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        path: Option<String>,
        asset: Option<String>,
        name: Option<String>,
        tags: Option<Vec<String>>,
        folder: Option<String>,
        copy: Option<bool>,
        source: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let (name, tags, folder) = (a.name, a.tags.unwrap_or_default(), a.folder);
    let copy = a.copy.unwrap_or(false);
    let source = a.source.unwrap_or_else(|| "user".to_string());
    if !matches!(source.as_str(), "user" | "agent") {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown library.add source '{source}'"),
            "source must be 'user' or 'agent'",
        ));
    }
    let (path, known_hash) = resolve_library_source(state, a.path, a.asset).await?;
    let item = run_blocking("library.add", move || {
        crate::library::add_from_path(
            &path,
            name,
            tags,
            folder,
            copy,
            &source,
            known_hash,
            crate::library::now_ms(),
        )
    })
    .await?;
    Ok(VerbResult::ok(json!({ "item": item })))
}

/// library.relink{id,path} — repoint a missing linked item to the same media
/// bytes at a new path. Managed copies and different content are rejected.
pub(in crate::dispatch) async fn library_relink(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        id: String,
        path: String,
    }
    let a: Args = parse_args(args)?;
    let item = run_blocking("library.relink", move || {
        crate::library::relink_from_path(&a.id, Path::new(&a.path))
    })
    .await?;
    Ok(VerbResult::ok(json!({ "item": item })))
}

/// library.list{type?, folder?, tag?, q?, sort?, collection?, ids?, offset?, limit?}
/// → one bounded page plus exact totals and stable whole-library navigation facets.
pub(in crate::dispatch) async fn library_list(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        #[serde(rename = "type")]
        kind: Option<String>,
        folder: Option<String>,
        tag: Option<String>,
        q: Option<String>,
        sort: Option<String>,
        collection: Option<String>,
        ids: Option<Vec<String>>,
        offset: Option<usize>,
        limit: Option<usize>,
    }
    let a: Args = parse_args(args)?;
    let collection = a.collection.as_deref().unwrap_or("all");
    if !matches!(collection, "all" | "favorites" | "missing") {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown library collection '{collection}'"),
            "collection must be 'all', 'favorites', or 'missing'",
        ));
    }
    let offset = a.offset.unwrap_or(0);
    let limit = a
        .limit
        .unwrap_or(crate::library::LIBRARY_LIST_DEFAULT_LIMIT);
    if limit == 0 || limit > crate::library::LIBRARY_LIST_MAX_LIMIT {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "library.list limit must be between 1 and {}",
                crate::library::LIBRARY_LIST_MAX_LIMIT
            ),
            "request a bounded page",
        ));
    }
    if a.ids
        .as_ref()
        .is_some_and(|ids| ids.len() > crate::library::LIBRARY_LIST_MAX_IDS)
    {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "library.list accepts at most {} exact ids",
                crate::library::LIBRARY_LIST_MAX_IDS
            ),
            "split larger exact-membership checks into bounded batches",
        ));
    }
    let ids = a
        .ids
        .map(|ids| ids.into_iter().collect::<std::collections::HashSet<_>>());
    let sort = a.sort.as_deref().unwrap_or("added");
    // media_ok is COMPUTED for the requested page at read time (a stat on the resolved
    // source/blob — never a persisted flag, same doctrine as media.check):
    // a linked original that went missing reads honestly as false, so the UI
    // can show the kind glyph WITHOUT requesting a poster that can only 404
    // (each 404 is a console error line in the webview) and agents see dead
    // links in library.list instead of discovering them on use.
    let (items, folders, tags, total, next_offset) = crate::library::with_manifest(|m| {
        let blobs_dir = crate::userdata::library_blobs_dir();
        let mut matches = m.query_refs(
            a.kind.as_deref(),
            a.folder.as_deref(),
            a.tag.as_deref(),
            a.q.as_deref(),
            collection == "favorites",
            ids.as_ref(),
            sort,
        );
        // Missing is the only collection that must stat every candidate because
        // file existence itself defines membership. All other views stat only the
        // bounded page below.
        if collection == "missing" {
            matches.retain(|item| {
                crate::library::item_media_path_from_item(item, blobs_dir.as_deref()).is_none()
            });
        }
        let total = matches.len();
        let (start, end, next_offset) = crate::library::page_bounds(total, offset, limit);
        let items: Vec<Value> = matches[start..end]
            .iter()
            .copied()
            .map(|it| {
                let mut v = serde_json::to_value(it).unwrap_or_else(|_| json!({}));
                if let Some(obj) = v.as_object_mut() {
                    obj.insert(
                        "media_ok".to_string(),
                        json!(
                            crate::library::item_media_path_from_item(it, blobs_dir.as_deref())
                                .is_some()
                        ),
                    );
                }
                v
            })
            .collect();
        (items, m.folders.clone(), m.tag_facets(), total, next_offset)
    });
    Ok(VerbResult::ok(json!({
        "items": items,
        "folders": folders,
        "tags": tags,
        "total": total,
        "offset": offset,
        "limit": limit,
        "next_offset": next_offset
    })))
}

//! Bounded readable-picture reports for media.check.

use super::*;

/// media.check{asset?} — read-only OFFLINE-media report. Existence is computed
/// from the filesystem at call time, never persisted (a stored offline flag can
/// go stale in both directions — the filesystem is the source of truth). For
/// each asset: does its source file exist, when was it modified, and how many
/// timeline clips reference it. The UI polls this for offline badges and recent
/// smart-bin filters; agents run it before render.
/// Result: {count, offline_count, assets:[{asset, path, exists, readable?,
/// modified_ms?, referenced}]}. An existing picture without a cached or
/// completed verdict omits `readable` until another bounded request checks it.
pub(crate) async fn media_check(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    media_check_with(state, args, decode_picture_with_deadline).await
}

pub(super) async fn decode_picture_with_deadline(
    source: PathBuf,
    deadline: std::time::Instant,
    pass: crate::picture_checks::PassToken,
) -> Result<Option<bool>, CutError> {
    run_blocking("media.check.picture", move || {
        let _pass = pass;
        let budget = deadline.saturating_duration_since(std::time::Instant::now());
        match cut_media::picture_decode::check_with_budget(&source, budget) {
            Ok(()) => Ok(Some(true)),
            Err(error) if error.code == error_codes::UNREADABLE_MEDIA => Ok(Some(false)),
            Err(error)
                if error.message == "operation timed out"
                    && budget < std::time::Duration::from_secs(10) =>
            {
                Ok(None)
            }
            Err(error) if error.message == "operation timed out" => Ok(Some(false)),
            Err(error) => Err(error),
        }
    })
    .await
}

pub(super) async fn media_check_with<F, Fut>(
    state: &AppState,
    args: Value,
    decode: F,
) -> Result<VerbResult, CutError>
where
    F: Fn(PathBuf, std::time::Instant, crate::picture_checks::PassToken) -> Fut,
    Fut: std::future::Future<Output = Result<Option<bool>, CutError>>,
{
    #[derive(serde::Deserialize)]
    struct Args {
        asset: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    // Snapshot only the report's facts, not transcripts or whole tracks. The
    // bounded decoder must never hold the project lock while it runs.
    let sources = store
        .project
        .assets
        .iter()
        .filter(|(id, _)| a.asset.as_ref().is_none_or(|want| want == *id))
        .map(|(id, asset)| {
            let referenced = store
                .project
                .all_sequence_tracks()
                .flat_map(|track| &track.clips)
                .filter(|clip| matches!(clip, cut_core::Clip::Media(media) if &media.asset == id))
                .count();
            (
                id.clone(),
                asset.path.clone(),
                asset_source_path(&store.dir, asset),
                asset
                    .probe
                    .as_ref()
                    .and_then(|probe| probe["kind"].as_str())
                    == Some("image"),
                referenced,
            )
        })
        .collect::<Vec<_>>();
    drop(guard);
    let mut rows = Vec::new();
    let mut offline = 0usize;
    // A concurrent caller reports cached verdicts and pending pictures. It
    // never queues another decoder pass behind the current one.
    let pass = state.picture_checks.try_begin();
    let deadline = std::time::Instant::now() + crate::picture_checks::PASS_BUDGET;
    let mut decoded = 0usize;
    for (id, path, p, image, referenced) in sources {
        let key = crate::picture_checks::SourceKey::capture(&p);
        let exists = key.is_some();
        let modified_ms = if exists { source_modified_ms(&p) } else { None };
        let readable = if !exists {
            Some(false)
        } else if !image {
            Some(true)
        } else {
            let key = key.expect("existing file has an identity");
            if let Some(verdict) = state.picture_checks.get(&key) {
                Some(verdict)
            } else if let Some(pass) = pass.as_ref().filter(|_| {
                decoded < crate::picture_checks::MAX_PICTURES_PER_PASS
                    && std::time::Instant::now() < deadline
            }) {
                decoded += 1;
                let outcome = tokio::time::timeout_at(
                    tokio::time::Instant::from_std(deadline),
                    decode(p.clone(), deadline, pass.clone()),
                )
                .await;
                let (verdict, cacheable) = match outcome {
                    Ok(Ok(verdict)) => (verdict, verdict.is_some()),
                    Ok(Err(_)) => (None, false),
                    Err(_) => (None, false),
                };
                // File replacement during decode cannot give the replacement
                // the old source's verdict.
                if crate::picture_checks::SourceKey::capture(&p).as_ref() == Some(&key) {
                    if let Some(verdict) = verdict.filter(|_| cacheable) {
                        state.picture_checks.insert(key, verdict);
                    }
                    verdict
                } else {
                    None
                }
            } else {
                None
            }
        };
        if readable == Some(false) {
            offline += 1;
        }
        let mut row = json!({
            "asset": id,
            "path": path,
            "exists": exists,
            "modified_ms": modified_ms,
            "referenced": referenced,
        });
        if let Some(readable) = readable {
            row["readable"] = json!(readable);
        }
        rows.push(row);
    }
    if a.asset.is_some() && rows.is_empty() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            "no such asset".to_string(),
            "list assets via project.state (project.assets)".to_string(),
        ));
    }
    Ok(VerbResult::ok(json!({
        "count": rows.len(),
        "offline_count": offline,
        "assets": rows,
    })))
}

//! Revision-bound, page-bounded registered-media checks for project health.

use super::*;
use std::cmp::Ordering;

#[derive(Clone, Copy)]
enum DerivedState {
    Available,
    Missing,
    NotRecorded,
    NotApplicable,
}

impl DerivedState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Missing => "missing",
            Self::NotRecorded => "not_recorded",
            Self::NotApplicable => "not_applicable",
        }
    }
}

/// Check one stable asset-id page. Callers have already validated journal
/// identity and the revision continuation, so this never scans all asset files.
pub(super) fn checked_media_page(
    store: &ProjectStore,
    cursor: Option<&str>,
    limit: usize,
) -> Result<Value, CutError> {
    let mut asset_ids: Vec<&String> = store.project.assets.keys().collect();
    asset_ids.sort_by(|left, right| compare_asset_ids(left, right));
    let start = match cursor {
        None => 0,
        Some(cursor) => asset_ids
            .iter()
            .position(|id| id.as_str() == cursor)
            .map(|position| position + 1)
            .ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("asset cursor '{cursor}' is not in the current project"),
                    "asset pages are revision-bound; restart project.health without cursor",
                )
            })?,
    };
    let end = start.saturating_add(limit).min(asset_ids.len());
    let mut rows = Vec::with_capacity(end.saturating_sub(start));
    let mut counts = PageCounts::default();
    for asset_id in &asset_ids[start..end] {
        let asset = store
            .project
            .assets
            .get(asset_id.as_str())
            .expect("asset id came from the current project map");
        let source_available = source_is_available(&store.dir, asset);
        counts.offline += usize::from(!source_available);
        let proxy = proxy_state(&store.dir, asset_id, asset);
        let filmstrip = filmstrip_state(&store.dir, asset_id, asset);
        counts.proxy.add(proxy);
        counts.filmstrip.add(filmstrip);
        rows.push(json!({
            "asset": asset_id,
            "source": if source_available { "available" } else { "offline" },
            "proxy": proxy.as_str(),
            "filmstrip": filmstrip.as_str(),
        }));
    }
    let has_more = end < asset_ids.len();
    let mut page = serde_json::Map::new();
    page.insert("status".into(), json!("ready"));
    page.insert("asset_count".into(), json!(asset_ids.len()));
    page.insert("checked_count".into(), json!(rows.len()));
    page.insert("page".into(), counts.as_value());
    page.insert("assets".into(), json!(rows));
    page.insert("limit".into(), json!(limit));
    if let Some(cursor) = cursor {
        page.insert("cursor".into(), json!(cursor));
    }
    if has_more {
        page.insert("next_cursor".into(), json!(asset_ids[end - 1]));
    }
    page.insert("has_more".into(), json!(has_more));
    Ok(Value::Object(page))
}

pub(super) fn unavailable_media_page(limit: usize) -> Value {
    json!({
        "status": "unavailable",
        "asset_count": 0,
        "checked_count": 0,
        "page": PageCounts::default().as_value(),
        "assets": [],
        "limit": limit,
        "has_more": false,
    })
}

#[derive(Default)]
struct DerivedCounts {
    available: usize,
    missing: usize,
    not_recorded: usize,
    not_applicable: usize,
}

impl DerivedCounts {
    fn add(&mut self, state: DerivedState) {
        match state {
            DerivedState::Available => self.available += 1,
            DerivedState::Missing => self.missing += 1,
            DerivedState::NotRecorded => self.not_recorded += 1,
            DerivedState::NotApplicable => self.not_applicable += 1,
        }
    }
}

#[derive(Default)]
struct PageCounts {
    offline: usize,
    proxy: DerivedCounts,
    filmstrip: DerivedCounts,
}

impl PageCounts {
    fn as_value(&self) -> Value {
        json!({
            "offline": self.offline,
            "proxy_available": self.proxy.available,
            "proxy_missing": self.proxy.missing,
            "proxy_not_recorded": self.proxy.not_recorded,
            "proxy_not_applicable": self.proxy.not_applicable,
            "filmstrip_available": self.filmstrip.available,
            "filmstrip_missing": self.filmstrip.missing,
            "filmstrip_not_recorded": self.filmstrip.not_recorded,
            "filmstrip_not_applicable": self.filmstrip.not_applicable,
        })
    }
}

fn source_is_available(project_dir: &Path, asset: &cut_core::Asset) -> bool {
    let source = PathBuf::from(&asset.path);
    let source = if source.is_relative() {
        project_dir.join(source)
    } else {
        source
    };
    source.is_file()
}

fn proxy_state(project_dir: &Path, asset_id: &str, asset: &cut_core::Asset) -> DerivedState {
    if probe_kind(asset).is_some_and(|kind| kind != "video") {
        return DerivedState::NotApplicable;
    }
    derived_state(
        asset.proxy.is_some(),
        project_dir,
        asset_id,
        "proxies",
        "mp4",
    )
}

fn filmstrip_state(project_dir: &Path, asset_id: &str, asset: &cut_core::Asset) -> DerivedState {
    if probe_kind(asset) == Some("audio") {
        return DerivedState::NotApplicable;
    }
    derived_state(
        asset.filmstrip.is_some(),
        project_dir,
        asset_id,
        "filmstrip",
        "jpg",
    )
}

fn probe_kind(asset: &cut_core::Asset) -> Option<&str> {
    asset.probe.as_ref()?.get("kind")?.as_str()
}

fn derived_state(
    recorded: bool,
    project_dir: &Path,
    asset_id: &str,
    directory: &str,
    extension: &str,
) -> DerivedState {
    if !recorded {
        return DerivedState::NotRecorded;
    }
    if !is_generated_asset_id(asset_id) {
        return DerivedState::Missing;
    }
    let path = project_dir
        .join(directory)
        .join(format!("{asset_id}.{extension}"));
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => DerivedState::Available,
        _ => DerivedState::Missing,
    }
}

fn is_generated_asset_id(asset_id: &str) -> bool {
    asset_id.strip_prefix('a').is_some_and(|number| {
        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn compare_asset_ids(left: &str, right: &str) -> Ordering {
    match (asset_number(left), asset_number(right)) {
        (Some(left_number), Some(right_number)) => {
            left_number.cmp(&right_number).then_with(|| left.cmp(right))
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => left.cmp(right),
    }
}

fn asset_number(asset_id: &str) -> Option<u64> {
    asset_id.strip_prefix('a')?.parse().ok()
}

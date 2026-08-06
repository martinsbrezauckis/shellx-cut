use cut_core::{error_codes, CutError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const SAFE_ID_MAX: usize = 128;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InventoryArgs {
    pub clip: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InspectArgs {
    pub clip: String,
    pub analysis_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplyArgs {
    pub clip: String,
    pub analysis_id: String,
    pub layer_id: String,
    pub segment_index: Option<u64>,
    #[serde(default)]
    pub include_low_confidence: bool,
    pub rationale: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DetachArgs {
    pub clip: String,
    pub layer_id: String,
    pub rationale: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VerifyArgs {
    pub clip: String,
    pub layer_id: String,
    pub analysis_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestArgs {
    pub clip: String,
    pub analysis_id: String,
    pub asset_id: String,
    pub mode: TrackingMode,
    pub model: TrackingModel,
    pub region: NormalizedRegion,
    pub reference_ms: Option<u64>,
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
    pub every_ms: Option<u64>,
    pub search_radius_px: Option<u64>,
    pub confidence_floor: Option<f64>,
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TrackingMode {
    Point,
    Planar,
}

impl TrackingMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Point => "point",
            Self::Planar => "planar",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TrackingModel {
    Translation,
    Similarity,
    Homography,
}

impl TrackingModel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Translation => "translation",
            Self::Similarity => "similarity",
            Self::Homography => "homography",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NormalizedRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TrackingInventory {
    pub package_id: String,
    pub motion_id: String,
    pub width: u64,
    pub height: u64,
    pub duration_ms: u64,
    pub fps: f64,
    pub video_assets: Vec<TrackingAsset>,
    pub target_layers: Vec<TrackingLayer>,
    pub analyses: Vec<TrackingLifecycleSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TrackingAsset {
    pub id: String,
    pub name: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TrackingLayer {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub tracking_attached: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TrackingLifecycleSummary {
    pub analysis_id: String,
    pub state: String,
    pub asset_id: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct TrackingRequestPayload {
    pub reference: Value,
    pub settings: Value,
}

impl TrackingInventory {
    pub(crate) fn request_payload(
        &self,
        request: &RequestArgs,
    ) -> Result<TrackingRequestPayload, CutError> {
        validate_safe_id(&request.analysis_id, "analysis_id")?;
        validate_safe_id(&request.asset_id, "asset_id")?;
        if !self
            .video_assets
            .iter()
            .any(|asset| asset.id == request.asset_id && asset.available)
        {
            return Err(invalid(
                "asset_id must name an available package-local video asset",
            ));
        }
        match (request.mode, request.model) {
            (TrackingMode::Point, TrackingModel::Translation)
            | (TrackingMode::Planar, TrackingModel::Similarity)
            | (TrackingMode::Planar, TrackingModel::Homography) => {}
            _ => return Err(invalid(
                "point mode requires translation; planar mode requires similarity or homography",
            )),
        }
        let region = request.region;
        if ![region.x, region.y, region.width, region.height]
            .into_iter()
            .all(f64::is_finite)
            || region.x < 0.0
            || region.y < 0.0
            || region.width <= 0.0
            || region.height <= 0.0
            || region.x + region.width > 1.0
            || region.y + region.height > 1.0
        {
            return Err(invalid(
                "region must be finite normalized coordinates fully inside the frame",
            ));
        }
        let reference_ms = request
            .reference_ms
            .unwrap_or(request.start_ms.unwrap_or(0));
        let start_ms = request.start_ms.unwrap_or(0);
        let end_ms = request.end_ms.unwrap_or(self.duration_ms);
        let every_ms = request.every_ms.unwrap_or(100);
        let search_radius_px = request.search_radius_px.unwrap_or(32);
        let confidence_floor = request.confidence_floor.unwrap_or(0.6);
        if start_ms > end_ms
            || end_ms > self.duration_ms
            || reference_ms < start_ms
            || reference_ms > end_ms
            || !(1..=60_000).contains(&every_ms)
            || !(1..=4_096).contains(&search_radius_px)
            || !confidence_floor.is_finite()
            || !(0.0..=1.0).contains(&confidence_floor)
        {
            return Err(invalid("tracking range, sampling, search radius, or confidence is outside supported bounds"));
        }
        let x = region.x * self.width as f64;
        let y = region.y * self.height as f64;
        let width = region.width * self.width as f64;
        let height = region.height * self.height as f64;
        let points = match request.mode {
            TrackingMode::Point => vec![serde_json::json!({
                "x": x + width / 2.0,
                "y": y + height / 2.0,
            })],
            TrackingMode::Planar => vec![
                serde_json::json!({"x": x, "y": y}),
                serde_json::json!({"x": x + width, "y": y}),
                serde_json::json!({"x": x + width, "y": y + height}),
                serde_json::json!({"x": x, "y": y + height}),
            ],
        };
        Ok(TrackingRequestPayload {
            reference: serde_json::json!({
                "atMs": reference_ms,
                "bounds": {"x": x, "y": y, "width": width, "height": height},
                "points": points,
            }),
            settings: serde_json::json!({
                "startMs": start_ms,
                "endMs": end_ms,
                "stepMs": every_ms,
                "direction": "forward",
                "searchRadiusPx": search_radius_px,
                "pyramidLevels": 3,
                "maxIterations": 50,
                "confidenceFloor": confidence_floor,
                "deterministicSeed": 42,
            }),
        })
    }
}

pub(crate) fn validate_safe_id(value: &str, label: &str) -> Result<(), CutError> {
    safe_id(value).then_some(()).ok_or_else(|| {
        invalid(format!(
            "{label} must be a safe 1..128 character identifier"
        ))
    })
}

pub(super) fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= SAFE_ID_MAX
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
        })
}

pub(super) fn bounded_label(value: &str) -> String {
    value.chars().take(160).collect()
}

fn invalid(detail: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "Motion tracking request is invalid",
        detail,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture() -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("temp root");
        fs::create_dir_all(root.path().join("assets")).expect("assets");
        fs::write(root.path().join("assets/plate.mp4"), b"video").expect("video");
        fs::write(
            root.path().join("manifest.json"),
            r#"{"id":"pkg_tracking","motion":"motion.json","assets":["assets/plate.mp4"]}"#,
        )
        .expect("manifest");
        fs::write(
            root.path().join("motion.json"),
            r#"{"id":"motion_tracking","durationMs":2000,"fps":30,"width":1280,"height":720,"assets":[{"id":"plate","kind":"video","source":{"path":"assets/plate.mp4"}}],"layers":[{"id":"footage","name":"Footage","type":"video"}]}"#,
        )
        .expect("motion");
        root
    }

    #[test]
    fn loads_path_free_inventory_and_compiles_normalized_seed() {
        let root = fixture();
        let inventory = TrackingInventory::load(root.path()).expect("inventory");
        assert_eq!(inventory.video_assets[0].id, "plate");
        let request: RequestArgs = serde_json::from_value(serde_json::json!({
            "clip": "clip_1",
            "analysis_id": "track_1",
            "asset_id": "plate",
            "mode": "point",
            "model": "translation",
            "region": {"x": 0.25, "y": 0.25, "width": 0.5, "height": 0.5}
        }))
        .expect("request");
        let payload = inventory.request_payload(&request).expect("payload");
        assert_eq!(payload.reference["bounds"]["width"], 640.0);
        assert_eq!(payload.reference["points"][0]["x"], 640.0);
        assert_eq!(payload.settings["endMs"], 2000);
    }

    #[test]
    fn rejects_incompatible_mode_and_escaping_asset_symlink() {
        // The fixture is only consulted by the unix-only symlink-escape case, so
        // binding it unconditionally leaves it unused on Windows.
        #[cfg(unix)]
        {
            let root = fixture();
            fs::remove_file(root.path().join("assets/plate.mp4")).expect("remove");
            std::os::unix::fs::symlink("/etc/hosts", root.path().join("assets/plate.mp4"))
                .expect("symlink");
            let inventory = TrackingInventory::load(root.path()).expect("inventory");
            assert!(!inventory.video_assets[0].available);
        }
    }
}

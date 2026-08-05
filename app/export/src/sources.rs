//! Unique-source collection shared by the XML serializers.
//!
//! Every format wants the distinct source FILES in first-use document order
//! (video clips first, then audio layers) plus the probe-derived facts from
//! Export mapping inputs: path, stem, has A/V, channels, full SOURCE duration in frames,
//! SMPTE start, colorSpace. Missing probe degrades to safe defaults; source
//! duration falls back to the furthest source out-point any clip uses (the
//! minimum length that keeps every clip in range).

use std::collections::HashMap;

use crate::error::ExportError;
use crate::model::{file_stem, parse_smpte, ExportTimeline};
use crate::quantize::{Quantized, XItem};

/// Facts about one unique source file, ready for any serializer.
#[derive(Debug, Clone)]
pub struct SourceInfo {
    /// Asset-map key.
    pub asset_id: String,
    /// Absolute path as stored on the asset.
    pub path: String,
    /// Display name (file stem).
    pub stem: String,
    pub has_video: bool,
    pub has_audio: bool,
    /// Audio channel count. Missing probe is treated as mono so Premiere xmeml
    /// never invents a non-existent second channel for exploded stereo.
    pub audio_channels: u32,
    /// Full SOURCE media duration in frames (FCPXML asset duration rule).
    pub duration_frames: i64,
    /// Embedded SMPTE start timecode in frames (0 for ~all our footage).
    pub smpte_start: i64,
    /// FCPXML colorSpace string.
    pub color_space: String,
}

/// Collect unique sources in first-use order across video then audio items.
pub fn collect_sources(tl: &ExportTimeline, q: &Quantized) -> Result<Vec<SourceInfo>, ExportError> {
    let mut order: Vec<String> = Vec::new();
    let mut max_out: HashMap<String, i64> = HashMap::new();

    let mut visit = |items: &[XItem]| {
        for item in items {
            if let XItem::Clip(c) = item {
                if !order.contains(&c.asset) {
                    order.push(c.asset.clone());
                }
                let e = max_out.entry(c.asset.clone()).or_insert(0);
                *e = (*e).max(c.offset + c.dur);
            }
        }
    };
    for track in &q.video_tracks {
        visit(&track.items);
    }
    for layer in &q.audio {
        visit(layer);
    }

    order
        .into_iter()
        .map(|asset_id| {
            let asset = tl
                .assets
                .get(&asset_id)
                .ok_or_else(|| ExportError::MissingAsset {
                    clip_id: "?".to_string(),
                    asset: asset_id.clone(),
                })?;
            let probe = asset.probe.clone().unwrap_or_default();
            let duration_frames = match probe.duration_ms {
                Some(ms) => q.tb.frames_from_ms(ms)?,
                None => max_out.get(&asset_id).copied().unwrap_or(0),
            };
            let smpte_start = probe
                .timecode
                .as_deref()
                .map(|tc| parse_smpte(tc, q.tb.rounded()))
                .unwrap_or(0);
            Ok(SourceInfo {
                stem: file_stem(&asset.path),
                path: asset.path.clone(),
                has_video: probe.has_video.unwrap_or(true),
                has_audio: probe.has_audio.unwrap_or(true),
                audio_channels: probe.audio_channels.unwrap_or(1).max(1),
                duration_frames,
                smpte_start,
                color_space: color_space(&probe),
                asset_id,
            })
        })
        .collect()
}

/// FCPXML colorSpace from probe facts (using the
/// getColorspace). Wrong value changes color management, never breaks import —
/// the Rec. 709 default is the safe fallback for ordinary SDR footage.
fn color_space(p: &crate::model::AssetProbe) -> String {
    if p.pix_fmt.as_deref() == Some("rgb24") {
        return "sRGB IEC61966-2.1".into();
    }
    if p.color_primaries.as_deref() == Some("bt2020") {
        // HLG/PQ transfer -> HLG variant, otherwise plain Rec. 2020.
        return match p.color_transfer.as_deref() {
            Some("smpte2084") | Some("arib-std-b67") => "9-18-9 (Rec. 2020 HLG)".into(),
            _ => "9-1-9 (Rec. 2020)".into(),
        };
    }
    match p.color_space.as_deref() {
        Some("bt470bg") => "5-1-6 (Rec. 601 PAL)".into(),
        Some("smpte170m") => "6-1-6 (Rec. 601 NTSC)".into(),
        _ => "1-1-1 (Rec. 709)".into(),
    }
}

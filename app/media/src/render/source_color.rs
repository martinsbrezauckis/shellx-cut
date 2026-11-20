//! What colour a source really carries, and what a render does with it.
//!
//! Every export is SDR BT.709 processed in 8 bits: the render graph pins each
//! clip to `yuv420p`. An HDR source (HLG `arib-std-b67` or PQ `smpte2084`
//! transfer, usually BT.2020 primaries, 10-bit) used to be squeezed into that
//! 8-bit picture with its HDR tags carried through, which made a wrong file:
//! 8-bit video that players treat as HDR. Now an HDR source is tone-mapped to
//! SDR BT.709 (zscale + tonemap, the frames leave tagged BT.709), and the plan,
//! the receipt and the job result say what happened to every source. SDR
//! sources are untouched, so ordinary renders keep their exact bytes.
//! Keeping HDR in the export (10-bit HLG/PQ output) is not implemented.

use serde_json::Value;

/// The HDR transfer of a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hdr {
    /// Hybrid log-gamma (`arib-std-b67`).
    Hlg,
    /// Perceptual quantizer (`smpte2084`, HDR10).
    Pq,
}

impl Hdr {
    /// The zscale transfer name.
    fn zscale_transfer(self) -> &'static str {
        match self {
            Hdr::Hlg => "arib-std-b67",
            Hdr::Pq => "smpte2084",
        }
    }

    /// Short name for people ("HLG", "PQ").
    pub fn label(self) -> &'static str {
        match self {
            Hdr::Hlg => "HLG",
            Hdr::Pq => "PQ (HDR10)",
        }
    }
}

/// Colour facts of a source's first video stream, from its stored probe.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceColor {
    pub transfer: Option<String>,
    pub primaries: Option<String>,
    pub matrix: Option<String>,
    pub range: Option<String>,
    pub bit_depth: Option<u8>,
    pub hdr: Option<Hdr>,
}

fn known(v: &Value) -> Option<String> {
    v.as_str()
        .filter(|s| !s.is_empty() && *s != "unknown" && *s != "unspecified" && *s != "reserved")
        .map(String::from)
}

/// Bits per sample from ffprobe's `bits_per_raw_sample` or the pixel format.
fn bit_depth(stream: &Value) -> Option<u8> {
    if let Some(b) = stream["bits_per_raw_sample"]
        .as_str()
        .and_then(|s| s.parse::<u8>().ok())
        .filter(|b| *b > 0)
    {
        return Some(b);
    }
    let pix = stream["pix_fmt"].as_str()?;
    Some(if pix.contains("16") {
        16
    } else if pix.contains("12") {
        12
    } else if pix.contains("10") || pix == "p010le" || pix == "p010be" {
        10
    } else {
        8
    })
}

/// Read the colour facts of an asset (its stored `probe.raw`). Stills and
/// unprobed assets give the default (nothing known, not HDR).
pub fn source_color(asset: &cut_core::Asset) -> SourceColor {
    match asset.probe.as_ref() {
        Some(probe) => source_color_from_probe(probe),
        None => SourceColor::default(),
    }
}

/// [`source_color`] from a stored probe value (`asset.probe`).
pub fn source_color_from_probe(probe: &Value) -> SourceColor {
    if probe.get("kind").and_then(Value::as_str) == Some("image") {
        return SourceColor::default();
    }
    let streams = probe["raw"]["streams"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let Some(video) = streams.iter().find(|s| {
        s["codec_type"] == "video" && s["disposition"]["attached_pic"].as_i64().unwrap_or(0) == 0
    }) else {
        return SourceColor::default();
    };
    let transfer = known(&video["color_transfer"]);
    let hdr = match transfer.as_deref() {
        Some("arib-std-b67") => Some(Hdr::Hlg),
        Some("smpte2084") => Some(Hdr::Pq),
        _ => None,
    };
    SourceColor {
        transfer,
        primaries: known(&video["color_primaries"]),
        matrix: known(&video["color_space"]),
        range: known(&video["color_range"]),
        bit_depth: bit_depth(video),
        hdr,
    }
}

/// SECURITY: probe strings come from the media file's own metadata, which a
/// stranger may have written. Nothing from a probe is ever put into a filter
/// graph as-is: each value is mapped through a closed allow-list to a constant
/// spelling zscale accepts, and anything else falls back to the HDR default.
/// (A raw value such as `bt709,movie=/etc/passwd` would otherwise add a filter
/// that reads files.)
fn zscale_primaries(probe: Option<&str>) -> &'static str {
    match probe {
        Some("bt709") => "bt709",
        Some("smpte170m") => "smpte170m",
        Some("smpte240m") => "smpte240m",
        Some("smpte431") => "smpte431",
        Some("smpte432") => "smpte432",
        // bt2020 and every unknown or unexpected value.
        _ => "bt2020",
    }
}

/// See [`zscale_primaries`]: allow-listed matrix spelling.
fn zscale_matrix(probe: Option<&str>) -> &'static str {
    match probe {
        Some("bt2020c") => "bt2020c",
        Some("bt709") => "bt709",
        _ => "bt2020nc",
    }
}

/// See [`zscale_primaries`]: allow-listed range spelling.
fn zscale_range(probe: Option<&str>) -> &'static str {
    match probe {
        Some("pc") => "pc",
        _ => "tv",
    }
}

/// HDR -> SDR BT.709 tone map, as a leading-comma filter chain. Every hop
/// states its input explicitly, so the result does not depend on how earlier
/// filters tagged the frame; the last hop tags the frame BT.709, which the
/// encoder writes into the file. Every interpolated token is a constant (the
/// transfer comes from the [`Hdr`] enum, the rest from allow-lists).
pub fn hdr_to_sdr_filter(c: &SourceColor) -> String {
    let Some(hdr) = c.hdr else {
        return String::new();
    };
    let pin = zscale_primaries(c.primaries.as_deref());
    let min = zscale_matrix(c.matrix.as_deref());
    let rin = zscale_range(c.range.as_deref());
    format!(
        ",zscale=tin={t}:pin={pin}:min={min}:rin={rin}:t=linear:npl=100,format=gbrpf32le,\
         zscale=tin=linear:pin={pin}:p=bt709,tonemap=tonemap=hable:desat=0,\
         zscale=tin=linear:pin=bt709:t=bt709:p=bt709:m=bt709:r=tv",
        t = hdr.zscale_transfer()
    )
}

/// Re-tag frames as BT.709 without touching pixels: for an HDR source the
/// person tagged rec709 with `edit.color_space` (they chose no conversion),
/// so its HDR tags never reach an 8-bit file.
pub const RETAG_BT709: &str =
    ",setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv";

/// The per-clip colour-management hops at the END of the chain. A source the
/// PREP slot converted (HDR tone map, BT.601 matrix; see `source_prep`) is
/// BT.709 from there on, so its hops start from BT.709. With no such source
/// this is exactly the old chain, so ordinary renders keep their bytes.
pub(super) fn clip_color_chain(
    asset: Option<&cut_core::Asset>,
    input: Option<&cut_core::ColorSpace>,
    color: &cut_core::ColorConfig,
) -> String {
    let source = asset.map(source_color).unwrap_or_default();
    match (source.hdr, input) {
        (Some(_), None) => super::colorspace_filter(Some(&cut_core::ColorSpace::Rec709), color),
        (None, None) if super::source_prep::is_bt601(&source) => {
            super::colorspace_filter(Some(&cut_core::ColorSpace::Rec709), color)
        }
        (Some(_), Some(tag)) => {
            let hops = super::colorspace_filter(Some(tag), color);
            if hops.is_empty() {
                RETAG_BT709.to_string()
            } else {
                hops
            }
        }
        (None, _) => super::colorspace_filter(input, color),
    }
}

/// True when this source is HDR and untagged by the person, so a render of it
/// runs the zscale tone map (it needs an ffmpeg built with zimg).
pub fn needs_tone_map(asset: &cut_core::Asset, input: Option<&cut_core::ColorSpace>) -> bool {
    input.is_none() && source_color(asset).hdr.is_some()
}

/// Handling names (also the receipt schema enum).
pub const HANDLING_SDR: &str = "sdr";
pub const HANDLING_TONE_MAPPED: &str = "tone_mapped_to_sdr";
pub const HANDLING_TAGGED_NOT_CONVERTED: &str = "hdr_tagged_not_converted";
pub const HANDLING_BT601_CONVERTED: &str = "converted_bt601_to_bt709";

/// Bits per sample of the encoded file, from the encoder's `-pix_fmt`.
fn file_bit_depth(video_args: &[String]) -> u8 {
    let pix = video_args
        .windows(2)
        .find(|w| w[0] == "-pix_fmt")
        .map(|w| w[1].as_str())
        .unwrap_or("yuv420p");
    if pix.contains("10") || pix.starts_with("p010") {
        10
    } else {
        8
    }
}

/// What a render does with colour, for the plan, the receipt and the job.
/// `video_args` are the resolved encoder arguments.
pub fn color_report(
    project: &cut_core::Project,
    edl: &cut_core::Edl,
    video_args: &[String],
) -> cut_core::ColorReport {
    let mut seen = std::collections::BTreeMap::<String, Option<cut_core::ColorSpace>>::new();
    for seg in edl
        .segments
        .iter()
        .filter(|s| s.track_kind == cut_core::TrackKind::Video)
    {
        if let Some(a) = seg.asset.as_deref() {
            seen.entry(a.to_string()).or_insert(seg.input_color_space);
        }
    }
    let mut sources = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for (id, tag) in &seen {
        let Some(asset) = project.assets.get(id) else {
            continue;
        };
        let c = source_color(asset);
        let file = std::path::Path::new(&asset.path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| asset.path.clone());
        let bits = c
            .bit_depth
            .map(|b| format!(", {b}-bit"))
            .unwrap_or_default();
        let handling = match (c.hdr, tag) {
            (Some(hdr), None) => {
                notes.push(format!(
                    "{file} is HDR ({}{bits}) and was converted to SDR BT.709 for this export; \
                     exports cannot keep HDR yet.",
                    hdr.label()
                ));
                HANDLING_TONE_MAPPED
            }
            (Some(hdr), Some(tag)) => {
                notes.push(format!(
                    "{file} is HDR ({}{bits}) but is tagged {} with edit.color_space, so it was \
                     not converted and its colours will look flat. Remove the tag to convert it.",
                    hdr.label(),
                    tag.as_str()
                ));
                HANDLING_TAGGED_NOT_CONVERTED
            }
            (None, None) if super::source_prep::is_bt601(&c) => {
                notes.push(format!(
                    "{file} uses BT.601 (SD video) colour and was converted to BT.709 for this export."
                ));
                HANDLING_BT601_CONVERTED
            }
            (None, _) => {
                if let Some(b) = c.bit_depth.filter(|b| *b > 8) {
                    notes.push(format!(
                        "{file} is {b}-bit; this export is processed in 8 bits."
                    ));
                }
                HANDLING_SDR
            }
        };
        sources.push(cut_core::SourceColorReport {
            asset: id.clone(),
            file,
            transfer: c.transfer.clone(),
            bit_depth: c.bit_depth,
            handling: handling.to_string(),
        });
    }
    let file_bits = file_bit_depth(video_args);
    if file_bits > 8 && !notes.is_empty() {
        notes.push(format!(
            "The picture is processed in 8 bits and stored in a {file_bits}-bit file."
        ));
    }
    let output = match project.settings.color.output {
        cut_core::ColorSpace::Rec709 => "SDR BT.709".to_string(),
        other => format!("SDR {}", other.as_str()),
    };
    cut_core::ColorReport {
        output,
        processing_bit_depth: 8,
        file_bit_depth: file_bits,
        sources,
        note: (!notes.is_empty()).then(|| notes.join(" ")),
    }
}

/// True when the render will run the zscale tone map (it needs an ffmpeg
/// built with zscale/libzimg).
pub fn needs_zscale(report: &cut_core::ColorReport) -> bool {
    report
        .sources
        .iter()
        .any(|s| s.handling == HANDLING_TONE_MAPPED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn asset(stream: Value) -> cut_core::Asset {
        cut_core::Asset {
            path: "/m/clip.mp4".into(),
            hash: "sha256:00".into(),
            probe: Some(json!({"kind": "video", "raw": {"streams": [stream]}})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        }
    }

    #[test]
    fn an_hlg_ten_bit_source_is_read_as_hdr() {
        let a = asset(json!({"codec_type": "video", "pix_fmt": "yuv420p10le",
            "color_transfer": "arib-std-b67", "color_primaries": "bt2020",
            "color_space": "bt2020nc", "color_range": "tv"}));
        let c = source_color(&a);
        assert_eq!(c.hdr, Some(Hdr::Hlg));
        assert_eq!(c.bit_depth, Some(10));
        let chain = super::super::source_prep::clip_source_prep(Some(&a), None);
        assert!(
            chain.starts_with(",zscale=tin=arib-std-b67:pin=bt2020:min=bt2020nc:rin=tv:t=linear"),
            "{chain}"
        );
        assert!(chain.contains("tonemap=tonemap=hable"), "{chain}");
        assert!(chain.ends_with("t=bt709:p=bt709:m=bt709:r=tv"), "{chain}");
    }

    /// A crafted probe value never reaches the filter graph: each hostile
    /// spelling falls back to the constant default.
    #[test]
    fn hostile_probe_metadata_never_reaches_the_filter_graph() {
        for hostile in [
            "bt709,movie=/etc/passwd",
            "bt2020:t=linear",
            "bt709[x];[x]null",
            "bt709;movie=/etc/passwd",
            "bt709'",
        ] {
            let a = asset(json!({"codec_type": "video", "pix_fmt": "yuv420p10le",
                "color_transfer": "arib-std-b67", "color_primaries": hostile,
                "color_space": hostile, "color_range": hostile}));
            let chain = super::super::source_prep::clip_source_prep(Some(&a), None);
            for bad in ["movie", "/etc", "[", ";", "'", "t=linear:t"] {
                assert!(!chain.contains(bad), "{hostile}: {bad} in {chain}");
            }
            assert!(
                chain.starts_with(
                    ",zscale=tin=arib-std-b67:pin=bt2020:min=bt2020nc:rin=tv:t=linear:npl=100,"
                ),
                "{hostile}: the defaults are used: {chain}"
            );
            // Only the fixed chain's own separators remain.
            assert_eq!(chain.matches(',').count(), 5, "{hostile}: {chain}");
        }
    }

    #[test]
    fn sdr_sources_keep_the_exact_old_chain() {
        let color = cut_core::ColorConfig::default();
        for stream in [
            json!({"codec_type": "video", "pix_fmt": "yuv420p", "color_transfer": "bt709"}),
            json!({"codec_type": "video", "pix_fmt": "yuv420p10le"}),
            json!({"codec_type": "video", "pix_fmt": "yuv420p10le", "color_transfer": "bt2020-10"}),
        ] {
            let a = asset(stream);
            assert_eq!(clip_color_chain(Some(&a), None, &color), "");
            assert_eq!(
                clip_color_chain(Some(&a), Some(&cut_core::ColorSpace::Rec2020), &color),
                super::super::colorspace_filter(Some(&cut_core::ColorSpace::Rec2020), &color)
            );
        }
        assert_eq!(clip_color_chain(None, None, &color), "");
    }

    #[test]
    fn a_person_tagged_hdr_clip_is_not_converted_but_never_keeps_hdr_tags() {
        let a = asset(json!({"codec_type": "video", "pix_fmt": "yuv420p10le",
            "color_transfer": "smpte2084", "color_primaries": "bt2020"}));
        assert_eq!(source_color(&a).hdr, Some(Hdr::Pq));
        let chain = clip_color_chain(
            Some(&a),
            Some(&cut_core::ColorSpace::Rec709),
            &cut_core::ColorConfig::default(),
        );
        assert_eq!(chain, RETAG_BT709);
    }
}

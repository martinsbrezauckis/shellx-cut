//! Keep HDR (10-bit BT.2020 HLG/PQ) graph planning, HDR10 static metadata
//! extraction/emission, and honest SDR-fallback classification for grades and effects.

use super::options::merge_codec_kv_param;
use super::source_color::{
    source_color, zscale_matrix_pub, zscale_primaries_pub, zscale_range_pub, Hdr, SourceColor,
};
use cut_core::{ClipEffect, ClipGrade, Edl, EdlSegment, Project, TrackKind};
use serde_json::Value;

/// Parsed SMPTE ST 2086 mastering display primaries, white point, and luminance
/// in normalized 0..=1 / nits coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct MasteringDisplay {
    pub rx: f64,
    pub ry: f64,
    pub gx: f64,
    pub gy: f64,
    pub bx: f64,
    pub by: f64,
    pub wx: f64,
    pub wy: f64,
    pub min_lum: f64,
    pub max_lum: f64,
}

impl MasteringDisplay {
    /// libx265 `-x265-params master-display=...` format (50,000ths & 10,000ths).
    pub fn x265_param(&self) -> String {
        let c = |v: f64| (v * 50_000.0).round().max(0.0) as u64;
        let l = |v: f64| (v * 10_000.0).round().max(0.0) as u64;
        format!(
            "G({},{})B({},{})R({},{})WP({},{})L({},{})",
            c(self.gx),
            c(self.gy),
            c(self.bx),
            c(self.by),
            c(self.rx),
            c(self.ry),
            c(self.wx),
            c(self.wy),
            l(self.max_lum),
            l(self.min_lum),
        )
    }

    /// SVT-AV1 `-svtav1-params mastering-display=...` decimal format.
    pub fn svtav1_param(&self) -> String {
        format!(
            "G({:.4},{:.4})B({:.4},{:.4})R({:.4},{:.4})WP({:.4},{:.4})L({:.4},{:.4})",
            self.gx,
            self.gy,
            self.bx,
            self.by,
            self.rx,
            self.ry,
            self.wx,
            self.wy,
            self.max_lum,
            self.min_lum
        )
    }
}

/// HDR10 static metadata (ST 2086 mastering display + ST 2094/CTA 861.3 MaxCLL/MaxFALL).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hdr10Metadata {
    pub mastering_display: Option<MasteringDisplay>,
    pub max_cll: Option<(u32, u32)>,
}

impl Hdr10Metadata {
    pub fn is_empty(&self) -> bool {
        self.mastering_display.is_none() && self.max_cll.is_none()
    }

    pub fn max_cll_str(&self) -> Option<String> {
        self.max_cll.map(|(mc, ma)| format!("{mc},{ma}"))
    }
}

fn parse_rational(v: &Value) -> Option<f64> {
    if let Some(n) = v.as_f64() {
        return n.is_finite().then_some(n);
    }
    let s = v.as_str()?.trim();
    if let Some((num, den)) = s.split_once('/') {
        let a: f64 = num.trim().parse().ok()?;
        let b: f64 = den.trim().parse().ok()?;
        if a.is_finite() && b.is_finite() && b > 0.0 {
            return Some(a / b);
        }
        return None;
    }
    let v: f64 = s.parse().ok()?;
    v.is_finite().then_some(v)
}

/// Extract mastering display & MaxCLL/MaxFALL from an asset's stored probe JSON.
pub fn hdr10_metadata_from_asset(asset: &cut_core::Asset) -> Hdr10Metadata {
    let Some(probe) = asset.probe.as_ref() else {
        return Hdr10Metadata::default();
    };
    let Some(streams) = probe["raw"]["streams"].as_array() else {
        return Hdr10Metadata::default();
    };
    let Some(video) = streams.iter().find(|s| {
        s["codec_type"] == "video" && s["disposition"]["attached_pic"].as_i64().unwrap_or(0) == 0
    }) else {
        return Hdr10Metadata::default();
    };
    let mut out = Hdr10Metadata::default();
    let Some(sd_list) = video["side_data_list"].as_array() else {
        return out;
    };
    for item in sd_list {
        match item["side_data_type"].as_str() {
            Some("Mastering display metadata") if out.mastering_display.is_none() => {
                if let (
                    Some(rx),
                    Some(ry),
                    Some(gx),
                    Some(gy),
                    Some(bx),
                    Some(by),
                    Some(wx),
                    Some(wy),
                    Some(min_lum),
                    Some(max_lum),
                ) = (
                    parse_rational(&item["red_x"]),
                    parse_rational(&item["red_y"]),
                    parse_rational(&item["green_x"]),
                    parse_rational(&item["green_y"]),
                    parse_rational(&item["blue_x"]),
                    parse_rational(&item["blue_y"]),
                    parse_rational(&item["white_point_x"]),
                    parse_rational(&item["white_point_y"]),
                    parse_rational(&item["min_luminance"]),
                    parse_rational(&item["max_luminance"]),
                ) {
                    out.mastering_display = Some(MasteringDisplay {
                        rx,
                        ry,
                        gx,
                        gy,
                        bx,
                        by,
                        wx,
                        wy,
                        min_lum,
                        max_lum,
                    });
                }
            }
            Some("Content light level metadata") if out.max_cll.is_none() => {
                if let (Some(mc), Some(ma)) =
                    (item["max_content"].as_u64(), item["max_average"].as_u64())
                {
                    if let (Ok(mc), Ok(ma)) = (u32::try_from(mc), u32::try_from(ma)) {
                        out.max_cll = Some((mc, ma));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Segment-level HDR plan resolved for a timeline under Keep HDR.
#[derive(Debug, Clone)]
pub struct HdrPlan {
    pub target: Hdr,
    pub metadata: Hdr10Metadata,
    pub present_in_sources: bool,
    pub metadata_carried: bool,
}

/// Active video track order matching the composition renderer (`planned_video_tracks`).
pub fn first_hdr_target(project: &Project, edl: &Edl) -> Option<Hdr> {
    timeline_hdr_clips(project, edl)
        .into_iter()
        .next()
        .map(|(_, hdr, _)| hdr)
}

fn timeline_hdr_clips<'a>(
    project: &'a Project,
    edl: &'a Edl,
) -> Vec<(u64, Hdr, &'a cut_core::Asset)> {
    let mut out = Vec::new();
    let mut visible_video_tracks = Vec::new();
    let mut seen_base = false;
    for t in project
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video && !t.clips.is_empty())
    {
        if !seen_base {
            seen_base = true;
            if t.visible {
                visible_video_tracks.push(t.id.as_str());
            }
        } else if t.visible {
            visible_video_tracks.push(t.id.as_str());
        }
    }
    let mut segs: Vec<&EdlSegment> = edl
        .segments
        .iter()
        .filter(|s| {
            s.track_kind == TrackKind::Video && visible_video_tracks.contains(&s.track.as_str())
        })
        .collect();
    segs.sort_by_key(|s| s.timeline_in_ms);
    for seg in segs {
        if seg.input_color_space.is_some() {
            continue;
        }
        let Some(asset_id) = seg.asset.as_deref() else {
            continue;
        };
        let Some(asset) = project.assets.get(asset_id) else {
            continue;
        };
        if let Some(hdr) = source_color(asset).hdr {
            out.push((seg.timeline_in_ms, hdr, asset));
        }
    }
    out
}

/// Analyze timeline HDR transfer target and HDR10 metadata availability.
/// Returns None when the timeline contains no active untagged HDR clips.
pub fn analyze_hdr_plan(project: &Project, edl: &Edl) -> Option<HdrPlan> {
    let hdr_clips = timeline_hdr_clips(project, edl);
    let target = hdr_clips.first().map(|(_, h, _)| *h)?;
    let mut any_md = Hdr10Metadata::default();
    let mut pq_md = Hdr10Metadata::default();
    for (_, hdr, asset) in &hdr_clips {
        let md = hdr10_metadata_from_asset(asset);
        if !md.is_empty() {
            if any_md.is_empty() {
                any_md = md.clone();
            }
            if *hdr == Hdr::Pq && pq_md.is_empty() {
                pq_md = md;
            }
        }
    }
    let present_in_sources = !any_md.is_empty();
    let (metadata, metadata_carried) = if target == Hdr::Pq && !pq_md.is_empty() {
        (pq_md, true)
    } else {
        (Hdr10Metadata::default(), false)
    };
    Some(HdrPlan {
        target,
        metadata,
        present_in_sources,
        metadata_carried,
    })
}

fn grade_is_sdr_only(g: &ClipGrade) -> bool {
    !g.is_identity()
}

/// Names of grades/effects on `seg` that only operate safely in 8-bit/SDR BT.709
/// (verified empirically on FFmpeg 8: `eq`, `vignette` drop `yuv420p10le` -> `yuv420p`;
/// `lutrgb` posterize assumes `256/l`; `normalize` clips HDR nits; `colorchannelmixer` sepia is sRGB).
pub fn clip_sdr_only_features(seg: &EdlSegment) -> Vec<&'static str> {
    let mut list = Vec::new();
    let push_uniq = |v: &mut Vec<&'static str>, label: &'static str| {
        if !v.contains(&label) {
            v.push(label);
        }
    };
    if seg.grade.as_ref().is_some_and(grade_is_sdr_only)
        || seg.grade_stack.iter().any(grade_is_sdr_only)
    {
        push_uniq(&mut list, "color grade");
    }
    if seg
        .grade_windows
        .iter()
        .any(|w| grade_is_sdr_only(&w.grade))
    {
        push_uniq(&mut list, "power-window grade");
    }
    for e in &seg.effects {
        match e {
            ClipEffect::Vignette { .. } => push_uniq(&mut list, "vignette"),
            ClipEffect::Sepia => push_uniq(&mut list, "sepia"),
            ClipEffect::AutoColor { .. } => push_uniq(&mut list, "auto-color"),
            ClipEffect::Posterize { .. } => push_uniq(&mut list, "posterize"),
            _ => {}
        }
    }
    list
}

/// Plain list of SDR-only grade/effect features on an adjustment layer.
pub fn adjustment_sdr_only_features(
    grade: Option<&ClipGrade>,
    effects: &[ClipEffect],
) -> Vec<&'static str> {
    let mut list = Vec::new();
    let push_uniq = |v: &mut Vec<&'static str>, label: &'static str| {
        if !v.contains(&label) {
            v.push(label);
        }
    };
    if grade.is_some_and(grade_is_sdr_only) {
        push_uniq(&mut list, "color grade");
    }
    for e in effects {
        match e {
            ClipEffect::Vignette { .. } => push_uniq(&mut list, "vignette"),
            ClipEffect::Sepia => push_uniq(&mut list, "sepia"),
            ClipEffect::AutoColor { .. } => push_uniq(&mut list, "auto-color"),
            ClipEffect::Posterize { .. } => push_uniq(&mut list, "posterize"),
            _ => {}
        }
    }
    list
}

/// Convert/normalize an HDR source (`c`) to `target` BT.2020 limited range.
/// When transfer/primaries/matrix/range already match, emits `setparams` instead
/// of a float zscale roundtrip so source 10-bit code values stay untouched.
pub fn hdr_to_hdr_filter(c: &SourceColor, target: Hdr) -> String {
    let Some(src_hdr) = c.hdr else {
        return String::new();
    };
    let dst_t = target.zscale_transfer();
    let same_t = src_hdr == target;
    let same_p = c.primaries.as_deref() == Some("bt2020");
    let same_m = c.matrix.as_deref() == Some("bt2020nc");
    let same_r = !matches!(c.range.as_deref(), Some("pc"));
    if same_t && same_p && same_m && same_r {
        return format!(
            ",setparams=color_primaries=bt2020:color_trc={dst_t}:colorspace=bt2020nc:range=tv"
        );
    }
    let pin = zscale_primaries_pub(c.primaries.as_deref());
    let min = zscale_matrix_pub(c.matrix.as_deref());
    let rin = zscale_range_pub(c.range.as_deref());
    format!(
        ",zscale=tin={ti}:pin={pin}:min={min}:rin={rin}:t={dst_t}:p=bt2020:m=bt2020nc:r=tv",
        ti = src_hdr.zscale_transfer()
    )
}

/// Place SDR BT.709 picture into BT.2020 `target` HDR at 203 nits (ITU-R BT.2408).
pub fn sdr_to_hdr_filter(target: Hdr, range_in: Option<&str>, preserve_alpha: bool) -> String {
    let rin = zscale_range_pub(range_in);
    let dst_t = target.zscale_transfer();
    let pre = if !preserve_alpha && rin == "pc" {
        ",format=yuv420p"
    } else {
        ""
    };
    let effective_rin = if !preserve_alpha && rin == "pc" {
        "tv"
    } else {
        rin
    };
    format!(
        "{pre},zscale=tin=bt709:pin=bt709:min=bt709:rin={effective_rin}:t={dst_t}:p=bt2020:m=bt2020nc:r=tv:npl=203"
    )
}

/// Generated solid-black stream color tag filter for 10-bit BT.2020 HDR graphs.
pub fn black_10bit_suffix(target: Hdr) -> String {
    format!(
        "format=yuv420p10le,setparams=color_primaries=bt2020:color_trc={}:colorspace=bt2020nc:range=tv",
        target.zscale_transfer()
    )
}

/// Active video pixel/overlay token configuration so SDR renders stay character-identical
/// while Keep HDR renders run 10-bit end-to-end with zero 8-bit pins.
#[derive(Debug, Clone, Copy)]
pub(super) struct GraphColor {
    pub pix: &'static str,
    pub alpha: &'static str,
    pub ov: &'static str,
    pub hdr: Option<Hdr>,
}

impl GraphColor {
    pub const fn sdr() -> Self {
        Self {
            pix: "yuv420p",
            alpha: "yuva420p",
            ov: "",
            hdr: None,
        }
    }

    pub fn resolve_with(
        project: &Project,
        edl: &Edl,
        mode: super::options::RenderColorMode,
        target_override: Option<Hdr>,
    ) -> Self {
        let hdr = (mode == super::options::RenderColorMode::KeepHdr)
            .then(|| target_override.or_else(|| first_hdr_target(project, edl)))
            .flatten();
        if hdr.is_some() {
            Self {
                pix: "yuv420p10le",
                alpha: "yuva420p10le",
                ov: ":format=yuv420p10",
                hdr,
            }
        } else {
            Self::sdr()
        }
    }

    pub fn keep_hdr(self) -> bool {
        self.hdr.is_some()
    }

    pub fn black_stage(self) -> String {
        match self.hdr {
            Some(t) => black_10bit_suffix(t),
            None => "format=yuv420p".into(),
        }
    }
}

/// Overlay static opacity multiplier (`colorchannelmixer` on SDR for byte-identity;
/// native-plane `lut=a=` on 10-bit `yuva420p10le` to avoid swscale BT.601 color shift).
pub(super) fn overlay_static_opacity(
    t: &cut_core::ClipTransform,
    vopac: &str,
    keep_hdr: bool,
) -> String {
    if t.opacity < 1.0 && vopac.is_empty() {
        let o = (t.opacity.clamp(0.0, 1.0) * 1000.0).round() / 1000.0;
        if keep_hdr {
            format!(",lut=a='val*{o}'")
        } else {
            format!(",colorchannelmixer=aa={o}")
        }
    } else {
        String::new()
    }
}

/// Mask-to-black replacement filter (`lutyuv` preserves 10-bit; `drawbox` drops to 8-bit).
pub(super) fn mask_black_filter(keep_hdr: bool) -> &'static str {
    if keep_hdr {
        "lutyuv=y=64:u=512:v=512"
    } else {
        "drawbox=x=0:y=0:w=iw:h=ih:color=black@1:t=fill"
    }
}

/// Wrap `filter_chain` with HDR->SDR ... SDR->HDR when `sdr_only` is true on an HDR timeline.
pub(super) fn wrap_sdr_filter_if(
    target: Option<Hdr>,
    filter_chain: &str,
    sdr_only: bool,
) -> String {
    match target {
        Some(t) if sdr_only && !filter_chain.is_empty() => format!(
            "{}{}{}",
            hdr_to_sdr_from_target(t),
            filter_chain,
            sdr_to_hdr_filter(t, Some("tv"), false)
        ),
        _ => filter_chain.to_string(),
    }
}

/// Gate render paths cleanly by mode: SDR keeps tone-map check; Keep HDR verifies
/// timeline HDR source, `zscale`, and 10-bit encoder capability up front.
pub fn require_support_for_mode(
    project: &Project,
    edl: &Edl,
    mode: super::options::RenderColorMode,
    encoder: &str,
) -> Result<(), cut_core::CutError> {
    if mode == super::options::RenderColorMode::Sdr {
        return super::tone_map_support::require_tone_map_support(project, edl);
    }
    require_keep_hdr_support_with(
        project,
        edl,
        encoder,
        crate::hwencode::zscale_available,
        crate::hwencode::encoder_supports_10bit,
    )
}

/// Pure, testable preflight check for Keep HDR renders.
pub fn require_keep_hdr_support_with(
    project: &Project,
    edl: &Edl,
    encoder: &str,
    zscale_ok: impl FnOnce() -> bool,
    enc_10bit_ok: impl FnOnce(&str) -> bool,
) -> Result<(), cut_core::CutError> {
    if analyze_hdr_plan(project, edl).is_none() {
        return Err(cut_core::CutError::new(
            cut_core::error_codes::INVALID_ARGS,
            "Keep HDR export needs at least one HDR (HLG or PQ) video clip on the timeline",
            "All active video clips on this timeline are SDR (or tagged Rec.709 with edit.color_space), so there is no HDR picture to preserve.",
        )
        .with_suggested_action("Export with color: \"sdr\" (the default), or add an untagged HLG or PQ video clip."));
    }
    if !zscale_ok() {
        let mut error =
            crate::tool_setup::missing_feature_error(crate::toolpath::FfmpegFeature::Zscale);
        error.message = "Keep HDR export needs FFmpeg with zimg (the zscale filter), and the installed FFmpeg has none".into();
        return Err(error);
    }
    if !encoder.is_empty() && !enc_10bit_ok(encoder) {
        return Err(cut_core::CutError::new(
            cut_core::error_codes::TOOL_MISSING,
            format!(
                "Keep HDR export needs a 10-bit {encoder} encoder, and the installed FFmpeg does not support 10-bit pixel formats for {encoder}"
            ),
            format!("10-bit BT.2020 HDR cannot be encoded by {encoder} on this FFmpeg build."),
        )
        .with_suggested_action(
            "Install an FFmpeg build with 10-bit libx265 (Main 10) or libsvtav1 support (see Settings -> Environment), or export in SDR.",
        ));
    }
    Ok(())
}

/// HDR->SDR tone-map filter starting from the known target HDR space (used inside
/// power windows and adjustment layers when Keep HDR wraps an SDR-only grade/effect).
pub(super) fn hdr_to_sdr_from_target(target: Hdr) -> String {
    super::source_color::hdr_to_sdr_filter(&SourceColor {
        transfer: Some(target.zscale_transfer().into()),
        primaries: Some("bt2020".into()),
        matrix: Some("bt2020nc".into()),
        range: Some("tv".into()),
        bit_depth: Some(10),
        hdr: Some(target),
    })
}

/// Output stream color tags + encoder SEI/VUI params for Keep HDR.
pub fn apply_hdr_encode_args(
    args: &mut Vec<String>,
    target: Hdr,
    metadata: Option<&Hdr10Metadata>,
) {
    let trc = target.zscale_transfer();
    args.extend([
        "-colorspace".into(),
        "bt2020nc".into(),
        "-color_primaries".into(),
        "bt2020".into(),
        "-color_trc".into(),
        trc.into(),
        "-color_range".into(),
        "tv".into(),
    ]);
    let enc = args
        .windows(2)
        .find(|w| w[0] == "-c:v")
        .map(|w| w[1].clone())
        .unwrap_or_default();
    if enc == "libx265" {
        let mut params =
            format!("colorprim=bt2020:transfer={trc}:colormatrix=bt2020nc:repeat-headers=1");
        if target == Hdr::Pq {
            params.push_str(":hdr-opt=1");
            if let Some(md) = metadata {
                if let Some(disp) = &md.mastering_display {
                    params.push_str(&format!(":master-display={}", disp.x265_param()));
                }
                if let Some(cll) = md.max_cll_str() {
                    params.push_str(&format!(":max-cll={cll}"));
                }
            }
        }
        merge_codec_kv_param(args, "-x265-params", &params);
    } else if enc == "libsvtav1" {
        let tc = match target {
            Hdr::Pq => "16",
            Hdr::Hlg => "18",
        };
        let mut params =
            format!("color-primaries=9:transfer-characteristics={tc}:matrix-coefficients=9");
        if target == Hdr::Pq {
            if let Some(md) = metadata {
                if let Some(disp) = &md.mastering_display {
                    params.push_str(&format!(":mastering-display={}", disp.svtav1_param()));
                }
                if let Some(cll) = md.max_cll_str() {
                    params.push_str(&format!(":content-light={cll}"));
                }
            }
        }
        merge_codec_kv_param(args, "-svtav1-params", &params);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_ffprobe_hdr10_side_data_for_x265_and_svtav1() {
        let asset: cut_core::Asset = serde_json::from_value(json!({
            "path": "/m/pq.mp4",
            "hash": "sha256:00",
            "probe": {
                "kind": "video",
                "raw": {
                    "streams": [{
                        "codec_type": "video",
                        "pix_fmt": "yuv420p10le",
                        "color_transfer": "smpte2084",
                        "color_primaries": "bt2020",
                        "color_space": "bt2020nc",
                        "side_data_list": [
                            {
                                "side_data_type": "Mastering display metadata",
                                "red_x": "34000/50000", "red_y": "16000/50000",
                                "green_x": "13250/50000", "green_y": "34500/50000",
                                "blue_x": "7500/50000", "blue_y": "3000/50000",
                                "white_point_x": "15635/50000", "white_point_y": "16450/50000",
                                "min_luminance": "1/10000", "max_luminance": "10000000/10000"
                            },
                            {
                                "side_data_type": "Content light level metadata",
                                "max_content": 1000, "max_average": 400
                            }
                        ]
                    }]
                }
            }
        }))
        .unwrap();
        let md = hdr10_metadata_from_asset(&asset);
        assert_eq!(
            md.mastering_display.as_ref().unwrap().x265_param(),
            "G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1)"
        );
        assert_eq!(
            md.mastering_display.as_ref().unwrap().svtav1_param(),
            "G(0.2650,0.6900)B(0.1500,0.0600)R(0.6800,0.3200)WP(0.3127,0.3290)L(1000.0000,0.0001)"
        );
        assert_eq!(md.max_cll_str().as_deref(), Some("1000,400"));
    }

    fn hlg_project() -> Project {
        serde_json::from_value(json!({
            "name": "p",
            "schema": "shellx-cut/1",
            "settings": {"width": 320, "height": 180, "fps": 30.0, "audio_rate": 48000},
            "assets": {"a1": {"path": "/m/hlg.mp4", "hash": "sha256:00",
                "probe": {"kind": "video", "width": 320, "height": 180, "duration_ms": 1000,
                          "has_audio": false, "raw": {"streams": [{"codec_type": "video",
                          "pix_fmt": "yuv420p10le", "color_transfer": "arib-std-b67",
                          "color_primaries": "bt2020"}]}}}},
            "tracks": [{"id": "v1", "kind": "video", "clips": [
                {"id": "c1", "asset": "a1", "src_in_ms": 0, "src_out_ms": 1000}
            ]}]
        }))
        .expect("project")
    }

    #[test]
    fn keep_hdr_without_zscale_is_refused_with_the_way_forward() {
        let p = hlg_project();
        let edl = cut_core::edl_from_project(&p);
        let e = require_keep_hdr_support_with(&p, &edl, "libx265", || false, |_| true).unwrap_err();
        assert_eq!(e.code, cut_core::error_codes::TOOL_MISSING);
        assert!(
            e.message
                .starts_with("Keep HDR export needs FFmpeg with zimg"),
            "{}",
            e.message
        );
        assert!(e.suggested_action.is_some(), "{e:?}");
    }

    #[test]
    fn keep_hdr_without_a_10bit_encoder_is_refused_with_the_way_forward() {
        let p = hlg_project();
        let edl = cut_core::edl_from_project(&p);
        let e = require_keep_hdr_support_with(&p, &edl, "libx265", || true, |_| false).unwrap_err();
        assert_eq!(e.code, cut_core::error_codes::TOOL_MISSING);
        assert!(e.message.contains("10-bit libx265"), "{}", e.message);
        assert!(
            e.suggested_action
                .as_deref()
                .unwrap_or("")
                .contains("export in SDR"),
            "{e:?}"
        );
        assert!(require_keep_hdr_support_with(&p, &edl, "libx265", || true, |_| true).is_ok());
    }

    #[test]
    fn merges_hdr_params_without_duplicating_x265_params_flag() {
        let mut args: Vec<String> = vec![
            "-c:v".into(),
            "libx265".into(),
            "-x265-params".into(),
            "strict-cbr=1".into(),
        ];
        apply_hdr_encode_args(&mut args, Hdr::Pq, None);
        let x265_flags: Vec<_> = args
            .iter()
            .enumerate()
            .filter(|(_, a)| *a == "-x265-params")
            .collect();
        assert_eq!(x265_flags.len(), 1);
        assert!(args[x265_flags[0].0 + 1].contains("strict-cbr=1"));
        assert!(args[x265_flags[0].0 + 1].contains("transfer=smpte2084"));
    }
}

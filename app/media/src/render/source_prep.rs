//! Bring a source's picture into the render's colour space BEFORE anything
//! edits it.
//!
//! The per-clip chain is `trim … conform, PREP, animate, grade, effects,
//! colour-management hops`. PREP is where a source that is not plain BT.709
//! becomes BT.709, so that a grade, an effect or a key works on the same kind
//! of picture for every clip:
//! - an HDR source (HLG/PQ) is tone-mapped to SDR BT.709 here. It used to be
//!   tone-mapped after the grade, so a grade worked on HLG code values and its
//!   result was then bent by the tone map;
//! - a BT.601 source (SD video tagged `smpte170m` or `bt470bg`) has its YUV
//!   matrix converted to BT.709 with ffmpeg's built-in `colorspace` filter. It
//!   used to pass its BT.601 values through unchanged, so next to BT.709 clips
//!   (or under BT.709 tags taken from another clip) its colours shifted.
//!
//! PREP sits after the conform so the float tone map runs at the output size,
//! not at an 8K source size. A clip the person tagged with `edit.color_space`
//! gets no PREP: their tag decides. Untagged BT.709 and unknown sources get
//! nothing, so ordinary renders keep their bytes.
//!
//! SECURITY: every token comes from an allow-list or a constant; no probe
//! string reaches the filter graph.

use super::source_color::{hdr_to_sdr_filter, source_color, SourceColor};

/// The colorspace filter's name for a BT.601 source's matrix, from its tag.
fn bt601_standard(c: &SourceColor) -> Option<&'static str> {
    if c.hdr.is_some() {
        return None;
    }
    match c.matrix.as_deref() {
        // NTSC-family SD (and most "BT.601" tagged files) and PAL-family SD
        // share one luma/chroma matrix; the filter names it by family.
        Some("smpte170m") => Some("smpte170m"),
        Some("bt470bg") => Some("bt470bg"),
        _ => None,
    }
}

/// The source's primaries for the conversion: SMPTE-C or EBU only when the
/// file says so; an untagged SD file is taken to have BT.709 primaries, so
/// only its matrix changes (no guessed gamut shift).
fn bt601_primaries(c: &SourceColor) -> &'static str {
    match c.primaries.as_deref() {
        Some("smpte170m") => "smpte170m",
        Some("bt470bg") => "bt470bg",
        _ => "bt709",
    }
}

/// True when `c` is a BT.601 SDR source that PREP converts.
pub fn is_bt601(c: &SourceColor) -> bool {
    bt601_standard(c).is_some()
}

/// BT.601 -> BT.709 matrix/primaries/transfer conversion (leading comma), or
/// "" when the source is not a tagged BT.601 SDR source.
pub fn bt601_to_bt709_filter(c: &SourceColor) -> String {
    let Some(standard) = bt601_standard(c) else {
        return String::new();
    };
    let irange = if c.range.as_deref() == Some("pc") {
        "pc"
    } else {
        "tv"
    };
    // BT.601 and BT.709 share the transfer curve, so only the matrix (and,
    // when tagged, the primaries) change.
    let iprimaries = bt601_primaries(c);
    format!(
        ",colorspace=ispace={standard}:iprimaries={iprimaries}:itrc=bt709:irange={irange}:\
         space=bt709:primaries=bt709:trc=bt709:range=tv"
    )
}

/// The PREP filter for one clip (leading comma), or "".
pub(super) fn clip_source_prep(
    asset: Option<&cut_core::Asset>,
    input: Option<&cut_core::ColorSpace>,
) -> String {
    clip_source_prep_for(asset, input, None, false)
}

/// Color-mode-aware PREP stage:
/// - `hdr_target = None`: SDR default (`clip_source_prep`), byte-identical.
/// - `hdr_target = Some(target)`: Keep HDR.
///   * HDR source with SDR-only grade/effect: tone-map to SDR BT.709 here so its
///     grade/effect runs honestly in SDR before `clip_color_chain_for` converts back at 203 nits.
///   * HDR source without SDR-only grade/effect: normalize/convert directly in HDR (`hdr_to_hdr_filter`).
///   * SDR BT.601 source: normalize to BT.709 here before grade/effects, then `clip_color_chain_for` places in HDR at 203 nits.
pub(super) fn clip_source_prep_for(
    asset: Option<&cut_core::Asset>,
    input: Option<&cut_core::ColorSpace>,
    hdr_target: Option<super::source_color::Hdr>,
    sdr_fallback_needed: bool,
) -> String {
    if input.is_some() {
        return String::new();
    }
    let Some(c) = asset.map(source_color) else {
        return String::new();
    };
    match hdr_target {
        None => {
            if c.hdr.is_some() {
                hdr_to_sdr_filter(&c)
            } else {
                bt601_to_bt709_filter(&c)
            }
        }
        Some(target) => {
            if c.hdr.is_some() {
                if sdr_fallback_needed {
                    hdr_to_sdr_filter(&c)
                } else {
                    super::hdr::hdr_to_hdr_filter(&c, target)
                }
            } else {
                bt601_to_bt709_filter(&c)
            }
        }
    }
}

/// True when PREP converts this clip (a CPU filter the CUDA path lacks).
pub(super) fn needs_source_prep(asset: Option<&cut_core::Asset>) -> bool {
    !clip_source_prep(asset, None).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

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
    fn a_bt601_source_is_converted_to_bt709_before_anything_edits_it() {
        let ntsc = asset(json!({"codec_type": "video", "pix_fmt": "yuv420p",
            "color_space": "smpte170m", "color_range": "tv"}));
        assert_eq!(
            clip_source_prep(Some(&ntsc), None),
            ",colorspace=ispace=smpte170m:iprimaries=bt709:itrc=bt709:irange=tv:space=bt709:primaries=bt709:trc=bt709:range=tv"
        );
        let pal = asset(json!({"codec_type": "video", "pix_fmt": "yuv420p",
            "color_space": "bt470bg", "color_range": "pc"}));
        assert_eq!(
            clip_source_prep(Some(&pal), None),
            ",colorspace=ispace=bt470bg:iprimaries=bt709:itrc=bt709:irange=pc:space=bt709:primaries=bt709:trc=bt709:range=tv"
        );
        // The person's own tag decides.
        assert_eq!(
            clip_source_prep(Some(&ntsc), Some(&cut_core::ColorSpace::Rec709)),
            ""
        );
    }

    #[test]
    fn an_hdr_source_is_tone_mapped_in_the_prep_slot() {
        let hlg = asset(json!({"codec_type": "video", "pix_fmt": "yuv420p10le",
            "color_transfer": "arib-std-b67", "color_primaries": "bt2020",
            "color_space": "bt2020nc"}));
        assert!(clip_source_prep(Some(&hlg), None).starts_with(",zscale=tin=arib-std-b67"));
    }

    /// A one-clip project whose clip carries a brightness grade.
    fn graded_project(media: &std::path::Path, stream: Value) -> cut_core::Project {
        let mut p = cut_core::Project::new(
            "prep",
            cut_core::ProjectSettings {
                width: 320,
                height: 180,
                fps: 30.0,
                audio_rate: 48_000,
                color: cut_core::ColorConfig::default(),
            },
        );
        let mut a = asset(stream);
        a.path = media.to_string_lossy().into_owned();
        a.probe = Some(json!({"kind": "video", "width": 320, "height": 180,
            "duration_ms": 1000, "fps": 30.0, "has_audio": false,
            "raw": a.probe.as_ref().unwrap()["raw"].clone()}));
        p.assets.insert("a1".into(), a);
        let grade: cut_core::ClipGrade =
            serde_json::from_value(json!({"brightness": 0.1})).unwrap();
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(cut_core::Clip::Media(cut_core::MediaClip {
                id: "c1".into(),
                asset: "a1".into(),
                src_in_ms: 0,
                src_out_ms: 1000,
                effects: vec![],
                gain_db: 0.0,
                transform: None,
                crop: None,
                fade: None,
                xfade_in_ms: 0,
                xfade_kind: None,
                speed: 1.0,
                grade: Some(grade),
                matte: None,
                mask: None,
                reverse: false,
                freeze: None,
                animation: None,
                keyframes: vec![],
                eq: None,
                mute_ranges: vec![],
                stabilize: None,
                speed_ramp: None,
                input_color_space: None,
                nest: None,
                grade_stack: vec![],
                grade_windows: vec![],
                markers: vec![],
            }));
        p
    }

    /// The grade must see the converted (BT.709 SDR) picture: in the graph,
    /// the tone map / matrix conversion comes before `eq`.
    #[test]
    fn the_prep_runs_before_the_grade_in_the_render_graph() {
        let tmp = tempfile::tempdir().unwrap();
        let media = tmp.path().join("clip.mp4");
        super::super::tests::graph_fixture(&media);
        for (stream, marker) in [
            (
                json!({"codec_type": "video", "pix_fmt": "yuv420p10le",
                    "color_transfer": "arib-std-b67", "color_primaries": "bt2020",
                    "color_space": "bt2020nc"}),
                "zscale=tin=arib-std-b67",
            ),
            (
                json!({"codec_type": "video", "pix_fmt": "yuv420p", "color_space": "smpte170m"}),
                "colorspace=ispace=smpte170m",
            ),
        ] {
            let p = graded_project(&media, stream);
            let edl = cut_core::edl_from_project(&p);
            let g = super::super::build_graph(
                &p,
                &edl,
                tmp.path(),
                true,
                false,
                true,
                super::super::RenderOptions::default(),
                None,
            )
            .unwrap();
            let prep = g
                .filter
                .find(marker)
                .unwrap_or_else(|| panic!("{marker}:\n{}", g.filter));
            let grade = g
                .filter
                .find("eq=")
                .unwrap_or_else(|| panic!("no grade:\n{}", g.filter));
            assert!(
                prep < grade,
                "{marker} must come before the grade:\n{}",
                g.filter
            );
        }
    }

    #[test]
    fn bt709_unknown_and_hostile_sources_get_no_prep() {
        for stream in [
            json!({"codec_type": "video", "pix_fmt": "yuv420p", "color_space": "bt709"}),
            json!({"codec_type": "video", "pix_fmt": "yuv420p"}),
            json!({"codec_type": "video", "pix_fmt": "yuv420p",
                   "color_space": "smpte170m,movie=/etc/passwd"}),
        ] {
            assert_eq!(clip_source_prep(Some(&asset(stream)), None), "");
        }
        assert_eq!(clip_source_prep(None, None), "");
    }
}

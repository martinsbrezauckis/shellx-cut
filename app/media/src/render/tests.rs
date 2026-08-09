use super::input_paths::strip_verbatim_prefix;
use super::*;
use cut_core::{ColorConfig, ColorSpace, EdlSegment};

#[test]
fn sha256_file_streams_expected_digest() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("render.bin");
    std::fs::write(&path, b"shellx-cut").unwrap();

    assert_eq!(
        sha256_file(&path).unwrap(),
        "sha256:5c21077b34928c219a44ead14425bb32a75fd45ca4edc3b916e85bf56196122e"
    );
}

/// The byte-identical guarantee: the DEFAULT color config (rec709/rec709) with an
/// UNTAGGED clip emits NO color filter, so the per-clip chain is byte-identical to
/// a pre-color-management render. Also true for a clip whose input is tagged
/// rec709 == working (a redundant tag is a no-op).
#[test]
fn colorspace_filter_default_is_empty() {
    let def = ColorConfig::default();
    assert_eq!(colorspace_filter(None, &def), "");
    assert_eq!(colorspace_filter(Some(&ColorSpace::Rec709), &def), "");
}

/// working→output (project.color{working:rec709, output:rec2020}) on an UNTAGGED
/// clip emits exactly ONE zscale hop converting rec709→rec2020 (the case (b) of
/// the comparison test). The hop states the input explicitly and targets the rec2020
/// tokens (so libx264 tags the stream bt2020nc/bt2020/bt2020-10).
#[test]
fn colorspace_filter_working_to_output_one_hop() {
    let c = ColorConfig {
        working: ColorSpace::Rec709,
        output: ColorSpace::Rec2020,
    };
    let f = colorspace_filter(None, &c);
    assert_eq!(
        f,
        ",zscale=tin=bt709:pin=bt709:min=bt709:t=bt2020-10:p=bt2020:m=bt2020nc"
    );
}

/// A clip tagged srgb under the DEFAULT rec709 working+output (case (c)) emits ONE
/// hop srgb→rec709 (input→working; working==output so no second hop).
#[test]
fn colorspace_filter_input_tag_one_hop() {
    let f = colorspace_filter(Some(&ColorSpace::Srgb), &ColorConfig::default());
    assert_eq!(
        f,
        ",zscale=tin=iec61966-2-1:pin=bt709:min=bt709:t=bt709:p=bt709:m=bt709"
    );
}

/// A tagged clip AND a working≠output config emits TWO hops: input→working then
/// working→output (the literal input→working→output the design promises).
#[test]
fn colorspace_filter_two_hops() {
    let c = ColorConfig {
        working: ColorSpace::Linear,
        output: ColorSpace::Rec2020,
    };
    let f = colorspace_filter(Some(&ColorSpace::Srgb), &c);
    // hop1 srgb→linear, hop2 linear→rec2020.
    assert_eq!(
        f,
        ",zscale=tin=iec61966-2-1:pin=bt709:min=bt709:t=linear:p=bt709:m=bt709\
         ,zscale=tin=linear:pin=bt709:min=bt709:t=bt2020-10:p=bt2020:m=bt2020nc"
    );
}

/// Output tagging flags: rec709 output (default) emits NONE (byte-identical);
/// rec2020 output emits the explicit -colorspace/-color_primaries/-color_trc set.
#[test]
fn output_color_args_default_empty_rec2020_tagged() {
    assert!(output_color_args(&ColorConfig::default()).is_empty());
    let c = ColorConfig {
        working: ColorSpace::Rec709,
        output: ColorSpace::Rec2020,
    };
    assert_eq!(
        output_color_args(&c),
        vec![
            "-colorspace",
            "bt2020nc",
            "-color_primaries",
            "bt2020",
            "-color_trc",
            "bt2020-10"
        ]
    );
}

/// Windows vidstab path bug: the stabilize `.trf` path reached
/// ffmpeg with a `\\?\` verbatim prefix + drive colon and the filtergraph parser
/// shredded it (`No option name near …`), corrupting every later render of a
/// stabilized clip. Fix = strip the verbatim prefix + route through the SAME
/// proven escaper as ASS subtitle burn-in (escape_filter_path: forward-slash +
/// single-quote + escaped colon). NOTE: this asserts the STRING form only — the
/// authoritative proof is a Windows render, so this string test is
/// necessary-but-not-sufficient.
#[test]
fn vidstab_trf_path_is_filtergraph_safe_on_windows() {
    // \\?\ verbatim prefix stripped; \\?\UNC\ → \\; POSIX untouched.
    assert_eq!(
        strip_verbatim_prefix(Path::new(r"\\?\C:\Users\U\p.cutproj\stab\h.trf")),
        Path::new(r"C:\Users\U\p.cutproj\stab\h.trf"),
    );
    assert_eq!(
        strip_verbatim_prefix(Path::new(r"\\?\UNC\srv\share\h.trf")),
        Path::new(r"\\srv\share\h.trf"),
    );
    assert_eq!(
        strip_verbatim_prefix(Path::new("/home/u/p.cutproj/stab/h.trf")),
        Path::new("/home/u/p.cutproj/stab/h.trf"),
    );
    // The exact string ffmpeg receives: single-quoted, forward slashes, escaped
    // colon, NO raw backslash, NO verbatim prefix.
    let esc = escape_filter_path(&strip_verbatim_prefix(Path::new(
        r"\\?\C:\Users\U\stab\h.trf",
    )));
    assert_eq!(esc, r"'C\:/Users/U/stab/h.trf'");
    assert!(!esc.contains(r"\\") && !esc.contains(r"?"));
}

#[test]
fn vidstab_detect_uses_ascii_when_the_selected_filter_supports_it() {
    let trf = Path::new("/tmp/shellx cut/stab.trf");
    let modern = stab_detect_filter(250, 1750, trf, true);
    assert!(
        modern.contains("vidstabdetect=shakiness=8:accuracy=15:fileformat=ascii:result="),
        "newer or mixed ffmpeg builds must write a portable text transform: {modern}"
    );
    assert!(modern.contains("trim=start=0.250:end=1.750"), "{modern}");

    let legacy = stab_detect_filter(250, 1750, trf, false);
    assert!(
        legacy.contains("vidstabdetect=shakiness=8:accuracy=15:result="),
        "legacy ffmpeg keeps its text default without an unsupported option: {legacy}"
    );
    assert!(!legacy.contains("fileformat"), "{legacy}");

    let modern_transform = stab_transform_filter(trf, 15, true);
    assert!(
        modern_transform.contains(
            "vidstabtransform=input='/tmp/shellx cut/stab.trf':smoothing=15:crop=black:fileformat=ascii"
        ),
        "a modern transform reader must explicitly match the detector: {modern_transform}"
    );
    let legacy_transform = stab_transform_filter(trf, 15, false);
    assert!(
        !legacy_transform.contains("fileformat"),
        "{legacy_transform}"
    );
}

/// Transitions (edit.crossfade `transition`): fold_video emits the chosen
/// ffmpeg `xfade` style at the seam, and an unset/None style stays the classic
/// `transition=fade` dissolve (byte-identical to the pre-transitions graph).
#[test]
fn fold_video_emits_chosen_transition() {
    let styled = vec![
        SegStream {
            label: "s0".into(),
            dur_ms: 2000,
            xfade_in_ms: 0,
            xfade_kind: None,
        },
        SegStream {
            label: "s1".into(),
            dur_ms: 2000,
            xfade_in_ms: 400,
            xfade_kind: Some("wipeleft".into()),
        },
    ];
    let mut f = String::new();
    let total = fold_video(&mut f, &styled, "vout", 30.0);
    assert!(
        f.contains("xfade=transition=wipeleft:duration=0.400"),
        "fold_video must emit the chosen transition; got:\n{f}"
    );
    assert_eq!(total, 3600, "realized length = 2000 + 2000 - 400 overlap");

    // Unset style → classic dissolve (the default + pre-transitions behavior).
    let plain = vec![
        SegStream {
            label: "s0".into(),
            dur_ms: 2000,
            xfade_in_ms: 0,
            xfade_kind: None,
        },
        SegStream {
            label: "s1".into(),
            dur_ms: 2000,
            xfade_in_ms: 400,
            xfade_kind: None,
        },
    ];
    let mut f2 = String::new();
    fold_video(&mut f2, &plain, "vout", 30.0);
    assert!(
        f2.contains("xfade=transition=fade:"),
        "an unset transition stays the classic dissolve; got:\n{f2}"
    );
}

/// Effects (edit.effect): effect_filter emits each effect's ffmpeg filter in
/// order, and chroma key is emitted ONLY on an overlay clip (it reveals a
/// lower track; a base clip has nothing under it).
#[test]
fn effect_filter_emits_per_effect_and_gates_chroma() {
    use cut_core::ClipEffect as E;
    let fx = vec![
        E::Vignette { amount: 0.5 },
        E::Sharpen { amount: 1.0 },
        E::Blur { radius: 4.0 },
        E::Grain { amount: 20.0 },
    ];
    let base = effect_filter(&fx, false);
    assert!(base.contains(",vignette=angle="), "{base}");
    assert!(base.contains(",unsharp=5:5:1"), "{base}");
    assert!(base.contains(",gblur=sigma=4"), "{base}");
    assert!(base.contains(",noise=alls=20:allf=t+u"), "{base}");
    // effects preserve order (vignette before grain in the string).
    assert!(base.find("vignette").unwrap() < base.find("noise").unwrap());

    let chroma = vec![E::ChromaKey {
        color: "green".into(),
        similarity: 0.15,
        blend: 0.1,
    }];
    assert!(
        effect_filter(&chroma, true).contains(",chromakey=color=green:similarity="),
        "chroma key emits on an overlay"
    );
    assert_eq!(
        effect_filter(&chroma, false),
        "",
        "chroma key is skipped on a base clip"
    );

    // SECURITY: a filtergraph-injection payload in `color` is SKIPPED at
    // point-of-use (not interpolated), so it can never reach the graph even
    // if it bypassed the verb-boundary check (e.g. loaded from project.json).
    let evil = vec![E::ChromaKey {
        color: "green,movie=/etc/passwd[x];[x]".into(),
        similarity: 0.15,
        blend: 0.1,
    }];
    let out = effect_filter(&evil, true);
    assert_eq!(out, "", "injected chroma color must emit NOTHING");
    assert!(
        !out.contains("movie="),
        "no injected filter reaches the graph"
    );

    // No effects → empty (byte-identical to pre-effects).
    assert_eq!(effect_filter(&[], true), "");

    // Each stylized retro effect lowers to its ffmpeg filter, verified
    // to parse in a filter_complex context.
    assert_eq!(
        effect_filter(&[E::Vhs { amount: 0.5 }], false),
        ",rgbashift=rh=4:bh=-4,noise=alls=12:allf=t,gblur=sigma=0.6"
    );
    assert_eq!(
        effect_filter(&[E::Posterize { levels: 8.0 }], false),
        ",lutrgb=r=floor(val/32)*32:g=floor(val/32)*32:b=floor(val/32)*32"
    );
    assert_eq!(effect_filter(&[E::Invert], false), ",negate");
    assert!(effect_filter(&[E::Emboss], false).starts_with(",convolution=-2 -1 0"));

    // Effects lower in ARRAY ORDER, so re-setting via edit.effect{effects}
    // in a new order IS a reorder (no dedicated reorder verb needed).
    let ab = effect_filter(
        &[E::Blur { radius: 5.0 }, E::Sharpen { amount: 1.0 }],
        false,
    );
    let ba = effect_filter(
        &[E::Sharpen { amount: 1.0 }, E::Blur { radius: 5.0 }],
        false,
    );
    assert_ne!(
        ab, ba,
        "effect stack order is honored (reorderable by re-set)"
    );

    // Denoise is an AUDIO effect: skipped by the VIDEO effect_filter, emitted
    // by audio_effect_filter (afftdn); visual effects are skipped there.
    let dn = vec![E::Denoise { amount: 0.5 }];
    assert_eq!(
        effect_filter(&dn, true),
        "",
        "denoise is not a video filter"
    );
    assert!(
        audio_effect_filter(&dn).contains(",afftdn=nr="),
        "{}",
        audio_effect_filter(&dn)
    );
    let comp = vec![E::Compressor { amount: 0.5 }];
    assert!(
        audio_effect_filter(&comp).contains(",acompressor=threshold=-20dB:ratio="),
        "{}",
        audio_effect_filter(&comp)
    );
    assert_eq!(
        effect_filter(&comp, false),
        "",
        "compressor is not a video filter"
    );
    let gate = vec![E::Gate { amount: 0.5 }];
    assert!(
        audio_effect_filter(&gate).contains(",agate=threshold="),
        "{}",
        audio_effect_filter(&gate)
    );
    assert!(
        audio_effect_filter(&gate).contains(":ratio="),
        "gate emits a ratio"
    );
    assert_eq!(
        effect_filter(&gate, false),
        "",
        "gate is not a video filter"
    );
    assert_eq!(
        audio_effect_filter(&fx),
        "",
        "visual effects are not audio filters"
    );

    // mirror/flip/hue_shift emit their one-line filters (hue 0 = no-op).
    assert_eq!(effect_filter(&[E::Mirror], false), ",hflip");
    assert_eq!(effect_filter(&[E::Flip], false), ",vflip");
    assert!(effect_filter(&[E::HueShift { degrees: 90.0 }], false).contains(",hue=h=90"));
    assert_eq!(effect_filter(&[E::HueShift { degrees: 0.0 }], false), "");
    // Creative looks: rgb-split (chromatic aberration), pixelize, sepia.
    assert_eq!(
        effect_filter(&[E::RgbSplit { amount: 6.0 }], false),
        ",rgbashift=rh=6:bh=-6"
    );
    assert_eq!(
        effect_filter(&[E::RgbSplit { amount: 0.0 }], false),
        "",
        "0 split = no-op"
    );
    assert_eq!(
        effect_filter(&[E::Pixelize { size: 16.0 }], false),
        ",pixelize=width=16:height=16"
    );
    assert!(effect_filter(&[E::Sepia], false).contains(",colorchannelmixer=.393:.769:.189"));
    assert_eq!(
        effect_filter(&[E::AutoColor { amount: 0.7 }], false),
        ",normalize=strength=0.7"
    );
}

/// edit.eq filter builder (eq_filter): None=empty; high-pass + bands + low-pass
/// emit in that order; a constant-Q peaking band uses `t=q:w={Q}:g={dB}`.
#[test]
fn eq_filter_emits_chain() {
    use cut_core::{ClipEq, EqBand};
    assert_eq!(eq_filter(None), "", "no EQ → byte-identical");
    let eq = ClipEq {
        high_pass_hz: Some(120.0),
        low_pass_hz: Some(6000.0),
        bands: vec![EqBand {
            freq_hz: 1000.0,
            gain_db: 6.0,
            q: 1.0,
        }],
    };
    let s = eq_filter(Some(&eq));
    // order: highpass first, peaking band, lowpass last.
    assert_eq!(
        s, ",highpass=f=120,equalizer=f=1000:t=q:w=1:g=6,lowpass=f=6000",
        "{s}"
    );
}

/// edit.keyframe expression builder (kf_expr): empty=0, single=constant, a
/// linear ramp emits clamp-before-first + a per-segment lerp, hold steps.
#[test]
fn kf_expr_builds_piecewise() {
    use cut_core::KfInterp::{Hold, Linear};
    assert_eq!(kf_expr(&[], "t", Linear), "0");
    assert_eq!(kf_expr(&[(1.0, 0.5)], "t", Linear), "0.5");
    // linear ramp 0→1 over [0,2]: outer clamp before t=0, segment branch at t=2.
    let e = kf_expr(&[(0.0, 0.0), (2.0, 1.0)], "t", Linear);
    assert!(
        e.starts_with("if(lt(t,0),0,"),
        "clamps before first point: {e}"
    );
    assert!(e.contains("if(lt(t,2)"), "segment to t=2: {e}");
    assert!(e.contains("/2)"), "lerp divides by the segment span: {e}");
    // LINEAR is byte-identical to the pre-easing form (replay-safety guard).
    assert_eq!(
        e, "if(lt(t,0),0,if(lt(t,2),(0+(1)*(t-0)/2),1))",
        "linear must match the exact legacy string: {e}"
    );
    // hold = stepped (no interpolation division in the segments).
    let h = kf_expr(&[(0.0, 0.2), (1.0, 0.8)], "T", Hold);
    assert!(h.contains("if(lt(T,1),0.2,0.8)"), "hold steps to v0: {h}");
}

/// An eased ramp wraps the inter-keyframe fraction in the Penner expression
/// (st(0,…) stores it once; the body reads ld(0)). ease_in_out_cubic must NOT
/// be the bare linear lerp — that is the whole point of the channel.
#[test]
fn kf_expr_eased_wraps_fraction() {
    use cut_core::KfInterp::{EaseInOutCubic, EaseOutBounce};
    let e = kf_expr(&[(0.0, 0.0), (2.0, 1.0)], "t", EaseInOutCubic);
    assert!(
        e.contains("st(0,clip("),
        "stores the clamped fraction once: {e}"
    );
    assert!(e.contains("ld(0)"), "body reads the stored fraction: {e}");
    assert!(
        e.contains("if(lt(ld(0),0.5),4*"),
        "ease_in_out_cubic shape present: {e}"
    );
    assert!(
        !e.contains("(0+(1)*(t-0)/2)"),
        "eased ≠ the bare linear lerp: {e}"
    );
    // bounce reuses slot 1 for its inner argument.
    let b = kf_expr(&[(0.0, 0.0), (1.0, 1.0)], "t", EaseOutBounce);
    assert!(
        b.contains("st(1,"),
        "bounce stores its hop argument in slot 1: {b}"
    );
}

/// A scale keyframe lowers to the proven zoompan chain (centred, clamped z).
#[test]
fn scale_kf_zoompan_emits_clamped_zoompan() {
    use cut_core::{Keyframe, KfInterp, KfParam, KfPoint};
    let kfs = vec![Keyframe {
        param: KfParam::Scale,
        points: vec![
            KfPoint {
                t_ms: 0,
                value: 1.0,
            },
            KfPoint {
                t_ms: 2000,
                value: 1.5,
            },
        ],
        interp: KfInterp::EaseInOutCubic,
    }];
    let z = scale_kf_zoompan(&kfs, 1280, 720, 30.0, 2000).expect("scale kf → Some");
    assert!(
        z.contains("zoompan=z='max(1,min(10,"),
        "z clamped [1,10]: {z}"
    );
    assert!(z.contains("s=1280x720"), "output sized to the frame: {z}");
    assert!(
        z.contains("setpts=N/"),
        "rebuilds clean PTS (frame-explosion fix): {z}"
    );
    assert!(z.contains("iw/2-(iw/zoom)/2"), "centred zoom window: {z}");
    // No scale keyframe → None (falls through to edit.animate).
    assert!(scale_kf_zoompan(&[], 1280, 720, 30.0, 2000).is_none());
}

#[test]
fn volume_kf_filter_clamps_easing_overshoot() {
    use cut_core::{Keyframe, KfInterp, KfParam, KfPoint};
    let kfs = vec![Keyframe {
        param: KfParam::Volume,
        points: vec![
            KfPoint {
                t_ms: 0,
                value: 0.0,
            },
            KfPoint {
                t_ms: 1000,
                value: 1.0,
            },
        ],
        interp: KfInterp::EaseOutElastic,
    }];
    let filter = volume_kf_filter(&kfs);
    assert!(filter.starts_with(",volume='max(0,min(16,"));
    assert!(filter.ends_with("))':eval=frame"));
}

/// Render ONE full-range gray frame, 256 columns wide, where the LUMA of column
/// `X` is `255 * kf_value(X/255)` — i.e. the ffmpeg evaluation of the REAL
/// `kf_expr` lowering (var = `X`, a 0→1 ramp over the 256 columns). Reading row 0
/// back gives the evaluator's value at 256 exact fractions across [0,1]. This is
/// SPATIAL (one frame), so it has none of the frame-timestamp ambiguity a time-
/// based ramp would; `format=gray` BEFORE geq keeps everything full-range (a
/// limited-range yuv source would rescale 16–235→0–255 and corrupt the readback —
/// measured). Returns the 256 luma bytes, or None if ffmpeg is unavailable.
fn render_kf_ramp_row(interp: cut_core::KfInterp) -> Option<Vec<u8>> {
    use std::process::Command;
    let bin = crate::ffmpeg::ffmpeg_bin();
    let dir = tempfile::tempdir().ok()?;
    let out = dir.path().join("ramp.gray");
    // The real lowering: a 0→1 ramp keyed on column X over [0,255].
    let expr = kf_expr(&[(0.0, 0.0), (255.0, 1.0)], "X", interp);
    let st = Command::new(&bin)
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg("color=c=black:s=256x4:r=1:d=1")
        .args(["-frames:v", "1", "-vf"])
        // clip(…,0,1) mirrors the real opacity-alpha path (geq does NOT clamp; a
        // raw negative wraps to 255). The in-[0,1] body of every curve is what we
        // verify; the back/elastic overshoot tails clamp identically on both sides.
        .arg(format!("format=gray,geq=lum='255*clip(({expr}),0,1)'"))
        .args(["-f", "rawvideo", "-pix_fmt", "gray"])
        .arg(&out)
        .status()
        .ok()?;
    if !st.success() {
        return None;
    }
    let bytes = std::fs::read(&out).ok()?;
    if bytes.len() < 256 {
        return None;
    }
    Some(bytes[..256].to_vec()) // row 0 = columns 0..255
}

/// Live easing proof: the ffmpeg expression `kf_expr`/`ease_frac_expr`
/// emit is numerically EQUAL to the pure-Rust reference `KfInterp::sample`, across
/// the WHOLE [0,1] domain (256 sample points), through the REAL ffmpeg evaluator.
/// This is what guarantees the rendered motion follows the chosen curve — not just
/// that the string parses. Every one of the 18 Penner curves is checked (back/
/// elastic overshoot, so both sides clamp to [0,255] identically). Runs live on the
/// dev box (ffmpeg present); skips cleanly if ffmpeg is absent.
#[test]
fn eased_expr_matches_sample_via_ffmpeg() {
    use cut_core::KfInterp as K;
    let all = [
        K::EaseInQuad,
        K::EaseOutQuad,
        K::EaseInOutQuad,
        K::EaseInCubic,
        K::EaseOutCubic,
        K::EaseInOutCubic,
        K::EaseInExpo,
        K::EaseOutExpo,
        K::EaseInOutExpo,
        K::EaseInBack,
        K::EaseOutBack,
        K::EaseInOutBack,
        K::EaseInElastic,
        K::EaseOutElastic,
        K::EaseInOutElastic,
        K::EaseInBounce,
        K::EaseOutBounce,
        K::EaseInOutBounce,
    ];
    let mut max_err = 0.0_f64;
    for interp in all {
        let Some(row) = render_kf_ramp_row(interp) else {
            eprintln!("ffmpeg unavailable — skipping live easing proof");
            return;
        };
        assert_eq!(row.len(), 256, "{interp:?}: short row");
        for (x, &got) in row.iter().enumerate() {
            let frac = x as f64 / 255.0;
            // ffmpeg writes clamp(round(255*expr), 0, 255); mirror that on sample.
            let want = (255.0 * interp.sample(frac)).round().clamp(0.0, 255.0);
            let err = (got as f64 - want).abs();
            max_err = max_err.max(err);
            assert!(
                err <= 2.0,
                "{interp:?} col {x} (frac={frac:.3}): ffmpeg luma {got} vs sample {want} — \
                 the ffmpeg lowering DISAGREES with the Rust reference"
            );
        }
    }
    eprintln!("easing check: 18 curves × 256 points, max |ffmpeg−sample| = {max_err}");
}

/// Adaptive window sizing is the Stage-3 safety calibration: a window's peak
/// RSS ≈ window_len × resolution × overlays, so the window must SHRINK as
/// those grow to keep peak near the budget on a small box. (Measured: 720p
/// 10s 2-overlay ≈ 647 MB.)
#[test]
fn render_window_shrinks_with_resolution_and_overlays() {
    std::env::remove_var("SHELLX_CUT_RENDER_WINDOW_SEC");
    std::env::remove_var("SHELLX_CUT_RENDER_WINDOW_BUDGET_MB");
    let w720_1 = render_window_ms(1280, 720, 1);
    let w720_2 = render_window_ms(1280, 720, 2);
    let w4k_2 = render_window_ms(3840, 2160, 2);
    // More overlays → smaller window.
    assert!(w720_2 < w720_1, "2 overlays must shrink the window vs 1");
    // 4K (9× the pixels) shrinks far more than 720p → protects a 16GB box.
    assert!(w4k_2 < w720_2, "4K must use a smaller window than 720p");
    // 4K/2-overlay floors at the 2s clamp (~1.2GB peak, not 30s ≈ 17GB).
    assert_eq!(w4k_2, 2000, "4K/2-overlay clamps to the 2s minimum");
    // Always within the [2s,60s] clamp.
    assert!((2000..=60_000).contains(&w720_1));
    // Hard override wins over the adaptive estimate.
    std::env::set_var("SHELLX_CUT_RENDER_WINDOW_SEC", "12");
    assert_eq!(render_window_ms(3840, 2160, 4), 12_000);
    std::env::remove_var("SHELLX_CUT_RENDER_WINDOW_SEC");
}

fn xfade_window_segment(timeline_in_ms: u64, xfade_in_ms: u64) -> EdlSegment {
    EdlSegment {
        track: "v1".into(),
        track_kind: TrackKind::Video,
        clip_id: Some(format!("c{timeline_in_ms}")),
        asset: Some(format!("a{timeline_in_ms}")),
        timeline_in_ms,
        timeline_out_ms: timeline_in_ms + 2000,
        src_in_ms: Some(0),
        src_out_ms: Some(2000),
        gain_db: 0.0,
        fade: None,
        crop: None,
        xfade_in_ms,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        grade_stack: vec![],
        grade_windows: vec![],
        matte: None,
        mask: None,
        effects: vec![],
        reverse: false,
        freeze: None,
        animation: None,
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        input_color_space: None,
        caption_text: None,
        style_ref: None,
    }
}

#[test]
fn plan_windows_rechecks_after_dissolve_nudge() {
    let edl = Edl {
        duration_ms: 5000,
        adjustments: vec![],
        segments: vec![
            xfade_window_segment(2000, 600),
            xfade_window_segment(1500, 700),
        ],
    };

    let windows = plan_windows(&edl, 1600, 1000.0);

    assert_eq!(
        windows[0],
        (0, 2600),
        "boundary must move out of both overlapping dissolve regions"
    );
}

/// The GPU fast-track is OFF by default and only flips on for explicit truthy
/// opt-in values — the safety that keeps the deterministic software path the
/// default. (`render_target` ANDs this with the hardware probe; that full gate
/// is proven live on a CUDA box. Here we lock down the env parsing, which is
/// deterministic on any machine.)
#[test]
fn gpu_opt_in_is_off_by_default_and_parses_truthy() {
    std::env::remove_var("SHELLX_CUT_RENDER_GPU");
    assert!(!gpu_opt_in(), "unset = off (software default)");
    for v in ["1", "true", "TRUE", "Yes", "on"] {
        std::env::set_var("SHELLX_CUT_RENDER_GPU", v);
        assert!(gpu_opt_in(), "{v:?} must be truthy");
    }
    for v in ["0", "false", "no", "off", "maybe", ""] {
        std::env::set_var("SHELLX_CUT_RENDER_GPU", v);
        assert!(!gpu_opt_in(), "{v:?} must be falsy");
    }
    std::env::remove_var("SHELLX_CUT_RENDER_GPU");
}

/// The v1 GPU-scope predicate is CONSERVATIVE: a bare cut+opaque-PiP timeline
/// is friendly, but EVERY op without a CUDA filter (grade/fade/crop/xfade/
/// captions/titles/base transform) flips it to software. A false positive here would
/// corrupt a GPU render, so each rejection reason is pinned.
#[test]
fn gpu_friendly_predicate_is_conservative() {
    use cut_core::{Clip, ClipFade, ClipTransform, MediaClip, ProjectSettings, Track, TrackKind};
    let settings = ProjectSettings {
        width: 320,
        height: 240,
        fps: 30.0,
        audio_rate: 48_000,
        color: cut_core::ColorConfig::default(),
    };
    // A bare opaque media clip on the base video track.
    let base_clip = |grade, fade, crop| {
        Clip::Media(MediaClip {
            id: "c1".into(),
            asset: "a1".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop,
            fade,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade,
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
        })
    };
    // The base video source, probed at the project geometry (aspect matches
    // the output → scale_cuda=W:H is faithful → eligible).
    let asset = |w: u64, h: u64| cut_core::Asset {
        path: "/x.mp4".into(),
        hash: "sha256:test".into(),
        probe: Some(serde_json::json!({ "width": w, "height": h })),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    };
    let friendly = || {
        let mut p = Project::new("gpu", settings.clone());
        p.assets.insert("a1".into(), asset(320, 240));
        p.track_mut("v1").unwrap().clips = vec![base_clip(None, None, None)];
        p
    };
    let edl_of = |p: &Project| cut_core::edl_from_project(p);
    let opts = RenderOptions::default();

    // Baseline: single base track, cuts + scale, matching aspect → friendly.
    let p = friendly();
    assert!(
        timeline_is_gpu_friendly(&p, &edl_of(&p), &opts),
        "bare single-track cut timeline must be GPU-friendly"
    );

    // The v1 CUDA graph does not apply base geometry or opacity.
    let mut p = friendly();
    if let Clip::Media(clip) = &mut p.track_mut("v1").unwrap().clips[0] {
        clip.transform = Some(ClipTransform {
            x: 0.25,
            y: 0.25,
            scale: 0.5,
            opacity: 0.5,
        });
    }
    assert!(
        !timeline_is_gpu_friendly(&p, &edl_of(&p), &opts),
        "base transform or opacity -> software"
    );

    // grade rejects. (ClipGrade has no Default — every field is serde-defaulted,
    // so an empty object deserializes to the identity grade.)
    let grade = serde_json::from_str::<cut_core::ClipGrade>("{}").unwrap();
    let mut p = friendly();
    p.track_mut("v1").unwrap().clips = vec![base_clip(Some(grade), None, None)];
    assert!(
        !timeline_is_gpu_friendly(&p, &edl_of(&p), &opts),
        "grade -> software"
    );

    // fade rejects.
    let mut p = friendly();
    p.track_mut("v1").unwrap().clips = vec![base_clip(
        None,
        Some(ClipFade {
            in_ms: 200,
            out_ms: 200,
            kind: cut_core::FadeKind::Video,
        }),
        None,
    )];
    assert!(
        !timeline_is_gpu_friendly(&p, &edl_of(&p), &opts),
        "fade -> software"
    );

    // caption track rejects.
    let mut p = friendly();
    p.tracks.push(Track {
        id: "cap1".into(),
        kind: TrackKind::Caption,
        clips: vec![Clip::Caption(cut_core::CaptionClip {
            id: "s1".into(),
            text: "hi".into(),
            style_ref: None,
            range_ms: [0, 1000],
        })],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    assert!(
        !timeline_is_gpu_friendly(&p, &edl_of(&p), &opts),
        "captions -> software"
    );

    // title-prefixed video track rejects.
    let mut p = friendly();
    p.tracks.push(Track {
        id: "title1".into(),
        kind: TrackKind::Video,
        clips: vec![base_clip(None, None, None)],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    assert!(
        !timeline_is_gpu_friendly(&p, &edl_of(&p), &opts),
        "titles -> software"
    );

    // An overlay PiP clip on a second video track (distinct or shared asset).
    let pip_clip = |aid: &str| {
        Clip::Media(MediaClip {
            id: "ov".into(),
            asset: aid.into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: Some(ClipTransform {
                x: 0.5,
                y: 0.5,
                scale: 0.4,
                opacity: 0.5, // opacity<1 is fine: the overlay alpha path handles it
            }),
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
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
        })
    };
    let with_overlay = |aid: &str, add_a2: bool| {
        let mut p = friendly();
        if add_a2 {
            p.assets.insert("a2".into(), asset(320, 240));
        }
        p.tracks.push(Track {
            id: "v2".into(),
            kind: TrackKind::Video,
            clips: vec![pip_clip(aid)],
            gain_db: 0.0,
            gain_windows: vec![],
            blend_mode: None,
            visible: true,
            locked: false,
            muted: false,
            solo: false,
            pan: 0.0,
        });
        p
    };

    // An overlay reusing the base asset is outside the base-track-only scope.
    let p = with_overlay("a1", false);
    assert!(
        !timeline_is_gpu_friendly(&p, &edl_of(&p), &opts),
        "shared-asset PiP overlay -> software"
    );

    // A distinct overlay asset still falls back: FFmpeg 6.1 overlay_cuda loses
    // NVDEC crop metadata and exposes padded hardware-surface rows.
    let p = with_overlay("a2", true);
    assert!(
        !timeline_is_gpu_friendly(&p, &edl_of(&p), &opts),
        "distinct-asset PiP overlay -> software until CUDA geometry parity is proven"
    );

    // Aspect MISMATCH rejects (8:3 source into a 4:3 output would distort).
    let mut p = friendly();
    p.assets.insert("a1".into(), asset(640, 240));
    assert!(
        !timeline_is_gpu_friendly(&p, &edl_of(&p), &opts),
        "aspect-mismatched source -> software"
    );

    // UNKNOWN source geometry (no probe) rejects — can't guarantee a faithful
    // resize.
    let mut p = friendly();
    p.assets.get_mut("a1").unwrap().probe = None;
    assert!(
        !timeline_is_gpu_friendly(&p, &edl_of(&p), &opts),
        "no probe geometry -> software"
    );
}

/// The VRAM bound decides GPU-vs-software on the estimated peak: the GPU has
/// NO cgroup backstop, so a timeline whose estimate exceeds budget MUST fall back
/// to software rather than OOM the GPU. Machine-independent — exercises the
/// estimate + the `SHELLX_CUT_GPU_VRAM_BUDGET_MB` override directly (no GPU). This
/// test is the sole user of that env in-process, so the set/remove is race-free.
#[test]
fn gpu_vram_bound_falls_back_when_over_budget() {
    use cut_core::{Clip, MediaClip, ProjectSettings, Track, TrackKind};
    let settings = ProjectSettings {
        width: 1920,
        height: 1080,
        fps: 30.0,
        audio_rate: 48_000,
        color: cut_core::ColorConfig::default(),
    };
    let media = |asset: &str| {
        Clip::Media(MediaClip {
            id: "c1".into(),
            asset: asset.into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
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
        })
    };
    let asset = |w: u64, h: u64| cut_core::Asset {
        path: "/x.mp4".into(),
        hash: "sha256:test".into(),
        probe: Some(serde_json::json!({ "width": w, "height": h })),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    };
    // 4K base (16:9 → matches the 1080p output) on a single video track.
    let mut p = Project::new("vram", settings.clone());
    p.assets.insert("a1".into(), asset(3840, 2160));
    p.track_mut("v1").unwrap().clips = vec![media("a1")];
    let edl = cut_core::edl_from_project(&p);
    let opts = RenderOptions::default();

    let est = gpu_vram_estimate_bytes(&p, &edl, &opts);
    assert!(est > 0, "estimate must be positive");

    // Adding an overlay track raises the estimate (more in-flight VRAM surfaces).
    let mut p2 = p.clone();
    p2.assets.insert("a2".into(), asset(3840, 2160));
    p2.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![media("a2")],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let edl2 = cut_core::edl_from_project(&p2);
    assert!(
        gpu_vram_estimate_bytes(&p2, &edl2, &opts) > est,
        "an extra overlay track must raise the VRAM estimate"
    );

    // Budget just BELOW the estimate → does not fit → software fallback.
    let est_mb = est / (1024 * 1024);
    std::env::set_var(
        "SHELLX_CUT_GPU_VRAM_BUDGET_MB",
        est_mb.saturating_sub(1).max(1).to_string(),
    );
    assert!(
        !gpu_vram_fits(&p, &edl, &opts),
        "estimate over budget must NOT fit (software fallback)"
    );
    // Budget comfortably ABOVE the estimate → fits → GPU eligible.
    std::env::set_var("SHELLX_CUT_GPU_VRAM_BUDGET_MB", (est_mb + 8).to_string());
    assert!(
        gpu_vram_fits(&p, &edl, &opts),
        "estimate within budget must fit (GPU eligible)"
    );
    std::env::remove_var("SHELLX_CUT_GPU_VRAM_BUDGET_MB");
}

/// Extract the value following a flag in an args vec ("-crf" → "20").
fn arg_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
}

/// format_codec_args: each format maps to the right encoder + extension, and
/// h264 is byte-identical to the named preset (no-`format` replay invariant).
#[test]
fn format_codec_args_maps_codec_and_extension() {
    // h264 == the named quality preset, verbatim (byte-identical default).
    let (v, a, ext) = format_codec_args("h264", "standard").unwrap();
    let std = RenderPreset::named("standard").unwrap();
    assert_eq!(v, std.video_args);
    assert_eq!(a, std.audio_args);
    assert_eq!(ext, "mp4");
    // hevc → libx265 in mp4 with the Apple hvc1 tag.
    let (v, _, ext) = format_codec_args("hevc", "standard").unwrap();
    assert_eq!(arg_after(&v, "-c:v"), Some("libx265"));
    assert_eq!(arg_after(&v, "-tag:v"), Some("hvc1"));
    assert_eq!(ext, "mp4");
    // vp9 → libvpx-vp9 + Opus in webm.
    let (v, a, ext) = format_codec_args("vp9", "standard").unwrap();
    assert_eq!(arg_after(&v, "-c:v"), Some("libvpx-vp9"));
    assert_eq!(arg_after(&a, "-c:a"), Some("libopus"));
    assert_eq!(ext, "webm");
    // prores → prores_ks 422 in mov.
    let (v, _, ext) = format_codec_args("prores", "high").unwrap();
    assert_eq!(arg_after(&v, "-c:v"), Some("prores_ks"));
    assert_eq!(ext, "mov");
    // av1 → libsvtav1 in mp4.
    let (v, _, ext) = format_codec_args("av1", "draft").unwrap();
    assert_eq!(arg_after(&v, "-c:v"), Some("libsvtav1"));
    assert_eq!(ext, "mp4");
    // quality tier shifts the rate knob (draft != high CRF for hevc).
    let draft = format_codec_args("hevc", "draft").unwrap().0;
    let high = format_codec_args("hevc", "high").unwrap().0;
    assert_ne!(arg_after(&draft, "-crf"), arg_after(&high, "-crf"));
    // unknown format → None (the verb turns it into an actionable error).
    assert!(format_codec_args("mkv", "standard").is_none());
}

/// parse_bitrate_kbps: unit grammar (M/k/bare) + range guard.
#[test]
fn parse_bitrate_units_and_range() {
    assert_eq!(parse_bitrate_kbps("12M"), Some(12_000)); // 12 Mbps → kbps
    assert_eq!(parse_bitrate_kbps("12m"), Some(12_000));
    assert_eq!(parse_bitrate_kbps("0.5M"), Some(500)); // fractional Mbps
    assert_eq!(parse_bitrate_kbps("12000k"), Some(12_000));
    assert_eq!(parse_bitrate_kbps("12000"), Some(12_000)); // bare = kbps
    assert_eq!(parse_bitrate_kbps("  20M "), Some(20_000)); // trimmed
    assert_eq!(parse_bitrate_kbps("0"), None); // zero rejected
    assert_eq!(parse_bitrate_kbps("-5M"), None); // negative rejected
    assert_eq!(parse_bitrate_kbps("10"), None); // <50 kbps out of range
    assert_eq!(parse_bitrate_kbps("999M"), None); // >500 Mbps out of range
    assert_eq!(parse_bitrate_kbps("fast"), None); // garbage
}

#[test]
fn pixel_rounding_helpers_clamp_nonfinite_and_keep_even_geometry() {
    assert_eq!(even_size_px(f64::NAN, 1920), 2);
    assert_eq!(even_size_px(f64::INFINITY, 1920), 1920);
    assert_eq!(even_size_px(123.6, 1919), 124);
    assert_eq!(even_pos_px(f64::NEG_INFINITY, 1919), 0);
    assert_eq!(even_pos_px(f64::INFINITY, 1919), 1918);
    assert_eq!(even_pos_px(13.9, 1919), 14);
}

/// apply_bitrate: strips the CRF/quality knob and states a real bitrate
/// target; per-encoder rate grammar; ProRes is left untouched.
#[test]
fn apply_bitrate_rewrites_rate_control() {
    // Software x264 VBR: -crf gone, -b:v/-maxrate/-bufsize present.
    let base = format_codec_args("h264", "standard").unwrap().0;
    assert!(arg_after(&base, "-crf").is_some());
    let vbr = apply_bitrate(base.clone(), 12_000, false, "libx264");
    assert_eq!(arg_after(&vbr, "-crf"), None); // CRF stripped
    assert_eq!(arg_after(&vbr, "-b:v"), Some("12000k"));
    assert_eq!(arg_after(&vbr, "-maxrate"), Some("17400k")); // 1.45×
    assert!(arg_after(&vbr, "-bufsize").is_some());
    assert!(arg_after(&vbr, "-minrate").is_none()); // VBR ≠ CBR
                                                    // x264 CBR: min=max=target + true-CBR HRD flag (pads to target).
    let cbr = apply_bitrate(base.clone(), 12_000, true, "libx264");
    assert_eq!(arg_after(&cbr, "-minrate"), Some("12000k"));
    assert_eq!(arg_after(&cbr, "-maxrate"), Some("12000k"));
    assert_eq!(arg_after(&cbr, "-x264-params"), Some("nal-hrd=cbr"));
    // x265 CBR uses strict-cbr instead.
    let hevc = format_codec_args("hevc", "standard").unwrap().0;
    let cbr265 = apply_bitrate(hevc, 8_000, true, "libx265");
    assert_eq!(arg_after(&cbr265, "-x265-params"), Some("strict-cbr=1"));
    // vp9's `-b:v 0` (CRF mode) is replaced by a real bitrate.
    let vp9 = format_codec_args("vp9", "standard").unwrap().0;
    assert_eq!(arg_after(&vp9, "-b:v"), Some("0"));
    let vp9b = apply_bitrate(vp9, 8_000, false, "libvpx-vp9");
    assert_eq!(arg_after(&vp9b, "-b:v"), Some("8000k"));
    // NVENC uses -rc vbr/cbr, not -minrate.
    let nv = apply_bitrate(
        vec![
            "-c:v".into(),
            "h264_nvenc".into(),
            "-cq".into(),
            "23".into(),
        ],
        10_000,
        false,
        "h264_nvenc",
    );
    assert_eq!(arg_after(&nv, "-rc"), Some("vbr"));
    assert_eq!(arg_after(&nv, "-cq"), None); // -cq stripped
    assert_eq!(arg_after(&nv, "-b:v"), Some("10000k"));
    // ProRes ignores a bitrate target (profile-fixed).
    let pr = format_codec_args("prores", "standard").unwrap().0;
    assert_eq!(apply_bitrate(pr.clone(), 50_000, false, "prores_ks"), pr);
}

/// set_audio_bitrate rewrites -b:a; leaves PCM (no -b:a) untouched.
#[test]
fn audio_bitrate_override() {
    let aac = vec!["-c:a".into(), "aac".into(), "-b:a".into(), "192k".into()];
    let hi = set_audio_bitrate(aac, 384);
    assert_eq!(arg_after(&hi, "-b:a"), Some("384k"));
    let pcm = vec!["-c:a".into(), "pcm_s16le".into()];
    assert_eq!(set_audio_bitrate(pcm.clone(), 384), pcm); // no -b:a → unchanged
}

/// platform_spec: canonical ids + aliases resolve to the researched geometry;
/// unknown → None. Vertical platforms are 9:16, YouTube/X 16:9.
#[test]
fn platform_specs_resolve() {
    let yt = platform_spec("youtube").unwrap();
    assert_eq!((yt.width, yt.height), (1920, 1080));
    assert_eq!(yt.audio_kbps, 384);
    let tt = platform_spec("tiktok").unwrap();
    assert_eq!((tt.width, tt.height), (1080, 1920)); // 9:16
                                                     // Aliases map to the same spec.
    assert_eq!(platform_spec("shorts"), platform_spec("tiktok"));
    assert_eq!(platform_spec("twitter"), platform_spec("x"));
    assert_eq!(platform_spec("ig"), platform_spec("reels"));
    assert_eq!(platform_spec("youtube_4k").unwrap().width, 3840);
    assert!(platform_spec("myspace").is_none());
    // Every canonical name resolves.
    for name in PLATFORM_NAMES {
        assert!(platform_spec(name).is_some(), "{name} should resolve");
    }
}

/// edit.speed render filters: video setpts divides by speed (identity at
/// 1.0); audio uses pitch-preserved atempo, sqrt-split outside [0.5,2.0].
#[test]
fn speed_filter_strings() {
    // Video setpts.
    assert_eq!(video_setpts(1.0), "setpts=PTS-STARTPTS"); // byte-identical
    assert_eq!(video_setpts(2.0), "setpts=(PTS-STARTPTS)/2");
    assert_eq!(video_setpts(0.5), "setpts=(PTS-STARTPTS)/0.5");
    assert_eq!(video_setpts(1.75), "setpts=(PTS-STARTPTS)/1.75");

    // Audio, pitch-preserved (the default): empty at 1.0, single atempo in
    // range, sqrt-split outside it (√4=2, √0.25=0.5 — both valid atempo).
    assert_eq!(audio_speed_filter(1.0, true, 48_000), "");
    assert_eq!(audio_speed_filter(2.0, true, 48_000), "atempo=2,");
    assert_eq!(audio_speed_filter(0.5, true, 48_000), "atempo=0.5,");
    assert_eq!(audio_speed_filter(4.0, true, 48_000), "atempo=2,atempo=2,");
    assert_eq!(
        audio_speed_filter(0.25, true, 48_000),
        "atempo=0.5,atempo=0.5,"
    );
    // 3.0 splits into √3≈1.732051 twice (1.732051² ≈ 3.0).
    assert_eq!(
        audio_speed_filter(3.0, true, 48_000),
        "atempo=1.732051,atempo=1.732051,"
    );

    // Audio varispeed (preserve_pitch=false) — the reserved v2 path:
    // reinterpret at rate*speed then resample back (pitch follows speed).
    assert_eq!(audio_speed_filter(1.0, false, 48_000), "");
    assert_eq!(
        audio_speed_filter(2.0, false, 48_000),
        "asetrate=96000,aresample=48000,"
    );
    assert_eq!(
        audio_speed_filter(0.5, false, 44_100),
        "asetrate=22050,aresample=44100,"
    );
}

#[test]
fn grade_filter_strings() {
    use cut_core::ClipGrade;
    let ident = ClipGrade {
        contrast: 1.0,
        brightness: 0.0,
        saturation: 1.0,
        gamma: 1.0,
        temperature_k: None,
        lut: None,
    };
    // None / identity → "" (byte-identical to ungraded).
    assert_eq!(grade_filter(None), "");
    assert_eq!(grade_filter(Some(&ident)), "");
    // Parametric-only (B&W punch).
    let g = ClipGrade {
        saturation: 0.0,
        contrast: 1.15,
        ..ident.clone()
    };
    assert_eq!(
        grade_filter(Some(&g)),
        ",eq=contrast=1.15:brightness=0:saturation=0:gamma=1"
    );
    // Temperature appended (note: eq is still emitted even at identity knobs).
    let gt = ClipGrade {
        temperature_k: Some(5200),
        ..ident.clone()
    };
    assert_eq!(
        grade_filter(Some(&gt)),
        ",eq=contrast=1:brightness=0:saturation=1:gamma=1,colortemperature=temperature=5200"
    );
    // Temperature clamps to ffmpeg's supported band (>=1000).
    let gc = ClipGrade {
        temperature_k: Some(50),
        ..ident.clone()
    };
    assert!(grade_filter(Some(&gc)).contains("colortemperature=temperature=1000"));
    // A LUT appends lut3d=file=… last.
    let gl = ClipGrade {
        lut: Some("/x/look.cube".into()),
        ..ident
    };
    assert!(grade_filter(Some(&gl)).contains("lut3d=file="));
}

/// grade_stack_filter (edit.grade_stack): an EMPTY stack is byte-identical to the
/// single-grade path, a SINGLE-element stack is byte-identical to that one grade via
/// edit.grade, and an N-element stack concatenates each layer's filter IN ORDER.
#[test]
fn grade_stack_filter_layers_and_byte_identity() {
    use cut_core::ClipGrade;
    let ident = ClipGrade {
        contrast: 1.0,
        brightness: 0.0,
        saturation: 1.0,
        gamma: 1.0,
        temperature_k: None,
        lut: None,
    };
    let g1 = ClipGrade {
        saturation: 0.0,
        contrast: 1.15,
        ..ident.clone()
    };
    let g2 = ClipGrade {
        temperature_k: Some(5200),
        ..ident.clone()
    };
    // EMPTY stack == the single-grade path EXACTLY (legacy, byte-identical):
    // for None and for a present single grade.
    assert_eq!(grade_stack_filter(None, &[]), grade_filter(None));
    assert_eq!(grade_stack_filter(Some(&g1), &[]), grade_filter(Some(&g1)));
    // SINGLE-element stack (grade None, as edit.grade_stack leaves it) == the
    // equivalent single edit.grade → BYTE-IDENTICAL render.
    assert_eq!(
        grade_stack_filter(None, std::slice::from_ref(&g1)),
        grade_filter(Some(&g1))
    );
    // TWO-element stack = layer1's filter THEN layer2's filter, in order.
    assert_eq!(
        grade_stack_filter(None, &[g1.clone(), g2.clone()]),
        format!("{}{}", grade_filter(Some(&g1)), grade_filter(Some(&g2)))
    );
    // A non-empty stack supersedes the single grade arg (a stacked clip has
    // grade=None, but prove the stack wins even if a grade were passed).
    assert_eq!(
        grade_stack_filter(Some(&g2), std::slice::from_ref(&g1)),
        grade_filter(Some(&g1))
    );
}

/// grade_window_block (edit.grade_window): the region-grade composite reuses the proven
/// split→effect→alphamerge→overlay recipe — but the "effect" is the window's grade
/// filter (so the region's pixels match the equivalent whole-frame edit.grade) and the
/// alpha comes from the baked shape PNG input. `invert` negates the alpha; `shortest=1`
/// is load-bearing (bounds the infinite -loop 1 PNG). The vfade rides the FINAL block.
#[test]
fn grade_window_block_emits_region_grade_composite() {
    use cut_core::{ClipGrade, GradeWindow, MaskShape, WindowShape};
    let grade = ClipGrade {
        brightness: 0.5,
        contrast: 1.0,
        saturation: 1.0,
        gamma: 1.0,
        temperature_k: None,
        lut: None,
    };
    let gw = GradeWindow {
        window: WindowShape {
            shape: MaskShape::Rect,
            points: vec![[0.0, 0.0], [0.5, 1.0]],
            feather: 0.0,
            invert: false,
        },
        grade: grade.clone(),
    };
    let blk = grade_window_block(&gw, 3, "v0pre", "v0", "v0w0", 1920, 1080, "");
    // The graded copy carries the EXACT whole-frame grade filter (so inside the region
    // the look matches a plain edit.grade with the same params).
    let gf = grade_filter(Some(&grade));
    assert!(gf.contains("brightness=0.5"), "grade filter: {gf}");
    assert!(
        blk.contains(&format!("null{gf},format=yuv420p")),
        "graded copy uses the grade filter: {blk}"
    );
    // Region scoping: the baked alpha is PNG input #3, scaled to frame, alphamerged in,
    // overlaid with shortest=1; output is the target label.
    assert!(
        blk.contains("[3:v]scale=1920:1080:flags=bilinear,format=gray["),
        "alpha from PNG input #3: {blk}"
    );
    assert!(blk.contains("alphamerge"), "scoped via alphamerge: {blk}");
    assert!(
        blk.contains("overlay=shortest=1,format=yuv420p[v0];"),
        "shortest=1 overlay → target label, no fade: {blk}"
    );
    assert!(
        !blk.contains("negate"),
        "non-inverted window does NOT negate: {blk}"
    );

    // Inverted window negates the alpha (grade the SURROUND, region untouched).
    let gw_inv = GradeWindow {
        window: WindowShape {
            invert: true,
            ..gw.window.clone()
        },
        grade: grade.clone(),
    };
    let blk_i = grade_window_block(&gw_inv, 3, "v0pre", "v0", "v0w0", 1920, 1080, "");
    assert!(
        blk_i.contains("format=gray,negate["),
        "inverted window negates the alpha: {blk_i}"
    );

    // A non-final block carries no vfade; the FINAL block appends it before the label.
    let blk_fade = grade_window_block(
        &gw,
        3,
        "v0pre",
        "v0",
        "v0w0",
        1920,
        1080,
        ",fade=t=in:st=0:d=0.5",
    );
    assert!(
        blk_fade.contains("overlay=shortest=1,format=yuv420p,fade=t=in:st=0:d=0.5[v0];"),
        "final block rides vfade: {blk_fade}"
    );
}

/// Power windows and masks produce a named region-composite stream before the
/// base clip's transform/opacity. Those snippets are comma-prefixed by design;
/// the join must provide a real filter head or ffmpeg interprets the leading
/// comma as an empty filter (`No such filter: ''`).
#[test]
fn region_composite_base_transform_has_explicit_filter_head() {
    let transform = cut_core::ClipTransform {
        x: 0.4,
        y: 0.0,
        scale: 1.0,
        opacity: 1.0,
    };
    let vtransform = base_transform_filter(&transform, 1280, 720);
    assert!(
        vtransform.starts_with(",scale="),
        "fixture transform: {vtransform}"
    );

    let block = base_region_transform_block("v0region", "v0", &vtransform, "");
    assert!(
        block.starts_with("[v0region]null,scale="),
        "region join must head the comma-prefixed transform: {block}"
    );
    assert!(
        !block.contains("] ,") && !block.contains("],"),
        "region join must not expose an empty filter: {block}"
    );
    assert!(
        block.ends_with("[v0];"),
        "region join output label: {block}"
    );
}

/// WindowShape::to_mask lowers a power window to the geometry-only ClipMask the renderer
/// bakes — invert is left FALSE here (the renderer negates), so two windows differing
/// only in invert share one baked alpha (identical cache_tag).
#[test]
fn window_shape_to_mask_is_geometry_only() {
    use cut_core::{MaskShape, WindowShape};
    let w = WindowShape {
        shape: MaskShape::Ellipse,
        points: vec![[0.5, 0.5], [0.3, 0.2]],
        feather: 0.04,
        invert: true,
    };
    let m = w.to_mask();
    assert_eq!(m.shape, MaskShape::Ellipse);
    assert_eq!(m.points, vec![[0.5, 0.5], [0.3, 0.2]]);
    assert_eq!(m.feather, 0.04);
    assert!(!m.invert, "invert is applied by the renderer, not the bake");
    assert!(m.track.is_none() && m.regions.is_empty());
    // Same geometry, opposite invert → SAME baked alpha (shared cache_tag).
    let w2 = WindowShape {
        invert: false,
        ..w.clone()
    };
    assert_eq!(
        w.to_mask().cache_tag(1920, 1080),
        w2.to_mask().cache_tag(1920, 1080)
    );
}

#[test]
fn bake_mask_png_atomic_publishes_final_without_temp_artifacts() {
    use cut_core::{ClipMask, MaskEffect, MaskShape};
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("cache").join("mask.png");
    let mask = ClipMask {
        shape: MaskShape::Rect,
        points: vec![[0.1, 0.1], [0.9, 0.9]],
        feather: 0.0,
        invert: false,
        effect: MaskEffect::Blur,
        strength: None,
        range_ms: None,
        track: None,
        regions: vec![],
    };

    bake_mask_png_atomic(&mask, 64, 64, &out).expect("atomic bake");
    let bytes = std::fs::read(&out).expect("final png");
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "valid PNG header");
    let leftovers: Vec<_> = std::fs::read_dir(out.parent().unwrap())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "atomic publish should clean temp files: {leftovers:?}"
    );
}

/// the proxy-crop contract: source→proxy crop mapping. The exact regression scenario is a 4K
/// source (3840×2160) cropped to the 16:9 content rect {96,54,3648,2052};
/// at the 0.25 proxy scale (no letterbox — 4K is already 16:9) it must land
/// at proxy coords [24,14,912,512] (even-forced). The mapped rect must
/// always sit inside the 960×540 proxy frame.
#[test]
fn map_crop_to_proxy_4k_fixture_rect() {
    use cut_core::ClipCrop;
    let c = ClipCrop {
        x: 96,
        y: 54,
        w: 3648,
        h: 2052,
    };
    let [x, y, w, h] = map_crop_to_proxy(&c, 3840, 2160).expect("maps");
    // f = 0.25, no letterbox. 96*.25=24, 54*.25=13.5→14, 3648*.25=912,
    // 2052*.25=513→512 (even). All within 960×540.
    assert_eq!([x, y, w, h], [24, 14, 912, 512]);
    assert!(x + w <= crate::proxy::PROXY_WIDTH && y + h <= crate::proxy::PROXY_HEIGHT);
    // All even (yuv420 crop alignment).
    assert!(x % 2 == 0 && y % 2 == 0 && w % 2 == 0 && h % 2 == 0);
}

/// A LETTERBOXED source (a 1:1 square in a 16:9 proxy) puts the content in a
/// centered pillarbox; a crop's proxy x must include the pillar offset.
#[test]
fn map_crop_to_proxy_letterboxes_non_16x9_source() {
    use cut_core::ClipCrop;
    // 1080×1080 square → f = min(960/1080, 540/1080) = 0.5 → 540×540 box,
    // centered in 960 wide → pad_x = (960-540)/2 = 210, pad_y = 0.
    // Crop the top-left 540×540 source quadrant {0,0,540,540}.
    let c = ClipCrop {
        x: 0,
        y: 0,
        w: 540,
        h: 540,
    };
    let [x, y, w, h] = map_crop_to_proxy(&c, 1080, 1080).expect("maps");
    assert_eq!(x, 210, "pillar offset folded into proxy x");
    assert_eq!(y, 0);
    assert_eq!([w, h], [270, 270], "540*0.5 each");
    assert!(x + w <= crate::proxy::PROXY_WIDTH);
}

/// A degenerate (sub-2px) mapped rect bails to None so the caller keeps the
/// composed fallback rather than emit a bad crop= filter.
#[test]
fn map_crop_to_proxy_degenerate_is_none() {
    use cut_core::ClipCrop;
    // A 2px-wide crop of a 4K source maps to <1px proxy → None.
    let c = ClipCrop {
        x: 0,
        y: 0,
        w: 2,
        h: 2,
    };
    assert!(map_crop_to_proxy(&c, 3840, 2160).is_none());
}

/// source_dims reads width/height from the probe; missing/zero → None
/// (the scrub path then keeps the composed fallback).
#[test]
fn source_dims_reads_probe_geometry() {
    use cut_core::Asset;
    let mk = |probe: Option<serde_json::Value>| Asset {
        path: "/x.mp4".into(),
        hash: "sha256:x".into(),
        probe,
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    };
    assert_eq!(
        source_dims(&mk(Some(serde_json::json!({"width":3840,"height":2160})))),
        Some((3840, 2160))
    );
    assert_eq!(source_dims(&mk(None)), None);
    assert_eq!(
        source_dims(&mk(Some(serde_json::json!({"width":0,"height":2160})))),
        None
    );
    assert_eq!(
        source_dims(&mk(Some(serde_json::json!({"kind":"audio"})))),
        None
    );
}

/// Exact-size, exact-fps clips should avoid redundant conform/fps filters.
/// This preserves stabilized footage: ffmpeg's `scale`/`fps` filters are not
/// visually free even when their requested output equals the input.
#[test]
fn exact_probe_uses_lossless_conform_and_timestamp_normalizer() {
    use cut_core::Asset;
    let asset = Asset {
        path: "/x.mp4".into(),
        hash: "sha256:x".into(),
        probe: Some(serde_json::json!({
            "kind": "video",
            "width": 320,
            "height": 240,
            "fps": 30.0,
        })),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    };

    assert_eq!(
        conform_filter_for_asset(&asset, None, 320, 240, Fit::Contain),
        "setsar=1"
    );
    assert_eq!(
        fps_filter_for_asset(&asset, 30.0, 1.0, false),
        "settb=expr=1/30,setpts=N/(30*TB)"
    );
    assert!(conform_filter_for_asset(&asset, None, 1280, 720, Fit::Contain).starts_with("scale="));
    assert_eq!(fps_filter_for_asset(&asset, 24.0, 1.0, false), "fps=24");
    assert_eq!(fps_filter_for_asset(&asset, 30.0, 2.0, false), "fps=30");
    assert_eq!(fps_filter_for_asset(&asset, 30.0, 1.0, true), "fps=30");
}

/// fold_video crossfade: a flagged seam emits `xfade` with the right
/// offset (acc end − overlap) and shortens the running duration by the
/// overlap; an unflagged seam emits a pairwise `concat`. Returns the
/// realized (crossfade-shortened) total duration.
#[test]
fn fold_video_xfade_offsets_and_duration() {
    let seg = |label: &str, dur, xf| SegStream {
        label: label.into(),
        dur_ms: dur,
        xfade_in_ms: xf,
        xfade_kind: None,
    };
    // Three 2000ms segments: a hard cut a→b, then a 1000ms crossfade b→c.
    let segs = vec![
        seg("v0", 2000, 0),
        seg("v1", 2000, 0),
        seg("v2", 2000, 1000),
    ];
    let mut f = String::new();
    let total = fold_video(&mut f, &segs, "vout", 60.0);
    // Hard cut first → pairwise concat; crossfade second → xfade.
    assert!(
        f.contains("[v0][v1]concat=n=2:v=1:a=0"),
        "hard cut is a pairwise concat:\n{f}"
    );
    // the crossfade-timebase contract: BOTH xfade legs are timebase+fps normalised (settb=AVTB,fps)
    // before the xfade — without this the 1/1000000 (concat output) vs
    // 1/60 (frame) mismatch breaks the whole compose graph at 60fps.
    assert!(
        f.contains("settb=AVTB,fps=60") && f.matches("settb=AVTB,fps=60").count() == 2,
        "both xfade input legs normalise timebase+fps (the crossfade-timebase contract):\n{f}"
    );
    // After the concat the accumulator is 4000ms; the xfade offset = 4000-1000.
    assert!(
        f.contains("xfade=transition=fade:duration=1.000:offset=3.000"),
        "crossfade offset = acc_end − overlap:\n{f}"
    );
    // Realized total = 2000 + 2000 + (2000 − 1000) = 5000ms (the overlap
    // shortens the timeline — matches the EDL pullback + ffmpeg xfade).
    assert_eq!(
        total, 5000,
        "crossfade shortens the realized duration by the overlap"
    );
    // The final stream is bound to the requested label.
    assert!(f.contains("[vout]"), "output bound to vout:\n{f}");
}

/// fold_audio: a flagged seam emits `acrossfade=d=`, an unflagged seam
/// a pairwise concat — the audio mirror of fold_video.
#[test]
fn fold_audio_acrossfade_at_flagged_seam() {
    let seg = |label: &str, xf| SegStream {
        label: label.into(),
        dur_ms: 2000,
        xfade_in_ms: xf,
        xfade_kind: None,
    };
    let segs = vec![seg("a0", 0), seg("a1", 500)];
    let mut f = String::new();
    fold_audio(&mut f, &segs, "aout");
    assert!(
        f.contains("[a0][a1]acrossfade=d=0.500"),
        "flagged seam → acrossfade:\n{f}"
    );
    assert!(f.contains("[aout]"), "output bound to aout:\n{f}");
    // No crossfade → plain pairwise concat (the byte-identical path is the
    // N-way concat in build_graph; fold_audio is only called when a seam
    // crossfades, but its no-xfade branch must still be a concat).
    let mut f2 = String::new();
    fold_audio(&mut f2, &[seg("a0", 0), seg("a1", 0)], "aout");
    assert!(
        f2.contains("concat=n=2:v=0:a=1"),
        "unflagged seam → concat:\n{f2}"
    );
}

/// output_geometry: project default, match_source uses the
/// largest VIDEO source, a cropped clip contributes its CROP rect, and the
/// audio mirror of the same asset must NOT re-introduce the full source
/// height (the live-proof bug: an a1t segment carries the asset's video
/// probe but no crop, so counting it undid the crop → bands came back).
#[test]
fn output_geometry_match_source_respects_crop_and_ignores_audio() {
    use cut_core::{Asset, Clip, ClipCrop, MediaClip, Project, ProjectSettings};
    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 1920,
            height: 1080,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        Asset {
            path: "/x.mp4".into(),
            hash: "sha256:x".into(),
            probe: Some(
                serde_json::json!({"kind":"video","width":3840,"height":2160,"duration_ms":54000}),
            ),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    // Cropped video clip (remove the 54px bands) + audio mirror (no crop).
    let mut vclip = MediaClip {
        id: "c1".into(),
        asset: "a1".into(),
        src_in_ms: 0,
        src_out_ms: 3000,
        effects: vec![],
        gain_db: 0.0,
        transform: None,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
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
    };
    vclip.crop = Some(ClipCrop {
        x: 0,
        y: 54,
        w: 3840,
        h: 2052,
    });
    p.track_mut("v1").unwrap().clips.push(Clip::Media(vclip));
    p.track_mut("a1t")
        .unwrap()
        .clips
        .push(Clip::Media(MediaClip {
            id: "c2".into(),
            asset: "a1".into(),
            src_in_ms: 0,
            src_out_ms: 3000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
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
        }));
    let edl = cut_core::edl_from_project(&p);

    // Project mode: settings geometry, unchanged.
    let proj = RenderOptions {
        fit: Fit::Contain,
        resolution: Resolution::Project,
        loudness_target: None,
    };
    assert_eq!(proj.output_geometry(&p, &edl), (1920, 1080));

    // match_source: the cropped VIDEO clip's rect (3840x2052), NOT the
    // full source height (2160) the audio mirror would otherwise inject.
    let ms = RenderOptions {
        fit: Fit::Contain,
        resolution: Resolution::MatchSource,
        loudness_target: None,
    };
    assert_eq!(
        ms.output_geometry(&p, &edl),
        (3840, 2052),
        "match_source must use the crop rect and ignore the audio mirror's full-height probe"
    );

    // Explicit (reframe / multi-format): the override geometry verbatim,
    // even-rounded, independent of project settings and timeline sources.
    let ex = RenderOptions {
        fit: Fit::Cover,
        resolution: Resolution::Explicit {
            width: 1080,
            height: 1920,
        },
        loudness_target: None,
    };
    assert_eq!(
        ex.output_geometry(&p, &edl),
        (1080, 1920),
        "explicit geometry verbatim"
    );
    let odd = RenderOptions {
        fit: Fit::Cover,
        resolution: Resolution::Explicit {
            width: 1081,
            height: 1921,
        },
        loudness_target: None,
    };
    assert_eq!(
        odd.output_geometry(&p, &edl),
        (1080, 1920),
        "explicit dims rounded even"
    );
}

/// Loudness normalization (render.final `normalize_loudness`): a target
/// appends a single-pass loudnorm to the mixed audio (→ anorm); no target =
/// byte-identical default audio graph (no loudnorm).
#[test]
fn loudnorm_applied_only_when_target_set() {
    use cut_core::{Asset, Clip, MediaClip, Project, ProjectSettings};
    // build_graph asserts the asset file exists — give it a real one.
    let tmp = tempfile::tempdir().unwrap();
    let media = tmp.path().join("x.mp4");
    std::fs::write(&media, b"stub").unwrap();
    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 1920,
            height: 1080,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        Asset {
            path: media.to_string_lossy().into_owned(),
            hash: "sha256:x".into(),
            probe: Some(serde_json::json!({
                "kind":"video","width":1920,"height":1080,"duration_ms":3000,"has_audio":true
            })),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    let clip = |id: &str| {
        Clip::Media(MediaClip {
            id: id.into(),
            asset: "a1".into(),
            src_in_ms: 0,
            src_out_ms: 3000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
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
        })
    };
    p.track_mut("v1").unwrap().clips.push(clip("c1"));
    p.track_mut("a1t").unwrap().clips.push(clip("c2"));
    let edl = cut_core::edl_from_project(&p);
    let dir = std::path::Path::new("/tmp");

    // No target → no loudnorm (byte-identical default audio graph).
    let g0 = build_graph(
        &p,
        &edl,
        dir,
        true,
        true,
        true,
        RenderOptions::default(),
        None,
    )
    .unwrap();
    assert!(
        !g0.filter.contains("loudnorm"),
        "default must not normalize:\n{}",
        g0.filter
    );

    // Target -16 LUFS → single-pass loudnorm, audio mapped to anorm.
    let opts = RenderOptions {
        loudness_target: Some(-16),
        ..RenderOptions::default()
    };
    let g = build_graph(&p, &edl, dir, true, true, true, opts, None).unwrap();
    assert!(
        g.filter.contains("loudnorm=I=-16:TP=-1.0:LRA=11"),
        "target → loudnorm:\n{}",
        g.filter
    );
    assert_eq!(g.audio_out.as_deref(), Some("anorm"));
}

#[test]
fn hidden_base_video_preserves_overlay_layer_over_black() {
    use cut_core::{
        Asset, Clip, ClipTransform, MediaClip, Project, ProjectSettings, Track, TrackKind,
    };
    let tmp = tempfile::tempdir().unwrap();
    let base_media = tmp.path().join("base.mp4");
    let overlay_media = tmp.path().join("overlay.mp4");
    std::fs::write(&base_media, b"base").unwrap();
    std::fs::write(&overlay_media, b"overlay").unwrap();

    let mut p = Project::new("t", ProjectSettings::default());
    p.assets.insert(
        "base".into(),
        Asset {
            path: base_media.to_string_lossy().into_owned(),
            hash: "sha256:base".into(),
            probe: Some(
                serde_json::json!({"kind":"video","width":1920,"height":1080,"duration_ms":1000}),
            ),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    p.assets.insert(
        "overlay".into(),
        Asset {
            path: overlay_media.to_string_lossy().into_owned(),
            hash: "sha256:overlay".into(),
            probe: Some(
                serde_json::json!({"kind":"video","width":1920,"height":1080,"duration_ms":1000}),
            ),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    let clip = |id: &str, asset: &str, transform: Option<ClipTransform>| {
        Clip::Media(MediaClip {
            id: id.into(),
            asset: asset.into(),
            src_in_ms: 0,
            src_out_ms: 1000,
            effects: vec![],
            gain_db: 0.0,
            transform,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
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
        })
    };
    p.track_mut("v1").unwrap().visible = false;
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(clip("c_base", "base", None));
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![clip(
            "c_overlay",
            "overlay",
            Some(ClipTransform {
                x: 0.25,
                y: 0.25,
                scale: 0.5,
                opacity: 1.0,
            }),
        )],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let edl = cut_core::edl_from_project(&p);

    let g = build_graph(
        &p,
        &edl,
        tmp.path(),
        false,
        false,
        true,
        RenderOptions::default(),
        None,
    )
    .unwrap();

    assert_eq!(g.inputs.len(), 1, "hidden base media must not be loaded");
    assert_eq!(g.inputs[0].path, overlay_media);
    assert!(
        g.filter
            .contains("color=c=black:s=1920x1080:r=30:d=1.000,format=yuv420p[v0];"),
        "hidden base should render a black canvas:\n{}",
        g.filter
    );
    assert!(
        g.filter
            .contains("[v0][o1_s0]overlay=0:0:eof_action=pass[vo1];"),
        "visible overlay must remain an overlay over the black base:\n{}",
        g.filter
    );
}

/// Mute/solo regression for the audio mix (`edit.mute` / `edit.solo`). A
/// non-audible track (muted, or soloed-OUT) is DROPPED from the mix → it
/// contributes SILENCE, and its gain is never touched. Proves the boolean rule
/// (Project::audio_track_audible) END-TO-END at the graph level: default both
/// play (amix=2), mute drops one (→ single-track, no amix), mute-all → no audio,
/// solo isolates one (→ no amix). The gain on every track stays 0 dB throughout
/// (the data-loss falsifier: the old mute would have written -100 dB into gain).
#[test]
fn audio_mix_honors_mute_and_solo() {
    use cut_core::{Asset, Clip, MediaClip, Project, ProjectSettings, Track, TrackKind};
    let tmp = tempfile::tempdir().unwrap();
    let media = tmp.path().join("x.mp4");
    std::fs::write(&media, b"stub").unwrap();
    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 1920,
            height: 1080,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        Asset {
            path: media.to_string_lossy().into_owned(),
            hash: "sha256:x".into(),
            probe: Some(serde_json::json!({
                "kind":"audio","duration_ms":3000,"has_audio":true
            })),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    let clip = |id: &str| {
        Clip::Media(MediaClip {
            id: id.into(),
            asset: "a1".into(),
            src_in_ms: 0,
            src_out_ms: 3000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
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
        })
    };
    // Two AUDIO tracks (default a1t + a2t), each with one clip of the same asset.
    p.track_mut("a1t").unwrap().clips.push(clip("c1"));
    p.tracks.push(Track {
        id: "a2t".into(),
        kind: TrackKind::Audio,
        clips: vec![clip("c2")],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let dir = std::path::Path::new("/tmp");
    // Audio-only graph (with_video=false, with_audio=true), the render_audio path.
    let build = |p: &Project| {
        let edl = cut_core::edl_from_project(p);
        build_graph(
            p,
            &edl,
            dir,
            false,
            true,
            false,
            RenderOptions::default(),
            None,
        )
        .unwrap()
    };

    // DEFAULT: both audio tracks are audible → amix across 2 inputs.
    let g = build(&p);
    assert!(
        g.filter.contains("amix=inputs=2"),
        "default → both tracks mix:\n{}",
        g.filter
    );
    assert!(g.audio_out.is_some());

    // MUTE a2t → only a1t remains: single-track mix (NO amix), audio still present.
    p.track_mut("a2t").unwrap().muted = true;
    let g = build(&p);
    assert!(
        !g.filter.contains("amix"),
        "muted track dropped → single-track mix, no amix:\n{}",
        g.filter
    );
    assert!(
        g.audio_out.is_some(),
        "the unmuted track still produces audio"
    );

    // MUTE BOTH → no audible track → no audio output at all (the mix is silence).
    p.track_mut("a1t").unwrap().muted = true;
    let g = build(&p);
    assert!(
        g.audio_out.is_none(),
        "all tracks muted → no audio in the mix:\n{}",
        g.filter
    );

    // Clear mutes, SOLO a1t → only a1t audible, a2t silenced (no amix).
    p.track_mut("a1t").unwrap().muted = false;
    p.track_mut("a2t").unwrap().muted = false;
    p.track_mut("a1t").unwrap().solo = true;
    let g = build(&p);
    assert!(
        !g.filter.contains("amix"),
        "solo isolates one track → no amix:\n{}",
        g.filter
    );
    assert!(g.audio_out.is_some(), "the soloed track plays");

    // The data-loss falsifier: every track's GAIN stayed 0 dB the whole time —
    // mute/solo are flags, they never overwrote the dialed level.
    for t in &p.tracks {
        assert_eq!(
            t.gain_db, 0.0,
            "gain must be untouched by mute/solo: {}",
            t.id
        );
    }

    // edit.pan at the graph level: center emits NO pan
    // filter (byte-identical mix); off-center emits the balance stage with the
    // cosine attenuation on the OPPOSITE channel only (never a boost).
    p.track_mut("a1t").unwrap().solo = false;
    let g = build(&p);
    assert!(
        !g.filter.contains("]pan=stereo"),
        "center pan must add nothing:\n{}",
        g.filter
    );
    p.track_mut("a2t").unwrap().pan = 1.0; // full right → left channel silent
    let g = build(&p);
    assert!(
        g.filter
            .contains("pan=stereo|c0=0.000000*c0|c1=1.000000*c1"),
        "full-right pan silences the left channel at unity right:\n{}",
        g.filter
    );
    p.track_mut("a2t").unwrap().pan = -0.5; // half left → right attenuated cos(π/4)
    let g = build(&p);
    assert!(
        g.filter
            .contains("pan=stereo|c0=1.000000*c0|c1=0.707107*c1"),
        "half-left pan attenuates the right channel by cos(45°):\n{}",
        g.filter
    );
    assert_eq!(
        p.track_mut("a2t").unwrap().gain_db,
        0.0,
        "pan never touches gain"
    );
}

/// edit.mute_range render falsifier: SOURCE-time mute ranges
/// land as post-speed `between(t,…)` volume gates; speed divides, reverse
/// mirrors; a range outside the visible window emits NOTHING (graphs stay
/// byte-identical when nothing is muted).
#[test]
fn mute_gate_lands_in_audio_graph() {
    use cut_core::{Asset, Clip, Project, ProjectSettings};
    let tmp = tempfile::tempdir().unwrap();
    let media = tmp.path().join("x.mp4");
    std::fs::write(&media, b"stub").unwrap();
    let mut p = Project::new("t", ProjectSettings::default());
    p.assets.insert(
        "a1".into(),
        Asset {
            path: media.to_string_lossy().into_owned(),
            hash: "sha256:x".into(),
            probe: Some(serde_json::json!({
                "kind":"audio","duration_ms":10000,"has_audio":true
            })),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    // src window [500, 3000): a mute at source [1000, 1500) sits 0.5s in.
    let mut mc = cut_core::edit::make_media_clip("c1", "a1", 500, 3000);
    mc.mute_ranges = vec![[1000, 1500]];
    p.track_mut("a1t").unwrap().clips.push(Clip::Media(mc));
    let dir = std::path::Path::new("/tmp");
    let build = |p: &Project| {
        let edl = cut_core::edl_from_project(p);
        build_graph(
            p,
            &edl,
            dir,
            false,
            true,
            false,
            RenderOptions::default(),
            None,
        )
        .unwrap()
    };

    let g = build(&p);
    assert!(
        g.filter
            .contains(",volume='if(between(t,0.500000,1.000000),0,1)':eval=frame"),
        "source [1000,1500) with src_in 500 → gate [0.5s,1.0s):\n{}",
        g.filter
    );

    // Speed 2× halves the output-time window.
    let set = |p: &mut Project, f: &dyn Fn(&mut cut_core::MediaClip)| match &mut p
        .track_mut("a1t")
        .unwrap()
        .clips[0]
    {
        Clip::Media(c) => f(c),
        _ => panic!(),
    };
    set(&mut p, &|c| c.speed = 2.0);
    let g = build(&p);
    assert!(
        g.filter
            .contains(",volume='if(between(t,0.250000,0.500000),0,1)':eval=frame"),
        "2x speed halves the gate window:\n{}",
        g.filter
    );

    // Reverse mirrors the window inside the visible range.
    set(&mut p, &|c| {
        c.speed = 1.0;
        c.reverse = true;
    });
    let g = build(&p);
    assert!(
        g.filter
            .contains(",volume='if(between(t,1.500000,2.000000),0,1)':eval=frame"),
        "reverse mirrors: src [1000,1500) in window [500,3000) -> out [1.5s,2.0s):\n{}",
        g.filter
    );

    // A range wholly outside the visible window emits no gate at all.
    set(&mut p, &|c| {
        c.reverse = false;
        c.mute_ranges = vec![[5000, 6000]];
    });
    let g = build(&p);
    assert!(
        !g.filter.contains("if(between"),
        "outside-window range must emit nothing:\n{}",
        g.filter
    );
}

/// Audio-only exports must not touch video-only alpha helpers. A masked
/// video clip normally bakes `cache/mask/*.png` and adds it as a parallel
/// ffmpeg input for `maskedmerge`; export.audio builds with `with_video=false`
/// and should keep only the source media input plus audio filters.
#[test]
fn audio_only_graph_skips_mask_alpha_inputs_and_bakes() {
    use cut_core::{Asset, Clip, ClipMask, MaskEffect, MaskShape, Project, ProjectSettings};

    let tmp = tempfile::tempdir().unwrap();
    let media = tmp.path().join("x.mp4");
    std::fs::write(&media, b"stub").unwrap();
    let mut p = Project::new(
        "audio-only-mask",
        ProjectSettings {
            width: 320,
            height: 180,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        Asset {
            path: media.to_string_lossy().into_owned(),
            hash: "sha256:x".into(),
            probe: Some(serde_json::json!({
                "kind":"video","width":320,"height":180,"duration_ms":3000,"has_audio":true
            })),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );

    let mut vclip = cut_core::edit::make_media_clip("v1c1", "a1", 0, 3000);
    vclip.mask = Some(ClipMask {
        shape: MaskShape::Rect,
        points: vec![[0.1, 0.1], [0.4, 0.4]],
        feather: 0.0,
        invert: false,
        effect: MaskEffect::Blur,
        strength: Some(8.0),
        range_ms: None,
        track: None,
        regions: Vec::new(),
    });
    let aclip = cut_core::edit::make_media_clip("a1c1", "a1", 0, 3000);
    p.track_mut("v1").unwrap().clips.push(Clip::Media(vclip));
    p.track_mut("a1t").unwrap().clips.push(Clip::Media(aclip));

    let edl = cut_core::edl_from_project(&p);
    let g = build_graph(
        &p,
        &edl,
        tmp.path(),
        false,
        true,
        false,
        RenderOptions::default(),
        None,
    )
    .unwrap();

    assert_eq!(g.video_out, "");
    assert!(g.audio_out.is_some(), "audio export still maps the audio");
    assert_eq!(
        g.inputs.len(),
        1,
        "audio-only graph should not add mask alpha inputs"
    );
    assert!(
        !g.filter.contains("maskedmerge"),
        "video mask filter leaked into audio-only graph:\n{}",
        g.filter
    );
    assert!(
        !tmp.path().join("cache").exists(),
        "audio-only graph must not bake video mask files"
    );
}

/// Static mask alpha is rendered at the graph's resolved output geometry,
/// not the project default geometry, so explicit/match-source renders keep
/// feather blur and cache keys aligned with the actual frame being composited.
#[test]
fn mask_alpha_inputs_use_resolved_output_geometry() {
    use cut_core::{Asset, Clip, ClipMask, MaskEffect, MaskShape, Project, ProjectSettings};

    let tmp = tempfile::tempdir().unwrap();
    let media = tmp.path().join("x.mp4");
    std::fs::write(&media, b"stub").unwrap();
    let mut p = Project::new(
        "mask-explicit-geometry",
        ProjectSettings {
            width: 320,
            height: 180,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        Asset {
            path: media.to_string_lossy().into_owned(),
            hash: "sha256:x".into(),
            probe: Some(serde_json::json!({
                "kind":"video","width":320,"height":180,"duration_ms":3000,"has_audio":true
            })),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );

    let mask = ClipMask {
        shape: MaskShape::Rect,
        points: vec![[0.1, 0.1], [0.4, 0.4]],
        feather: 0.05,
        invert: false,
        effect: MaskEffect::Blur,
        strength: Some(8.0),
        range_ms: None,
        track: None,
        regions: Vec::new(),
    };
    let mut vclip = cut_core::edit::make_media_clip("v1c1", "a1", 0, 3000);
    vclip.mask = Some(mask.clone());
    p.track_mut("v1").unwrap().clips.push(Clip::Media(vclip));

    let edl = cut_core::edl_from_project(&p);
    let opts = RenderOptions {
        fit: Fit::Cover,
        resolution: Resolution::Explicit {
            width: 1080,
            height: 1920,
        },
        loudness_target: None,
    };
    let g = build_graph(&p, &edl, tmp.path(), false, false, true, opts, None).unwrap();
    let expected = mask_alpha_path(tmp.path(), &mask, 1080, 1920);
    let project_size = mask_alpha_path(tmp.path(), &mask, 320, 180);

    assert!(
        g.inputs.iter().any(|input| input.path == expected),
        "mask input should use explicit output geometry"
    );
    assert!(
        expected.exists(),
        "explicit-size mask alpha should be baked"
    );
    assert!(
        !project_size.exists(),
        "project-size mask alpha should not be baked for explicit output"
    );
}

/// Overlay opacity (edit.transform.opacity): an overlay video track clip with
/// opacity<1 emits a `colorchannelmixer=aa=<o>` alpha scale on its segment and
/// composites via `overlay=0:0`; an opaque (1.0) overlay emits NO alpha filter
/// (no regression for the common fully-opaque PiP).
#[test]
fn overlay_opacity_emits_alpha_filter() {
    use cut_core::{Asset, Clip, MediaClip, Project, ProjectSettings, Track, TrackKind};
    let tmp = tempfile::tempdir().unwrap();
    let media = tmp.path().join("x.mp4");
    std::fs::write(&media, b"stub").unwrap();
    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 1920,
            height: 1080,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        Asset {
            path: media.to_string_lossy().into_owned(),
            hash: "sha256:x".into(),
            probe: Some(serde_json::json!({
                "kind":"video","width":1920,"height":1080,"duration_ms":3000,"has_audio":true
            })),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    let media_clip = |id: &str, transform: Option<cut_core::ClipTransform>| {
        Clip::Media(MediaClip {
            id: id.into(),
            asset: "a1".into(),
            src_in_ms: 0,
            src_out_ms: 3000,
            effects: vec![],
            gain_db: 0.0,
            transform,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
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
        })
    };
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(media_clip("c1", None));
    let pip = |opacity: f64| {
        Some(cut_core::ClipTransform {
            x: 0.5,
            y: 0.5,
            scale: 0.5,
            opacity,
        })
    };
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![media_clip("c2", pip(0.5))],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let edl = cut_core::edl_from_project(&p);
    let g = build_graph(
        &p,
        &edl,
        tmp.path(),
        true,
        true,
        true,
        RenderOptions::default(),
        None,
    )
    .unwrap();
    assert!(
        g.filter.contains("colorchannelmixer=aa=0.5"),
        "opacity 0.5 must scale the overlay alpha:\n{}",
        g.filter
    );
    assert!(
        g.filter.contains("overlay=0:0"),
        "overlay composited onto the base:\n{}",
        g.filter
    );
    // Opaque overlay → no alpha filter (geometry place stays, opacity drops out).
    p.tracks.last_mut().unwrap().clips = vec![media_clip("c2", pip(1.0))];
    let edl2 = cut_core::edl_from_project(&p);
    let g2 = build_graph(
        &p,
        &edl2,
        tmp.path(),
        true,
        true,
        true,
        RenderOptions::default(),
        None,
    )
    .unwrap();
    assert!(
        !g2.filter.contains("colorchannelmixer=aa="),
        "an opaque overlay emits no alpha filter:\n{}",
        g2.filter
    );
}

/// plan_scrub_frame eligibility + source mapping. A plain
/// base-track media segment with a proxy on disk is fast-path eligible and
/// maps the timeline position to src_in + offset; a caption burned in at the
/// position, an overlay PiP at the position, a gap, a missing proxy, and an
/// out-of-range time each fall back (return None) so the caller never shows
/// a wrong frame from the single-input seek. A cropped clip (the proxy-crop contract) now ALSO
/// stays on the fast path with its crop mapped onto the letterboxed proxy
/// grid (it only falls back when the asset lacks the source dims to map).
#[test]
fn plan_scrub_frame_eligibility_and_mapping() {
    use cut_core::{Asset, CaptionClip, Clip, ClipTransform, MediaClip, Project, ProjectSettings};
    // A real (tiny, empty-content) proxy file so the exists() check passes.
    let tmp = tempfile::tempdir().unwrap();
    let proxy_path = tmp.path().join("proxies").join("a1.mp4");
    std::fs::create_dir_all(proxy_path.parent().unwrap()).unwrap();
    std::fs::write(&proxy_path, b"not-a-real-mp4-but-exists").unwrap();

    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 1920,
            height: 1080,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        Asset {
            path: "/src/a1.mp4".into(),
            hash: "sha256:a1".into(),
            probe: Some(serde_json::json!({"kind":"video","width":3840,"height":2160})),
            transcript: None,
            perception: None,
            // proxy stored RELATIVE to project_dir (how the import writes it).
            proxy: Some("proxies/a1.mp4".into()),
            filmstrip: None,
        },
    );
    // Two base-track clips: c1 [0,4000) src [1000,5000); c2 [4000,7000).
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(Clip::Media(MediaClip {
            id: "c1".into(),
            asset: "a1".into(),
            src_in_ms: 1000,
            src_out_ms: 5000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
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
        }));
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(Clip::Media(MediaClip {
            id: "c2".into(),
            asset: "a1".into(),
            src_in_ms: 0,
            src_out_ms: 3000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
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
        }));
    let project_dir = tmp.path();
    let edl = cut_core::edl_from_project(&p);

    // Eligible: at_ms=2000 falls in c1 (timeline [0,4000)); src maps to
    // src_in(1000) + (2000-0) = 3000.
    let plan = plan_scrub_frame(&p, &edl, project_dir, 2000)
        .expect("plain base media segment with a proxy is fast-path eligible");
    assert_eq!(
        plan.src_pos_ms, 3000,
        "src_pos = src_in + offset into the segment"
    );
    assert_eq!(
        plan.proxy_path, proxy_path,
        "resolves the project-relative proxy"
    );

    // Eligible: at_ms=5000 falls in c2 (timeline [4000,7000)); src maps to
    // src_in(0) + (5000-4000) = 1000.
    let plan2 = plan_scrub_frame(&p, &edl, project_dir, 5000).expect("c2 eligible");
    assert_eq!(plan2.src_pos_ms, 1000);

    // Out of range → None.
    assert!(
        plan_scrub_frame(&p, &edl, project_dir, 99_000).is_none(),
        "past end → fallback"
    );

    // No proxy → None (graceful fallback to the composed path).
    let mut p_noproxy = p.clone();
    p_noproxy.assets.get_mut("a1").unwrap().proxy = None;
    let edl_np = cut_core::edl_from_project(&p_noproxy);
    assert!(
        plan_scrub_frame(&p_noproxy, &edl_np, project_dir, 2000).is_none(),
        "no proxy → composed fallback"
    );

    // A caption burned in at the position → None (scrub-fast omits captions).
    let mut p_cap = p.clone();
    p_cap.tracks.push(cut_core::Track {
        id: "cap1".into(),
        kind: cut_core::TrackKind::Caption,
        clips: vec![Clip::Caption(CaptionClip {
            id: "s1".into(),
            text: "hi".into(),
            style_ref: None,
            range_ms: [1800, 2200],
        })],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let edl_cap = cut_core::edl_from_project(&p_cap);
    assert!(
        plan_scrub_frame(&p_cap, &edl_cap, project_dir, 2000).is_none(),
        "caption at the position → composed fallback (captions not in scrub-fast)"
    );

    // An overlay (PiP) media segment at the position → None (can't composite
    // a single-input seek).
    let mut p_ov = p.clone();
    p_ov.tracks.push(cut_core::Track {
        id: "v2".into(),
        kind: cut_core::TrackKind::Video,
        clips: vec![Clip::Media(MediaClip {
            id: "ov1".into(),
            asset: "a1".into(),
            src_in_ms: 0,
            src_out_ms: 4000,
            effects: vec![],
            gain_db: 0.0,
            transform: Some(ClipTransform::identity()),
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
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
        })],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let edl_ov = cut_core::edl_from_project(&p_ov);
    assert!(
        plan_scrub_frame(&p_ov, &edl_ov, project_dir, 2000).is_none(),
        "overlay PiP at the position → composed fallback"
    );

    // A cropped base clip (the proxy-crop contract): the source-space crop now STAYS on the
    // fast path, mapped onto the letterboxed proxy grid (replaces the old
    // composed-fallback behavior). The asset's probe is 3840×2160 (4K, set
    // above), the proxy box is 960×540. Hand-derived expectation for the
    // regression OBS-bar crop {x:0, y:54, w:3840, h:2052}:
    //   f = min(960/3840, 540/2160) = min(0.25, 0.25) = 0.25 (4K is already
    //       16:9, so the proxy has NO letterbox → pad_x = pad_y = 0).
    //   px = even(0   + 0·0.25)    = even(0)       = 0
    //   py = even(0   + 54·0.25)   = even(13.5→14) = 14
    //   pw = even(3840·0.25)       = even(960)     = 960   (px+pw = 960 = PROXY_WIDTH, at edge, no clamp)
    //   ph = even(2052·0.25)       = even(513→512) = 512   (py+ph = 526 ≤ 540)
    // → proxy_crop = [0, 14, 960, 512].
    let mut p_crop = p.clone();
    if let Clip::Media(c) = &mut p_crop.track_mut("v1").unwrap().clips[0] {
        c.crop = Some(cut_core::ClipCrop {
            x: 0,
            y: 54,
            w: 3840,
            h: 2052,
        });
    }
    let edl_crop = cut_core::edl_from_project(&p_crop);
    let plan_crop = plan_scrub_frame(&p_crop, &edl_crop, project_dir, 2000).expect(
        "the proxy-crop contract: cropped clip now stays on the fast path (mapped proxy crop)",
    );
    assert_eq!(
        plan_crop.proxy_crop,
        Some([0, 14, 960, 512]),
        "source crop {{0,54,3840,2052}} maps to proxy [0,14,960,512] (f=0.25, no letterbox)"
    );
    // The mapped rect must sit inside the proxy frame and be even-aligned.
    let [cx, cy, cw, ch] = plan_crop.proxy_crop.unwrap();
    assert!(cx + cw <= crate::proxy::PROXY_WIDTH && cy + ch <= crate::proxy::PROXY_HEIGHT);
    assert!(cx % 2 == 0 && cy % 2 == 0 && cw % 2 == 0 && ch % 2 == 0);
    // src_pos mapping is unaffected by the crop (still src_in + offset).
    assert_eq!(
        plan_crop.src_pos_ms, 3000,
        "crop does not change the seek position"
    );

    // A cropped clip whose ASSET lacks a video probe → None: the mapping
    // needs source dims; without them we keep the composed fallback rather
    // than guess (correctness over speed for the rare unprobed case).
    let mut p_crop_np = p_crop.clone();
    p_crop_np.assets.get_mut("a1").unwrap().probe = None;
    let edl_crop_np = cut_core::edl_from_project(&p_crop_np);
    assert!(
        plan_scrub_frame(&p_crop_np, &edl_crop_np, project_dir, 2000).is_none(),
        "cropped clip with no probe geometry → composed fallback"
    );
}

/// edit.fade render lowering (the fade-edit contract): segment-local times, clamping,
/// kind/stream selection, alpha mode for overlays.
#[test]
fn fade_suffix_filters() {
    let f = |in_ms, out_ms, kind| cut_core::ClipFade {
        in_ms,
        out_ms,
        kind,
    };
    let both = cut_core::FadeKind::Both;
    // No fade → empty suffix (graphs stay minimal).
    assert_eq!(fade_suffix(None, 3000, true, false), "");
    // Video fade on the base: dip to black, out starts at dur-out.
    assert_eq!(
        fade_suffix(Some(&f(500, 1000, both)), 3000, true, false),
        ",fade=t=in:st=0:d=0.500,fade=t=out:st=2.000:d=1.000"
    );
    // Overlay mode fades alpha instead.
    assert_eq!(
        fade_suffix(Some(&f(500, 0, both)), 3000, true, true),
        ",fade=t=in:st=0:d=0.500:alpha=1"
    );
    // Audio side uses afade.
    assert_eq!(
        fade_suffix(Some(&f(0, 250, both)), 3000, false, false),
        ",afade=t=out:st=2.750:d=0.250"
    );
    // Kind selects the stream: a video-only fade emits nothing on audio
    // chains (and vice versa).
    assert_eq!(
        fade_suffix(
            Some(&f(500, 0, cut_core::FadeKind::Video)),
            3000,
            false,
            false
        ),
        ""
    );
    assert_eq!(
        fade_suffix(
            Some(&f(500, 0, cut_core::FadeKind::Audio)),
            3000,
            true,
            false
        ),
        ""
    );
    // Fade longer than the segment clamps (a trim shrank the clip after
    // the fade was set) — never an out-of-range filter.
    assert_eq!(
        fade_suffix(Some(&f(5000, 0, both)), 2000, true, false),
        ",fade=t=in:st=0:d=2.000"
    );
    // Fade-in and fade-out together must not exceed the segment duration.
    assert_eq!(
        fade_suffix(Some(&f(1500, 1500, both)), 2000, true, false),
        ",fade=t=in:st=0:d=1.500,fade=t=out:st=1.500:d=0.500"
    );
}

/// The preset registry: three tiers, default = standard, quality knobs
/// per the quality regression contract (draft=CRF18 medium / standard=CRF20 slow
/// / high=CRF17 slow + 256k audio). Receipt comparability depends on the
/// names staying stable — treat changes here as contract changes.
#[test]
fn preset_registry_tiers() {
    for name in PRESET_NAMES {
        let p = RenderPreset::named(name).expect("registered preset resolves");
        assert_eq!(&p.name, name, "name field matches registry key");
    }
    assert!(
        RenderPreset::named("h264_1080p30").is_none(),
        "legacy name retired"
    );
    assert!(RenderPreset::named("nope").is_none());
    assert_eq!(
        RenderPreset::default().name,
        "standard",
        "default tier is standard"
    );

    let draft = RenderPreset::named("draft").unwrap();
    assert_eq!(arg_after(&draft.video_args, "-crf"), Some("18"));
    assert_eq!(arg_after(&draft.video_args, "-preset"), Some("medium"));
    assert_eq!(arg_after(&draft.audio_args, "-b:a"), Some("192k"));

    let standard = RenderPreset::named("standard").unwrap();
    assert_eq!(arg_after(&standard.video_args, "-crf"), Some("20"));
    assert_eq!(arg_after(&standard.video_args, "-preset"), Some("slow"));

    let high = RenderPreset::named("high").unwrap();
    assert_eq!(arg_after(&high.video_args, "-crf"), Some("17"));
    assert_eq!(arg_after(&high.audio_args, "-b:a"), Some("256k"));
}

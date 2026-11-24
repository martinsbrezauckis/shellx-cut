//! Overlay placement. Rectangle width is saved; height follows the source.
use cut_core::ClipTransform;

pub(super) fn geometry(
    t: &ClipTransform,
    w: u32,
    h: u32,
    source: Option<(u32, u32)>,
) -> (u32, u32, u32, u32) {
    let ow = super::even_size_px(f64::from(w) * t.scale, w);
    let oh = if t.shape.is_rect() {
        let (sw, sh) = source.unwrap_or((w, h));
        super::even_size_px(f64::from(ow) * f64::from(sh) / f64::from(sw), u32::MAX)
    } else {
        ow
    };
    // Placement is top-left anchored like CSS. Overflow clips at the stage;
    // never move a saved rectangle or squash it to fit the output.
    let max_x = w;
    let max_y = h;
    (
        ow,
        oh,
        super::even_pos_px(f64::from(w) * t.x, max_x),
        super::even_pos_px(f64::from(h) * t.y, max_y),
    )
}

pub(super) fn place_filter(
    t: &ClipTransform,
    w: u32,
    h: u32,
    mask: &str,
    source: Option<(u32, u32)>,
) -> String {
    let (ow, oh, ox, oy) = geometry(t, w, h, source);
    // Pad before clipping overflow at the stage bounds, just like CSS.
    format!(
        ",scale={ow}:{oh},setsar=1{mask},pad={}:{}:{ox}:{oy}:color=black@0.0,crop={w}:{h}:0:0",
        w.max(ow + ox),
        h.max(oh + oy)
    )
}

pub(super) fn mask_filter_for(t: &ClipTransform, d: u32, tag: &str, keep_hdr: bool) -> String {
    if t.shape.is_rect() {
        return String::new();
    }
    let c = (f64::from(d) - 1.0) / 2.0;
    let r = match t.shape {
        cut_core::types::OverlayShape::Rounded => f64::from(d) * t.radius.unwrap_or(0.18),
        _ => c,
    }
    .min(c);
    // Signed distance to a rounded square (circle when radius = half width).
    let qx = format!("abs(X-{c})-{}", c - r);
    let qy = format!("abs(Y-{c})-{}", c - r);
    let dist = format!("hypot(max({qx},0),max({qy},0))+min(max({qx},{qy}),0)-{r}");
    let inside = format!("lte({dist},0)");
    let (ring, rgba) = if let Some(ring) = &t.ring {
        let hex = ring
            .colour
            .strip_prefix('#')
            .filter(|s| matches!(s.len(), 6 | 8) && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .unwrap_or("ffffff");
        let mut rgba = [255u8; 4];
        for (i, v) in rgba.iter_mut().enumerate().take(hex.len() / 2) {
            *v = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(255);
        }
        (
            format!("gt({dist},-{})", (f64::from(d) * ring.width).max(2.0)),
            rgba,
        )
    } else {
        ("0".into(), [0; 4])
    };
    let a = f64::from(rgba[3]) / 255.0;
    // Only the source alpha varies. Shape/ring geometry is evaluated on ONE
    // frame, then looped; no per-video-frame expression evaluator is involved.
    let still = |format: &str, filter: &str, label: &str| {
        format!(
        "color=c=black:s={d}x{d}:r=30,trim=end_frame=1,format={format},{filter},loop=loop=-1:size=1:start=0[{tag}_{label}];"
    )
    };
    let (ap, rp, gray, scale_max, yuva) = if keep_hdr {
        ("gbrap10le", "gbrp10le", "gray10le", 1023u32, "yuva420p10le")
    } else {
        ("gbrap", "gbrp", "gray", 255u32, "yuva420p")
    };
    let ch = |v: u8| u32::from(v) * scale_max / 255;
    let mut graph = format!(
        ",format={ap},split[{tag}_rgb][{tag}_alpha];\
         [{tag}_alpha]alphaextract[{tag}_a];"
    );
    graph.push_str(&still(
        gray,
        &format!("geq=lum='{inside}*{scale_max}'"),
        "shape",
    ));
    graph.push_str(&format!(
        "[{tag}_a][{tag}_shape]blend=all_mode=multiply:shortest=1[{tag}_masked_a];\
         [{tag}_rgb]format={rp}[{tag}_colour];"
    ));
    let colour = if t.ring.is_some() {
        graph.push_str(&still(
            gray,
            &format!("geq=lum='if({ring},{scale_max}*{a},0)'"),
            "ring",
        ));
        graph.push_str(&still(
            rp,
            &format!(
                "lutrgb=r={}:g={}:b={}",
                ch(rgba[0]),
                ch(rgba[1]),
                ch(rgba[2])
            ),
            "ink",
        ));
        graph.push_str(&format!(
            "[{tag}_colour][{tag}_ink][{tag}_ring]maskedmerge[{tag}_paint];"
        ));
        "paint"
    } else {
        "colour"
    };
    graph.push_str(&format!(
        "[{tag}_{colour}][{tag}_masked_a]alphamerge=shortest=1,format={yuva}"
    ));
    graph
}

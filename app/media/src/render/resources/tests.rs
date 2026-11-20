use super::*;
use serde_json::json;

fn s(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|x| x.to_string()).collect()
}

/// A project at `w`x`h`@60 whose v1 holds `cuts` = (src_in, src_out) of one
/// 8-bit 4:2:0 source of `sw`x`sh` at 60 fps.
fn project(w: u32, h: u32, sw: u64, sh: u64, cuts: &[(u64, u64)]) -> (Project, Edl) {
    let settings = cut_core::ProjectSettings {
        width: w,
        height: h,
        fps: 60.0,
        ..Default::default()
    };
    let mut p = Project::new("est", settings);
    p.assets.insert(
        "a1".into(),
        cut_core::Asset {
            path: "/x.mp4".into(),
            hash: "sha256:test".into(),
            probe: Some(json!({
                "width": sw, "height": sh, "fps": 60.0,
                "raw": {"streams": [{"codec_type": "video", "pix_fmt": "yuv420p"}]}
            })),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    p.track_mut("v1").unwrap().clips = cuts
        .iter()
        .enumerate()
        .map(|(i, (a, b))| {
            serde_json::from_value(
                json!({"id": format!("c{i}"), "asset": "a1", "src_in_ms": a, "src_out_ms": b}),
            )
            .unwrap()
        })
        .collect();
    let edl = cut_core::edl_from_project(&p);
    (p, edl)
}

const VT: &[&str] = &["-c:v", "h264_videotoolbox"];

/// The HQ record (F2): the 13.3 s 4K60 phone clip five times in a row was
/// estimated at 39 GB (37 GB held) and measured 10.2 GB. Now one copy.
#[test]
fn repeated_clip_counts_one_shared_copy() {
    let (p, e) = project(3840, 2160, 3840, 2160, &[(0, 13_313); 5]);
    let est = estimate_memory_for(&p, &e, 3840, 2160, &s(VT), 36);
    // 13.313 s x 60 frames x 12.4 MB = 9.3 GB (+5% padded buffers), never
    // four copies.
    assert!((9_800..10_100).contains(&est.held_frames_mb), "{est:?}");
    assert!(est.total_mb < 12_000, "{est:?}");
}

/// Five cuts of one clip in reverse order: the decoder passes four of them
/// while serving the first; they are distinct frames.
#[test]
fn reversed_parts_hold_what_the_decoder_passed() {
    let parts: Vec<(u64, u64)> = (0..5).rev().map(|i| (i * 2_000, (i + 1) * 2_000)).collect();
    let (p, e) = project(3840, 2160, 3840, 2160, &parts);
    let est = estimate_memory_for(&p, &e, 3840, 2160, &s(VT), 36);
    // 8 s x 60 frames x 12.4 MB (+5% padded buffers).
    assert!((5_900..6_100).contains(&est.held_frames_mb), "{est:?}");
}

/// A 4K source in a 720p project is scaled in every segment, so each waiting
/// segment holds its own 720p copy (four copies of 13.3 s).
#[test]
fn converting_chains_hold_a_copy_each_at_output_size() {
    let (p, e) = project(1280, 720, 3840, 2160, &[(0, 13_313); 5]);
    let est = estimate_memory_for(&p, &e, 1280, 720, &s(VT), 36);
    // 4 x 798.8 frames x 1.32 MB = 4.2 GB, +15% for padded pool buffers
    // (measured 4.65 GB held on the Studio).
    assert!((4_750..4_950).contains(&est.held_frames_mb), "{est:?}");
}

/// Five different sources one after another on one track: one decoder is
/// active, the waiting ones keep about a third of theirs (measured 20-29%).
#[test]
fn waiting_decoders_count_a_part() {
    let (mut p, _) = project(3840, 2160, 3840, 2160, &[(0, 13_313)]);
    let a1 = p.assets["a1"].clone();
    for i in 2..=5 {
        p.assets.insert(format!("a{i}"), a1.clone());
    }
    p.track_mut("v1").unwrap().clips = (1..=5)
        .map(|i| {
            serde_json::from_value(json!({"id": format!("c{i}"), "asset": format!("a{i}"), "src_in_ms": 0, "src_out_ms": 13_313}))
                .unwrap()
        })
        .collect();
    let e = cut_core::edl_from_project(&p);
    let est = estimate_memory_for(&p, &e, 3840, 2160, &s(VT), 36);
    // One 4K decoder with 16 threads is 491 MB; four waiting ones 35% each.
    assert_eq!(est.held_frames_mb, 0, "{est:?}");
    assert!((1_150..1_200).contains(&est.decode_mb), "{est:?}");
}

/// A segmented window's ffmpeg decodes a 4K source with the window's thread
/// share (36 on the Studio) instead of 16: measured +0.6 to 0.8 GB per window.
#[test]
fn window_decoders_use_the_window_thread_share() {
    let (p, e) = project(3840, 2160, 3840, 2160, &[(0, 5_000)]);
    let single = one_pass(&p, &e, 3840, 2160, &s(VT), 36, None);
    let window = one_pass(&p, &e, 3840, 2160, &s(VT), 36, Some(36));
    assert_eq!(single.decode_mb, 491, "{single:?}");
    assert_eq!(window.decode_mb, 1_029, "{window:?}");
    // An 8K source keeps its own 8 decoder threads in a window too.
    let (p, e) = project(7680, 4320, 7680, 4320, &[(0, 5_000)]);
    let w8 = one_pass(&p, &e, 7680, 4320, &s(VT), 36, Some(36));
    assert_eq!(
        w8.decode_mb,
        one_pass(&p, &e, 7680, 4320, &s(VT), 36, None).decode_mb
    );
}

/// The HQ preflight case: the 34.65 s 8K clip forty times (23 min) renders
/// segmented, one ffmpeg per window, so the need is one window's, not 3.7 TB.
#[test]
fn segmented_render_needs_its_largest_window() {
    let (p, e) = project(7680, 4320, 7680, 4320, &[(0, 34_651); 40]);
    let est = estimate_memory_for(&p, &e, 7680, 4320, &s(&["-c:v", "libx264"]), 36);
    assert!(est.windows.is_some_and(|n| n > 100), "{est:?}");
    // Encoder at 54 threads (24 GB) + one decoder + what a 2 s window can
    // hold at a seam: the next cut's frames, decoded before the window
    // reaches the end of the clip (at most one window: 120 frames, 6 GB);
    // far below the single pass's 103 GB of held frames.
    assert!(est.total_mb < 40_000, "{est:?}");
    assert!(est.held_frames_mb < 6_500, "{est:?}");
}

#[test]
fn encoder_threads_follow_the_pin_or_the_automatic_count() {
    let (p, e) = project(7680, 4320, 7680, 4320, &[(0, 12_000)]);
    let auto = estimate_memory_for(&p, &e, 7680, 4320, &s(&["-c:v", "libx264"]), 36);
    assert_eq!(auto.encoder_threads, Some(54));
    let pinned = estimate_memory_for(
        &p,
        &e,
        7680,
        4320,
        &s(&["-c:v", "libx264", "-threads", "12"]),
        36,
    );
    assert_eq!(pinned.encoder_threads, Some(12));
    // 33.2 Mpx x 11 B x 42 threads = 14.3 GB less.
    assert!(
        auto.total_mb - pinned.total_mb > 14_000,
        "{auto:?} {pinned:?}"
    );
    // `-threads 0` is ffmpeg's automatic count.
    let zero = estimate_memory_for(
        &p,
        &e,
        7680,
        4320,
        &s(&["-c:v", "libx264", "-threads", "0"]),
        36,
    );
    assert_eq!(zero, auto);
}

#[test]
fn encoder_thread_args_only_for_x264_without_explicit_threads() {
    let nvenc = s(&["-c:v", "hevc_nvenc"]);
    assert_eq!(
        encoder_thread_args(&nvenc, 7680, 4320),
        Vec::<String>::new()
    );
    let pinned = s(&["-c:v", "libx264", "-threads", "2"]);
    assert_eq!(
        encoder_thread_args(&pinned, 7680, 4320),
        Vec::<String>::new()
    );
}

#[test]
fn output_size_follows_bitrate_or_codec() {
    // 12 Mbps for 10 s + 192 kbps audio = about 14.5 MB.
    let mb = estimate_output_mb(
        10_000,
        3840,
        2160,
        60.0,
        &s(&["-c:v", "libx264", "-b:v", "12M"]),
        192,
    );
    assert!((14..=15).contains(&mb), "{mb}");
    let prores = estimate_output_mb(13_313, 3840, 2160, 58.6, &s(&["-c:v", "prores_ks"]), 1536);
    assert!(prores > 1500, "{prores}");
}

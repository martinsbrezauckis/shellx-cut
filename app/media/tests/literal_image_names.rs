//! Literal filenames containing FFmpeg `image2` printf/glob characters (`%03d`,
//! `%d`, `*`, `?`, `[`) must always read and write that exact single file,
//! never a neighbor sequence.

cut_core::isolate_test_process!();

use cut_core::{edl_from_project, Asset, Clip, ColorConfig, MediaClip, Project, ProjectSettings};
use cut_media::color_match::{auto_balance, sample_channel_stats, AutoBalanceMode};
use cut_media::ffmpeg::ffprobe_packet_counts;
use cut_media::ffmpeg::push_ffmpeg_literal_input;
use cut_media::ffmpeg::run_ffmpeg;
use cut_media::filmstrip::make_image_thumb;
use cut_media::poster::{make_poster, PosterKind};
use cut_media::render::extract_frame;
use cut_media::scopes::{measure as measure_scopes, render_scope, ScopeKind};
use cut_media::{
    probe, probe_source, render_final, render_preview_incremental, PathFence, RenderOptions,
    RenderPreset,
};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn direct_literal_input_uses_normalized_windows_path() {
    for (original, expected) in [
        (r"\\?\C:\Cut\clip.mp4", r"C:\Cut\clip.mp4"),
        (r"\\?\UNC\server\share\clip.mp4", r"\\server\share\clip.mp4"),
    ] {
        let mut command = Command::new("ffmpeg");
        let _ = push_ffmpeg_literal_input(&mut command, Path::new(original));
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(args.last().map(String::as_str), Some(expected));
    }
}

#[test]
fn ordinary_video_names_with_pattern_characters_remain_readable() {
    let dir = private_dir();
    for name in ["clip [1].mp4", "50% done.mov"] {
        let source = dir.path().join(name);
        make_mp4_fixture(&source, 16, 16, "#ff0000");
        assert_eq!(probe(&source).unwrap().kind, "video");
        run_ffmpeg(&[
            "-i".into(),
            source.display().to_string(),
            "-frames:v".into(),
            "1".into(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ])
        .unwrap();
    }
}

#[test]
fn unreadable_literal_images_do_not_decode_numbered_neighbours() {
    let dir = private_dir();
    for (name, neighbours, bytes) in [
        (
            "shot%03d.png",
            &["shot000.png", "shot001.png"][..],
            &b""[..],
        ),
        (
            "plain%d.png",
            &["plain0.png", "plain1.png"][..],
            &b"bad"[..],
        ),
        ("glob*.png", &["glob0.png", "glob1.png"][..], &b"bad"[..]),
        ("quest?.png", &["quest0.png", "quest1.png"][..], &b"bad"[..]),
        (
            "class[01].png",
            &["class0.png", "class1.png"][..],
            &b"bad"[..],
        ),
    ] {
        if cfg!(windows) && name.contains(['*', '?']) {
            continue; // not a legal file name on Windows
        }
        std::fs::write(dir.path().join(name), bytes).unwrap();
        for neighbour in neighbours {
            make_image_fixture(&dir.path().join(neighbour), 16, 16, "#ff0000");
        }
        let error = run_ffmpeg(&[
            "-i".into(),
            dir.path().join(name).display().to_string(),
            "-frames:v".into(),
            "1".into(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ])
        .expect_err("a corrupt literal must not decode its valid neighbours");
        assert!(
            error.code == "ffmpeg" || error.code == "unreadable_media",
            "{name}: {error}"
        );
    }
}

#[test]
fn failed_literal_format_probe_refuses_patterned_inputs() {
    let dir = private_dir();
    make_unreadable_patterned_fixtures(dir.path());
    assert_probe_failure_refused(dir.path(), &dir.path().join("missing-ffprobe"));
}

fn make_unreadable_patterned_fixtures(dir: &Path) {
    for (name, neighbours) in [
        ("shot%03d.png", &["shot000.png", "shot001.png"][..]),
        ("plain%d.png", &["plain0.png", "plain1.png"][..]),
        ("glob*.png", &["glob0.png", "glob1.png"][..]),
        ("quest?.png", &["quest0.png", "quest1.png"][..]),
        ("class[01].png", &["class0.png", "class1.png"][..]),
    ] {
        if cfg!(windows) && name.contains(['*', '?']) {
            continue; // not a legal file name on Windows
        }
        std::fs::write(dir.join(name), b"not an image").unwrap();
        for neighbour in neighbours {
            make_image_fixture(&dir.join(neighbour), 16, 16, "#ff0000");
        }
    }
}

fn assert_probe_failure_refused(dir: &Path, fake_probe: &Path) {
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "failed_literal_format_probe_child",
            "--nocapture",
        ])
        .env("CUT_LITERAL_PROBE_CHILD", dir)
        .env("SHELLX_CUT_FFPROBE", fake_probe)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn empty_format_answer_refuses_patterned_inputs() {
    use std::os::unix::fs::PermissionsExt;

    let dir = private_dir();
    make_unreadable_patterned_fixtures(dir.path());
    let fake_probe = dir.path().join("empty-ffprobe");
    std::fs::write(&fake_probe, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&fake_probe, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_probe_failure_refused(dir.path(), &fake_probe);
}

#[cfg(unix)]
#[test]
fn failed_format_answer_with_image2_text_still_refuses() {
    use std::os::unix::fs::PermissionsExt;

    let dir = private_dir();
    make_unreadable_patterned_fixtures(dir.path());
    let fake_probe = dir.path().join("failed-ffprobe");
    std::fs::write(&fake_probe, "#!/bin/sh\nprintf 'image2\\n'\nexit 1\n").unwrap();
    std::fs::set_permissions(&fake_probe, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_probe_failure_refused(dir.path(), &fake_probe);
}

#[cfg(unix)]
#[test]
fn oversized_format_answer_refuses_patterned_inputs() {
    use std::os::unix::fs::PermissionsExt;

    let dir = private_dir();
    make_unreadable_patterned_fixtures(dir.path());
    let fake_probe = dir.path().join("verbose-ffprobe");
    std::fs::write(
        &fake_probe,
        "#!/bin/sh\nprintf 'image2\\n'\ni=0\nwhile [ $i -lt 9000 ]; do printf x; i=$((i+1)); done\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake_probe, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_probe_failure_refused(dir.path(), &fake_probe);
}

#[test]
fn failed_literal_format_probe_child() {
    let Some(root) = std::env::var_os("CUT_LITERAL_PROBE_CHILD") else {
        return;
    };
    for name in [
        "shot%03d.png",
        "plain%d.png",
        "glob*.png",
        "quest?.png",
        "class[01].png",
    ] {
        if cfg!(windows) && name.contains(['*', '?']) {
            continue; // not a legal file name on Windows
        }
        let source = Path::new(&root).join(name);
        let error = run_ffmpeg(&[
            "-i".into(),
            source.display().to_string(),
            "-frames:v".into(),
            "1".into(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ])
        .expect_err("an unknown demuxer must never read a numbered neighbour");
        assert_eq!(error.code, "unreadable_media", "{name}: {error}");
        assert!(error.cause.contains("ffprobe"), "{name}: {error}");
        assert!(error.suggested_action.is_some(), "{name}: {error}");
    }
}

#[cfg(unix)]
#[test]
fn stalled_literal_format_probe_obeys_operation_deadline() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    let dir = private_dir();
    let source = dir.path().join("stall%03d.png");
    make_image_fixture(&source, 16, 16, "#ff0000");
    let fake_probe = dir.path().join("slow-ffprobe");
    std::fs::write(&fake_probe, "#!/bin/sh\nexec sleep 4\n").unwrap();
    std::fs::set_permissions(&fake_probe, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "stalled_literal_format_probe_child",
            "--nocapture",
        ])
        .env("CUT_LITERAL_STALL_CHILD", &source)
        .env("SHELLX_CUT_FFPROBE", &fake_probe)
        .spawn()
        .unwrap();
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "stalled-probe child failed");
            break;
        }
        if started.elapsed() > Duration::from_secs(2) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("format probe outlived the operation deadline");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(unix)]
#[test]
fn stalled_literal_format_probe_child() {
    use cut_media::ffmpeg::{with_render_process_control, RenderProcessControl};
    use std::time::Duration;

    let Some(source) = std::env::var_os("CUT_LITERAL_STALL_CHILD") else {
        return;
    };
    let control = RenderProcessControl::bounded(Duration::from_millis(200), || false);
    let error = with_render_process_control(&control, || {
        run_ffmpeg(&[
            "-i".into(),
            Path::new(&source).display().to_string(),
            "-frames:v".into(),
            "1".into(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ])
    })
    .expect_err("stalled format probe must stop at the deadline");
    assert!(error.to_string().contains("timed out"), "{error}");
    assert!(error.suggested_action.is_some(), "{error}");
}

fn private_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    dir
}

fn create_private_subdir(parent: &Path, name: &str) -> PathBuf {
    let dir = parent.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    dir
}

/// Generate a single solid-color image using `-update 1` so filenames with
/// `%03d` or `*` write literally as the test fixture rather than expanding.
fn make_image_fixture(path: &Path, w: u32, h: u32, color: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let out = Command::new(cut_media::ffmpeg::ffmpeg_bin())
        .args([
            "-hide_banner",
            "-nostdin",
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s={w}x{h}"),
            "-frames:v",
            "1",
            "-update",
            "1",
        ])
        .arg(path)
        .output()
        .expect("spawn ffmpeg to generate image fixture");
    assert!(
        out.status.success(),
        "fixture ffmpeg failed on {}: {}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn make_mp4_fixture(path: &Path, w: u32, h: u32, color: &str) {
    let out = Command::new(cut_media::ffmpeg::ffmpeg_bin())
        .args([
            "-hide_banner",
            "-nostdin",
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s={w}x{h}:r=25:d=0.4"),
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(path)
        .output()
        .expect("spawn ffmpeg to generate mp4 fixture");
    assert!(
        out.status.success(),
        "mp4 fixture ffmpeg failed on {}: {}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn make_wav_fixture(path: &Path) {
    let out = Command::new(cut_media::ffmpeg::ffmpeg_bin())
        .args([
            "-hide_banner",
            "-nostdin",
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=0.3",
        ])
        .arg(path)
        .output()
        .expect("spawn ffmpeg to generate wav fixture");
    assert!(
        out.status.success(),
        "wav fixture ffmpeg failed on {}: {}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Sample the average RGB triple of frame 0 of an image or video file by
/// copying its bytes to a plain temporary file without pattern characters so
/// readback itself is unaffected by pattern syntax.
fn mean_rgb_at_frame0(media_path: &Path) -> (u8, u8, u8) {
    let scratch = private_dir();
    let ext = media_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");
    let plain = scratch.path().join(format!("check.{ext}"));
    std::fs::copy(media_path, &plain)
        .unwrap_or_else(|e| panic!("expected rendered file at {}: {e}", media_path.display()));
    mean_rgb_from_plain(&plain)
}

fn mean_rgb_from_jpeg_bytes(bytes: &[u8]) -> (u8, u8, u8) {
    let scratch = private_dir();
    let plain = scratch.path().join("check.jpg");
    std::fs::write(&plain, bytes).unwrap();
    mean_rgb_from_plain(&plain)
}

fn mean_rgb_from_plain(plain: &Path) -> (u8, u8, u8) {
    let out = Command::new(cut_media::ffmpeg::ffmpeg_bin())
        .args(["-hide_banner", "-nostdin", "-v", "error", "-i"])
        .arg(plain)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .expect("spawn ffmpeg to read back RGB frame");
    assert!(
        out.status.success(),
        "readback failed for {}: {}",
        plain.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.len() >= 3 && out.stdout.len().is_multiple_of(3),
        "unexpected RGB byte count: {}",
        out.stdout.len()
    );
    let (pixels, _) = out.stdout.as_chunks::<3>();
    let n = pixels.len() as u64;
    let (mut sr, mut sg, mut sb) = (0u64, 0u64, 0u64);
    for px in pixels {
        sr += u64::from(px[0]);
        sg += u64::from(px[1]);
        sb += u64::from(px[2]);
    }
    ((sr / n) as u8, (sg / n) as u8, (sb / n) as u8)
}

fn assert_dominant_red((r, g, b): (u8, u8, u8), label: &str) {
    assert!(
        r > 180 && g < 70 && b < 70,
        "{label}: expected red frame (~255,0,0), got rgb=({r},{g},{b}) (likely read neighbor image instead of literal file)"
    );
}

fn still_project(asset_path: &Path, w: u32, h: u32, with_probe: bool) -> Project {
    let mut p = Project::new(
        "literal-image-test",
        ProjectSettings {
            width: w,
            height: h,
            fps: 25.0,
            audio_rate: 48_000,
            color: ColorConfig::default(),
        },
    );
    let probe_val = if with_probe {
        Some(json!({
            "kind": "image",
            "width": w,
            "height": h,
            "has_audio": false,
            "format": "image2",
            "raw": { "streams": [{ "codec_type": "video", "pix_fmt": "rgb24" }] }
        }))
    } else {
        None
    };
    p.assets.insert(
        "img".into(),
        Asset {
            path: asset_path.display().to_string(),
            hash: "sha256:literalimg".into(),
            probe: probe_val,
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    p.track_mut("v1").unwrap().clips = vec![Clip::Media(MediaClip {
        id: "c1".into(),
        asset: "img".into(),
        src_in_ms: 0,
        src_out_ms: 400,
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
    })];
    p
}

#[test]
fn probe_and_packet_counts_read_literal_printf_named_images_and_directories() {
    let dir = private_dir();

    // 1. Literal a%03d.png (64x36) beside a000.png / a001.png (128x72).
    let png_lit = dir.path().join("a%03d.png");
    make_image_fixture(&png_lit, 64, 36, "#ff0000");
    make_image_fixture(&dir.path().join("a000.png"), 128, 72, "#0000ff");
    make_image_fixture(&dir.path().join("a001.png"), 128, 72, "#00ff00");

    let probed = probe(&png_lit).expect("probe literal a%03d.png");
    assert_eq!(probed.kind, "image");
    assert_eq!((probed.width, probed.height), (Some(64), Some(36)));
    assert_eq!(probed.duration_ms, None);
    let sourced = probe_source(&png_lit).expect("probe_source literal a%03d.png");
    assert_eq!((sourced.width, sourced.height), (Some(64), Some(36)));
    assert_eq!(ffprobe_packet_counts(&png_lit).unwrap(), vec![(0, 1)]);

    // 2. Literal a%03d.jpg beside a000.jpg / a001.jpg.
    let jpg_lit = dir.path().join("a%03d.jpg");
    make_image_fixture(&jpg_lit, 64, 36, "#ff0000");
    make_image_fixture(&dir.path().join("a000.jpg"), 128, 72, "#0000ff");
    make_image_fixture(&dir.path().join("a001.jpg"), 128, 72, "#00ff00");
    let p_jpg = probe(&jpg_lit).expect("probe literal a%03d.jpg");
    assert_eq!((p_jpg.width, p_jpg.height), (Some(64), Some(36)));

    // 3. Literal p%d.png beside p0.png / p1.png.
    let pd_lit = dir.path().join("p%d.png");
    make_image_fixture(&pd_lit, 64, 36, "#ff0000");
    make_image_fixture(&dir.path().join("p0.png"), 128, 72, "#0000ff");
    make_image_fixture(&dir.path().join("p1.png"), 128, 72, "#00ff00");
    let p_pd = probe(&pd_lit).expect("probe literal p%d.png");
    assert_eq!((p_pd.width, p_pd.height), (Some(64), Some(36)));

    // 4. Pattern in directory component: dir%03d/card.png beside dir000/card.png.
    let dir_lit = dir.path().join("dir%03d/card.png");
    make_image_fixture(&dir_lit, 64, 36, "#ff0000");
    make_image_fixture(&dir.path().join("dir000/card.png"), 128, 72, "#0000ff");
    make_image_fixture(&dir.path().join("dir001/card.png"), 128, 72, "#00ff00");
    let p_dir = probe(&dir_lit).expect("probe literal dir%03d/card.png");
    assert_eq!((p_dir.width, p_dir.height), (Some(64), Some(36)));

    // 5. Missing missing%03d.png beside missing000.png must error, never read missing000.png.
    make_image_fixture(&dir.path().join("missing000.png"), 128, 72, "#0000ff");
    let missing_lit = dir.path().join("missing%03d.png");
    assert!(
        probe(&missing_lit).is_err(),
        "missing%03d.png must fail even when missing000.png exists"
    );
    assert!(probe_source(&missing_lit).is_err());

    // 6. Empty 0-byte empty%03d.png beside empty000.png must error, never read empty000.png.
    make_image_fixture(&dir.path().join("empty000.png"), 128, 72, "#0000ff");
    let empty_lit = dir.path().join("empty%03d.png");
    std::fs::write(&empty_lit, b"").unwrap();
    assert!(
        probe(&empty_lit).is_err(),
        "empty 0-byte empty%03d.png must fail even when empty000.png exists"
    );
    assert!(probe_source(&empty_lit).is_err());

    // 7. Lone lone%03d.png with NO neighbors must succeed as a literal file.
    let lone_dir = create_private_subdir(dir.path(), "lone");
    let lone_lit = lone_dir.join("lone%03d.png");
    make_image_fixture(&lone_lit, 64, 36, "#ff0000");
    let p_lone = probe(&lone_lit).expect("lone literal file with %03d must probe");
    assert_eq!((p_lone.width, p_lone.height), (Some(64), Some(36)));

    // 8. Symlink named link%03d.png -> target.png beside link000.png.
    #[cfg(unix)]
    {
        let target = dir.path().join("symlink_target.png");
        make_image_fixture(&target, 64, 36, "#ff0000");
        make_image_fixture(&dir.path().join("link000.png"), 128, 72, "#0000ff");
        let link_lit = dir.path().join("link%03d.png");
        std::os::unix::fs::symlink(&target, &link_lit).unwrap();
        let p_link = probe(&link_lit).expect("symlink link%03d.png must probe");
        assert_eq!((p_link.width, p_link.height), (Some(64), Some(36)));
    }
}

#[test]
fn probe_and_render_read_glob_characters_literally() {
    let dir = private_dir();
    let project_dir = create_private_subdir(dir.path(), "proj");
    let fence = PathFence::new(&project_dir).unwrap();
    let preset = RenderPreset::named("draft").unwrap();

    // Neighbors sorted before '*', '?', '[' in ASCII ('!' = 33 < '*' = 42 < '0' = 48 < '?' = 63 < '[' = 91).
    make_image_fixture(&project_dir.join("shot!.png"), 320, 180, "#0000ff");
    make_image_fixture(&project_dir.join("shot0.png"), 320, 180, "#0000ff");
    make_image_fixture(&project_dir.join("shotX.png"), 320, 180, "#0000ff");

    // `*` and `?` cannot be part of a file name on Windows; `[` can.
    let names: &[&str] = if cfg!(windows) {
        &["shot[0].png"]
    } else {
        &["shot*.png", "shot?.png", "shot[0].png"]
    };
    for (idx, name) in names.iter().enumerate() {
        let lit = project_dir.join(name);
        make_image_fixture(&lit, 160, 90, "#ff0000");

        let p = probe(&lit).unwrap_or_else(|e| panic!("probe {name}: {e}"));
        assert_eq!((p.width, p.height), (Some(160), Some(90)), "{name}");

        let project = still_project(&lit, 160, 90, true);
        let edl = edl_from_project(&project);
        let jpeg = extract_frame(&project, &edl, &project_dir, 0, Some(90))
            .unwrap_or_else(|e| panic!("extract_frame {name}: {e}"));
        assert_dominant_red(mean_rgb_from_jpeg_bytes(&jpeg), name);

        let out_mp4 = project_dir.join(format!("out_{idx}.mp4"));
        render_final(
            &project,
            &edl,
            &fence,
            &out_mp4,
            &preset,
            RenderOptions::default(),
            None,
        )
        .unwrap_or_else(|e| panic!("render_final {name}: {e}"));
        assert_dominant_red(mean_rgb_at_frame0(&out_mp4), name);
    }
}

#[test]
fn render_and_exact_frame_read_show_literal_printf_image_not_neighbor() {
    let dir = private_dir();
    let project_dir = create_private_subdir(dir.path(), "proj");
    let fence = PathFence::new(&project_dir).unwrap();
    let preset = RenderPreset::named("draft").unwrap();

    // Bright red literal file (160x90) alongside blue neighbors (320x180).
    let lit = project_dir.join("a%03d.png");
    make_image_fixture(&lit, 160, 90, "#ff0000");
    make_image_fixture(&project_dir.join("a000.png"), 320, 180, "#0000ff");
    make_image_fixture(&project_dir.join("a001.png"), 320, 180, "#00ff00");

    // Test both cached-still probe metadata AND unprobed asset paths, plus directory `%03d`.
    let dir_pct = create_private_subdir(&project_dir, "dir%03d");
    let dir_000 = create_private_subdir(&project_dir, "dir000");
    let dir_lit = dir_pct.join("slide.png");
    make_image_fixture(&dir_lit, 160, 90, "#ff0000");
    make_image_fixture(&dir_000.join("slide.png"), 320, 180, "#0000ff");

    for (tag, asset, with_probe) in [
        ("a%03d_with_probe", &lit, true),
        ("a%03d_unprobed", &lit, false),
        ("dir%03d_with_probe", &dir_lit, true),
    ] {
        let project = still_project(asset, 160, 90, with_probe);
        let edl = edl_from_project(&project);

        // 1. Exact-frame read (render::extract_frame)
        let jpeg = extract_frame(&project, &edl, &project_dir, 0, Some(90))
            .unwrap_or_else(|e| panic!("extract_frame {tag}: {e}"));
        assert_dominant_red(
            mean_rgb_from_jpeg_bytes(&jpeg),
            &format!("extract_frame {tag}"),
        );

        // 2. Final render (render_final -> graph_args -> -loop 1 -i ...)
        let out_mp4 = project_dir.join(format!("final_{tag}.mp4"));
        render_final(
            &project,
            &edl,
            &fence,
            &out_mp4,
            &preset,
            RenderOptions::default(),
            None,
        )
        .unwrap_or_else(|e| panic!("render_final {tag}: {e}"));
        assert_dominant_red(mean_rgb_at_frame0(&out_mp4), &format!("render_final {tag}"));

        // 3. Draft preview (render_preview_incremental -> draft_preview)
        let cache_dir = create_private_subdir(&project_dir, &format!("cache_{tag}"));
        let preview = render_preview_incremental(&project, &edl, &project_dir, &cache_dir, &preset)
            .unwrap_or_else(|e| panic!("render_preview_incremental {tag}: {e}"));
        assert_dominant_red(
            mean_rgb_at_frame0(&preview.path),
            &format!("render_preview_incremental {tag}"),
        );
    }
}

#[test]
fn thumbnail_poster_scopes_color_match_and_pct_output_dirs_use_literal_paths() {
    let dir = private_dir();
    let src_dir = create_private_subdir(dir.path(), "src");

    // Bright red literal file alongside bright blue neighbor sequence.
    let lit = src_dir.join("shot%03d.png");
    make_image_fixture(&lit, 160, 90, "#ff0000");
    make_image_fixture(&src_dir.join("shot000.png"), 320, 180, "#0000ff");
    make_image_fixture(&src_dir.join("shot001.png"), 320, 180, "#00ff00");

    // 1. make_image_thumb reads literal shot%03d.png (red), not shot000.png (blue).
    let thumb_dir = create_private_subdir(dir.path(), "thumbs");
    let thumb_path = make_image_thumb(&lit, &thumb_dir, "asset1").expect("make_image_thumb");
    assert_dominant_red(mean_rgb_at_frame0(&thumb_path), "make_image_thumb");

    // 2. make_poster reads literal shot%03d.png (red), not shot000.png (blue).
    let poster_out = dir.path().join("poster.jpg");
    make_poster(&lit, &poster_out, PosterKind::Image, 144).expect("make_poster");
    assert_dominant_red(mean_rgb_at_frame0(&poster_out), "make_poster");

    // 3. scopes::measure reads literal shot%03d.png (red: high Cr v_avg, low Cb u_avg).
    let sc = measure_scopes(&lit).expect("measure_scopes");
    assert!(
        sc.v_avg > sc.u_avg + 40.0,
        "scopes::measure expected red image (v_avg >> u_avg), got u_avg={} v_avg={}",
        sc.u_avg,
        sc.v_avg
    );

    // 4. color_match::sample_channel_stats + auto_balance read literal shot%03d.png (red).
    let stats = sample_channel_stats(&lit, 0.0).expect("sample_channel_stats");
    assert!(
        stats.mean_r > stats.mean_b + 100.0,
        "sample_channel_stats expected red image, got {:?}",
        stats
    );
    let balanced = auto_balance(&lit, 0.0, AutoBalanceMode::GrayWorld, 1.0).expect("auto_balance");
    assert!(
        balanced.stats.mean_r > balanced.stats.mean_b + 100.0,
        "auto_balance expected red image, got {:?}",
        balanced.stats
    );

    // 5. Single-image output paths inside a directory whose name contains `%03d`
    //    must write into `proj%03d.cutproj/...` without expanding `%03d` to `001`.
    let proj_pct = create_private_subdir(dir.path(), "proj%03d.cutproj");
    let proj_001 = create_private_subdir(dir.path(), "proj001.cutproj");
    let plain_red = dir.path().join("plain_red.png");
    make_image_fixture(&plain_red, 64, 36, "#ff0000");

    let out_thumb_dir = create_private_subdir(&proj_pct, "thumbs");
    let _out_thumb_001 = create_private_subdir(&proj_001, "thumbs");
    let pct_thumb =
        make_image_thumb(&plain_red, &out_thumb_dir, "t1").expect("make_image_thumb in %03d dir");
    assert!(
        pct_thumb.exists() && pct_thumb.starts_with(&out_thumb_dir),
        "thumb must be written to literal %03d dir: {}",
        pct_thumb.display()
    );
    assert!(!proj_001.join("thumbs/t1.jpg").exists());

    let pct_poster = proj_pct.join("poster.jpg");
    make_poster(&plain_red, &pct_poster, PosterKind::Image, 144).expect("make_poster in %03d dir");
    assert!(pct_poster.exists());
    assert!(!proj_001.join("poster.jpg").exists());

    let pct_scope = proj_pct.join("scope.jpg");
    render_scope(&plain_red, ScopeKind::Histogram, &pct_scope).expect("render_scope in %03d dir");
    assert!(pct_scope.exists());
    assert!(!proj_001.join("scope.jpg").exists());
}

#[test]
fn non_image2_files_with_percent_or_brackets_still_work() {
    let dir = private_dir();
    let project_dir = create_private_subdir(dir.path(), "proj");
    let fence = PathFence::new(&project_dir).unwrap();
    let preset = RenderPreset::named("draft").unwrap();

    // PNG/MP4/WAV files with `[` or `%%` or `%03d` where ffmpeg selects png_pipe / mov / wav.
    let bracket_png = project_dir.join("Screenshot [1].png");
    make_image_fixture(&bracket_png, 160, 90, "#ff0000");
    let pct_png = project_dir.join("100%%.png");
    make_image_fixture(&pct_png, 160, 90, "#ff0000");
    let clip_mp4 = project_dir.join("clip%03d.mp4");
    make_mp4_fixture(&clip_mp4, 160, 90, "#ff0000");
    let tone_wav = project_dir.join("tone%03d.wav");
    make_wav_fixture(&tone_wav);

    assert_eq!(
        (
            probe(&bracket_png).unwrap().width,
            probe(&bracket_png).unwrap().height
        ),
        (Some(160), Some(90))
    );
    assert_eq!(
        (
            probe(&pct_png).unwrap().width,
            probe(&pct_png).unwrap().height
        ),
        (Some(160), Some(90))
    );
    assert_eq!(probe(&clip_mp4).unwrap().kind, "video");
    assert_eq!(probe(&tone_wav).unwrap().kind, "audio");

    for (idx, asset) in [&bracket_png, &pct_png].iter().enumerate() {
        let project = still_project(asset, 160, 90, true);
        let edl = edl_from_project(&project);
        let out_mp4 = project_dir.join(format!("non_img2_{idx}.mp4"));
        render_final(
            &project,
            &edl,
            &fence,
            &out_mp4,
            &preset,
            RenderOptions::default(),
            None,
        )
        .unwrap();
        assert_dominant_red(mean_rgb_at_frame0(&out_mp4), &asset.display().to_string());
    }

    // Also render clip%03d.mp4 through render_final without Option pattern_type not found.
    let project = still_project(&clip_mp4, 160, 90, false);
    let edl = edl_from_project(&project);
    let out_mp4 = project_dir.join("clip_out.mp4");
    render_final(
        &project,
        &edl,
        &fence,
        &out_mp4,
        &preset,
        RenderOptions::default(),
        None,
    )
    .unwrap();
    assert_dominant_red(mean_rgb_at_frame0(&out_mp4), "clip%03d.mp4");
}

#[test]
fn edge_cases_and_concurrent_literal_reads_stay_isolated() {
    let dir = private_dir();
    let project_dir = create_private_subdir(dir.path(), "concurrent%03d");
    let fence = PathFence::new(&project_dir).unwrap();
    let preset = RenderPreset::named("draft").unwrap();

    // Non-image text payload disguised as corrupt%03d.png beside a valid corrupt000.png
    // must be rejected by both probe and probe_source rather than falling back to corrupt000.png.
    let corrupt = project_dir.join("corrupt%03d.png");
    std::fs::write(&corrupt, b"not an image").unwrap();
    make_image_fixture(&project_dir.join("corrupt000.png"), 128, 72, "#0000ff");
    assert!(probe(&corrupt).is_err());
    assert!(probe_source(&corrupt).is_err());

    // printf-like non-%d specifiers (%s%n) and large HD (1920x1080) still work literally.
    let weird = project_dir.join("tricky%s%n.png");
    make_image_fixture(&weird, 160, 90, "#ff0000");
    assert_eq!(
        (probe(&weird).unwrap().width, probe(&weird).unwrap().height),
        (Some(160), Some(90))
    );

    let hd = project_dir.join("hd%03d.png");
    make_image_fixture(&hd, 1920, 1080, "#ff0000");
    make_image_fixture(&project_dir.join("hd000.png"), 160, 90, "#0000ff");
    let hd_probe = probe(&hd).expect("probe 1920x1080 literal %03d");
    assert_eq!((hd_probe.width, hd_probe.height), (Some(1920), Some(1080)));

    // Run concurrent probes, thumbnail generation, and renders in the same %03d folder.
    let red = project_dir.join("par%03d.png");
    make_image_fixture(&red, 160, 90, "#ff0000");
    make_image_fixture(&project_dir.join("par000.png"), 320, 180, "#0000ff");
    std::thread::scope(|scope| {
        for worker in 0..4 {
            let red = &red;
            let project_dir = &project_dir;
            let fence = &fence;
            let preset = &preset;
            scope.spawn(move || {
                let p = probe(red).unwrap();
                assert_eq!((p.width, p.height), (Some(160), Some(90)));
                let thumb = make_image_thumb(red, project_dir, &format!("t{worker}")).unwrap();
                assert_dominant_red(mean_rgb_at_frame0(&thumb), "concurrent thumb");
                let project = still_project(red, 160, 90, true);
                let edl = edl_from_project(&project);
                let out = project_dir.join(format!("par_out_{worker}.mp4"));
                render_final(
                    &project,
                    &edl,
                    fence,
                    &out,
                    preset,
                    RenderOptions::default(),
                    None,
                )
                .unwrap();
                assert_dominant_red(mean_rgb_at_frame0(&out), "concurrent render_final");
            });
        }
    });
}

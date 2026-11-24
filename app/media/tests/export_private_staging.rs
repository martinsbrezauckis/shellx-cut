//! Red-first security and correctness tests for private export staging and
//! parent-pinned atomic publication in `cut-media`. The scenarios use Unix
//! links, modes and a shell FFmpeg wrapper; Windows is covered by the unit tests.
#![cfg(unix)]

use cut_core::{edl_from_project, Clip, ColorConfig, MediaClip, Project, ProjectSettings};
use cut_media::ffmpeg::{run_ffmpeg, run_ffmpeg_atomic_output};
use cut_media::gif::make_gif;
use cut_media::toolpath::{find_on_path, refresh_media_tools, ENV_FFMPEG};
use cut_media::{render_final, PathFence, RenderOptions, RenderPreset};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

cut_core::isolate_test_process!();

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct FfmpegWrapperGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    _dir: tempfile::TempDir,
    prev_ffmpeg: Option<OsString>,
    prev_envs: Vec<(&'static str, Option<OsString>)>,
}

impl FfmpegWrapperGuard {
    fn acquire_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn install_locked(
        lock: std::sync::MutexGuard<'static, ()>,
        extra_env: &[(&'static str, &str)],
    ) -> (Self, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let real = resolve_real_ffmpeg();
        let wrapper = dir.path().join("ffmpeg");
        let log_path = dir.path().join("ffmpeg-outputs.log");
        let script = r#"#!/bin/sh
set -eu
last=""
for arg in "$@"; do
  last="$arg"
done
case "$last" in
  *.mp4|*.mov|*.webm|*.gif|*.mp3|*.wav|*.jpg|*.png)
    if [ -n "${CUT_TEST_FFMPEG_LOG:-}" ]; then
      parent=$(dirname "$last")
      mode=$(stat -c '%a' "$parent" 2>/dev/null || stat -f '%Lp' "$parent" 2>/dev/null || echo unknown)
      printf '%s\t%s\t%s\n' "$parent" "$mode" "$last" >> "$CUT_TEST_FFMPEG_LOG"
    fi
    if [ -n "${CUT_TEST_SWAP_TARGET:-}" ] && [ ! -e "${CUT_TEST_SWAP_BACKUP:-}" ] && [ ! -L "${CUT_TEST_SWAP_TARGET:-}" ]; then
      mv "$CUT_TEST_SWAP_TARGET" "$CUT_TEST_SWAP_BACKUP"
      ln -s "$CUT_TEST_SWAP_EVIL" "$CUT_TEST_SWAP_TARGET"
    fi
    if [ "${CUT_TEST_FAIL_PARTIAL:-}" = "1" ]; then
      printf 'CORRUPT_PARTIAL_OUTPUT' > "$last"
      exit 1
    fi
    ;;
esac
exec "$CUT_TEST_REAL_FFMPEG" "$@"
"#;
        std::fs::write(&wrapper, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let prev_ffmpeg = std::env::var_os(ENV_FFMPEG);
        let mut prev_envs = vec![
            (
                "CUT_TEST_REAL_FFMPEG",
                std::env::var_os("CUT_TEST_REAL_FFMPEG"),
            ),
            (
                "CUT_TEST_FFMPEG_LOG",
                std::env::var_os("CUT_TEST_FFMPEG_LOG"),
            ),
            (
                "CUT_TEST_SWAP_TARGET",
                std::env::var_os("CUT_TEST_SWAP_TARGET"),
            ),
            (
                "CUT_TEST_SWAP_BACKUP",
                std::env::var_os("CUT_TEST_SWAP_BACKUP"),
            ),
            ("CUT_TEST_SWAP_EVIL", std::env::var_os("CUT_TEST_SWAP_EVIL")),
            (
                "CUT_TEST_FAIL_PARTIAL",
                std::env::var_os("CUT_TEST_FAIL_PARTIAL"),
            ),
        ];
        std::env::set_var("CUT_TEST_REAL_FFMPEG", &real);
        std::env::set_var("CUT_TEST_FFMPEG_LOG", &log_path);
        std::env::remove_var("CUT_TEST_SWAP_TARGET");
        std::env::remove_var("CUT_TEST_SWAP_BACKUP");
        std::env::remove_var("CUT_TEST_SWAP_EVIL");
        std::env::remove_var("CUT_TEST_FAIL_PARTIAL");
        for (k, v) in extra_env {
            prev_envs.push((k, std::env::var_os(k)));
            std::env::set_var(k, v);
        }
        std::env::set_var(ENV_FFMPEG, &wrapper);
        refresh_media_tools();
        (
            Self {
                _lock: lock,
                _dir: dir,
                prev_ffmpeg,
                prev_envs,
            },
            log_path,
        )
    }
}

impl Drop for FfmpegWrapperGuard {
    fn drop(&mut self) {
        match self.prev_ffmpeg.take() {
            Some(v) => std::env::set_var(ENV_FFMPEG, v),
            None => std::env::remove_var(ENV_FFMPEG),
        }
        for (k, v) in self.prev_envs.drain(..).rev() {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
        refresh_media_tools();
    }
}

fn resolve_real_ffmpeg() -> PathBuf {
    let bin = cut_media::ffmpeg::ffmpeg_bin();
    let path = PathBuf::from(&bin);
    if path.is_absolute() && path.is_file() {
        return path;
    }
    find_on_path("ffmpeg").expect("ffmpeg binary must be available for media tests")
}

fn create_dir_with_mode(path: &Path, mode: u32) {
    std::fs::create_dir_all(path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }
}

fn make_test_clip(path: &Path) {
    let args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=160x120:rate=15:duration=1",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=1",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-shortest",
        &path.display().to_string(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    run_ffmpeg(&args).expect("generate test clip");
}

fn one_clip_project(asset_path: &Path) -> Project {
    let mut p = Project::new(
        "staging-test",
        ProjectSettings {
            width: 160,
            height: 120,
            fps: 15.0,
            audio_rate: 48_000,
            color: ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        cut_core::Asset {
            path: asset_path.display().to_string(),
            hash: "sha256:staging".into(),
            probe: None,
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    let mk = |id: &str| {
        Clip::Media(MediaClip {
            id: id.into(),
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
            markers: vec![],
        })
    };
    p.track_mut("v1").unwrap().clips = vec![mk("c1")];
    p.track_mut("a1t").unwrap().clips = vec![mk("c1a")];
    p
}

fn dir_entries(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn cut_data_staging_dir() -> PathBuf {
    let home = std::env::var_os("SHELLX_CUT_HOME")
        .map(PathBuf::from)
        .expect("isolated SHELLX_CUT_HOME");
    home.join("export-staging")
}

#[cfg(unix)]
#[test]
fn ffmpeg_writes_only_into_private_data_folder_never_into_export_folder() {
    let lock = FfmpegWrapperGuard::acquire_lock();
    let root = tempfile::tempdir().unwrap();
    create_dir_with_mode(root.path(), 0o700);
    let src = root.path().join("src.mp4");
    make_test_clip(&src);

    let export_dir = root.path().join("group-export");
    create_dir_with_mode(&export_dir, 0o775);

    let (_guard, log_path) = FfmpegWrapperGuard::install_locked(lock, &[]);
    let cut_home = PathBuf::from(std::env::var_os("SHELLX_CUT_HOME").unwrap());

    let atomic_out = export_dir.join("atomic.mp4");
    let raw_args = vec![
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        "color=size=64x64:duration=0.2".into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "ultrafast".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        atomic_out.display().to_string(),
    ];
    run_ffmpeg_atomic_output(&raw_args, &atomic_out).expect("atomic output");

    let p = one_clip_project(&src);
    let edl = edl_from_project(&p);
    let fence = PathFence::new(&export_dir).unwrap();
    let preset = RenderPreset::named("draft").unwrap();
    let opts = RenderOptions::default();

    render_final(
        &p,
        &edl,
        &fence,
        Path::new("final.mp4"),
        &preset,
        opts,
        None,
    )
    .expect("render_final");

    cut_media::render::render_range(
        &p,
        &edl,
        &fence,
        Path::new("range.mp4"),
        &preset,
        [0, 500],
        opts,
        None,
    )
    .expect("render_range");

    let gif_out = export_dir.join("clip.gif");
    make_gif(&atomic_out, &gif_out, 10, 80, "floyd").expect("make_gif");

    let log = std::fs::read_to_string(&log_path).expect("ffmpeg log written");
    let lines: Vec<&str> = log.lines().collect();
    assert!(
        lines.len() >= 4,
        "expected at least 4 ffmpeg export writes, got {lines:?}"
    );
    for line in &lines {
        let mut cols = line.split('\t');
        let parent = Path::new(cols.next().unwrap());
        let mode = cols.next().unwrap();
        let target = cols.next().unwrap();
        assert!(
            !parent.starts_with(&export_dir),
            "ffmpeg must NOT stage or write inside export dir ({target}, parent={}, mode={mode})",
            parent.display()
        );
        assert!(
            parent.starts_with(&cut_home),
            "ffmpeg staging must live inside Cut's own data dir {}: got {target}",
            cut_home.display()
        );
        assert_eq!(
            mode, "700",
            "staging parent dir must be mode 0700 (current user only): {line}"
        );
    }

    for name in ["atomic.mp4", "final.mp4", "range.mp4", "clip.gif"] {
        let p = export_dir.join(name);
        assert!(p.is_file(), "final file missing: {}", p.display());
        assert!(
            std::fs::metadata(&p).unwrap().len() > 0,
            "final file empty: {}",
            p.display()
        );
    }
    let leftover = dir_entries(&cut_data_staging_dir());
    assert!(
        leftover.is_empty(),
        "staging folder must be clean after success: {leftover:?}"
    );
}

#[cfg(unix)]
#[test]
fn symlinked_parent_and_parent_rename_cannot_redirect_export() {
    let lock = FfmpegWrapperGuard::acquire_lock();
    let root = tempfile::tempdir().unwrap();
    create_dir_with_mode(root.path(), 0o700);
    let src = root.path().join("src.mp4");
    make_test_clip(&src);

    // Case A: a link the person made above the export folder (a linked Videos
    // folder) keeps working as in 0.6.121: the file lands in the real folder
    // only, never beside the link and never in the export folder as a temp.
    let evil_dir = root.path().join("evil-redirect");
    create_dir_with_mode(&evil_dir.join("sub"), 0o700);
    let link_parent = root.path().join("link-parent");
    std::os::unix::fs::symlink(&evil_dir, &link_parent).unwrap();
    let linked_out = link_parent.join("sub").join("redirected.mp4");

    let (_guard, _) = FfmpegWrapperGuard::install_locked(lock, &[]);
    let args = vec![
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        "color=size=64x64:duration=0.2".into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "ultrafast".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        linked_out.display().to_string(),
    ];
    let res = run_ffmpeg_atomic_output(&args, &linked_out);
    assert!(
        res.is_ok(),
        "own linked ancestor must still export: {res:?}"
    );
    assert_eq!(
        dir_entries(&evil_dir.join("sub")),
        vec!["redirected.mp4".to_string()],
        "the export must land in the real folder only"
    );

    // An export folder that is itself a link stays refused (as in 0.6.121).
    let final_link = root.path().join("final-link");
    std::os::unix::fs::symlink(evil_dir.join("sub"), &final_link).unwrap();
    let final_link_out = final_link.join("via-final-link.mp4");
    let mut final_link_args = args.clone();
    *final_link_args.last_mut().unwrap() = final_link_out.display().to_string();
    let final_link_res = run_ffmpeg_atomic_output(&final_link_args, &final_link_out);
    assert!(
        final_link_res.is_err(),
        "an export folder that is a link must be refused: {final_link_res:?}"
    );
    assert_eq!(
        dir_entries(&evil_dir.join("sub")),
        vec!["redirected.mp4".to_string()],
        "a refused export leaves nothing in the folder"
    );

    // Also render_range under a PathFence containing a symlinked sub-directory
    let fenced_root = root.path().join("fenced");
    create_dir_with_mode(&fenced_root, 0o700);
    std::os::unix::fs::symlink(&evil_dir, fenced_root.join("sub_link")).unwrap();
    let p = one_clip_project(&src);
    let edl = edl_from_project(&p);
    let fence = PathFence::new(&fenced_root).unwrap();
    let preset = RenderPreset::named("draft").unwrap();
    let opts = RenderOptions::default();
    let range_res = cut_media::render::render_range(
        &p,
        &edl,
        &fence,
        Path::new("sub_link/sub/range.mp4"),
        &preset,
        [0, 500],
        opts,
        None,
    );
    assert!(
        range_res.is_err(),
        "render_range must refuse symlinked parent: {range_res:?}"
    );
    assert!(
        !evil_dir.join("sub").join("range.mp4").exists(),
        "render_range must not write through symlinked parent"
    );

    // A project-relative path through a link inside the project: the fence
    // resolves it and the file lands in the real folder only (the server
    // refuses links in relative paths before this, as in 0.6.121).
    let inside_target = fenced_root.join("inside_target");
    create_dir_with_mode(&inside_target.join("sub"), 0o700);
    std::os::unix::fs::symlink(&inside_target, fenced_root.join("inside_link")).unwrap();
    let intra_res = cut_media::render::render_range(
        &p,
        &edl,
        &fence,
        Path::new("inside_link/sub/intra.mp4"),
        &preset,
        [0, 500],
        opts,
        None,
    );
    assert!(
        intra_res.is_ok(),
        "intra-fence link must export: {intra_res:?}"
    );
    // An ABSOLUTE path through the person's own link keeps working as in
    // 0.6.121: the fence resolves it and the file lands in the real folder only.
    let abs_res = cut_media::render::render_range(
        &p,
        &edl,
        &fence,
        &fenced_root.join("inside_link").join("sub").join("abs.mp4"),
        &preset,
        [0, 500],
        opts,
        None,
    );
    assert!(
        abs_res.is_ok(),
        "absolute linked path must still export: {abs_res:?}"
    );
    let mut landed = dir_entries(&inside_target.join("sub"));
    landed.sort();
    assert_eq!(
        landed,
        vec!["abs.mp4".to_string(), "intra.mp4".to_string()],
        "linked paths must land in the real folder only"
    );
    assert!(
        dir_entries(&fenced_root)
            .iter()
            .all(|n| n == "inside_target" || n == "inside_link" || n == "sub_link"),
        "no temporary file may appear in the fenced folder"
    );
    assert!(
        dir_entries(&cut_data_staging_dir()).is_empty(),
        "staging must be empty after the exports"
    );
    drop(_guard);

    let lock = FfmpegWrapperGuard::acquire_lock();
    // Case B: mid-flight rename + symlink swap of parent directory during ffmpeg execution
    let race_holder = root.path().join("race-holder");
    let real_parent = race_holder.join("team-dir");
    let real_exports = real_parent.join("exports");
    create_dir_with_mode(&real_exports, 0o775);
    let moved_backup = race_holder.join("team-dir.moved");
    let race_evil = root.path().join("race-evil");
    create_dir_with_mode(&race_evil.join("exports"), 0o775);

    let swap_target_s = real_parent.display().to_string();
    let swap_backup_s = moved_backup.display().to_string();
    let swap_evil_s = race_evil.display().to_string();
    let (_swap_guard, _) = FfmpegWrapperGuard::install_locked(
        lock,
        &[
            ("CUT_TEST_SWAP_TARGET", &swap_target_s),
            ("CUT_TEST_SWAP_BACKUP", &swap_backup_s),
            ("CUT_TEST_SWAP_EVIL", &swap_evil_s),
        ],
    );
    let race_out = real_exports.join("raced.mp4");
    let race_args = vec![
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        "color=size=64x64:duration=0.2".into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "ultrafast".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        race_out.display().to_string(),
    ];
    let race_res = run_ffmpeg_atomic_output(&race_args, &race_out);
    assert!(
        !race_evil.join("exports").join("raced.mp4").exists()
            && dir_entries(&race_evil.join("exports")).is_empty(),
        "mid-flight parent rename + symlink must NEVER write to redirected folder (res={race_res:?})"
    );
    let _ = race_res;
    let leftover = dir_entries(&cut_data_staging_dir());
    assert!(
        leftover.is_empty(),
        "staging must clean up even after parent swap refusal: {leftover:?}"
    );
}

#[cfg(unix)]
#[test]
fn failed_render_leaves_no_partial_file_in_export_or_staging() {
    let lock = FfmpegWrapperGuard::acquire_lock();
    let root = tempfile::tempdir().unwrap();
    create_dir_with_mode(root.path(), 0o700);
    let src = root.path().join("src.mp4");
    make_test_clip(&src);
    let dummy_gif_src = root.path().join("gif_src.mp4");
    std::fs::copy(&src, &dummy_gif_src).unwrap();

    let export_dir = root.path().join("exports");
    create_dir_with_mode(&export_dir, 0o755);

    let (_guard, _) = FfmpegWrapperGuard::install_locked(lock, &[("CUT_TEST_FAIL_PARTIAL", "1")]);

    let atomic_out = export_dir.join("failed_atomic.mp4");
    let raw_args = vec!["-f".into(), "mp4".into(), atomic_out.display().to_string()];
    assert!(run_ffmpeg_atomic_output(&raw_args, &atomic_out).is_err());

    let p = one_clip_project(&src);
    let edl = edl_from_project(&p);
    let fence = PathFence::new(&export_dir).unwrap();
    let preset = RenderPreset::named("draft").unwrap();
    let opts = RenderOptions::default();

    assert!(render_final(
        &p,
        &edl,
        &fence,
        Path::new("failed_final.mp4"),
        &preset,
        opts,
        None
    )
    .is_err());

    assert!(cut_media::render::render_range(
        &p,
        &edl,
        &fence,
        Path::new("failed_range.mp4"),
        &preset,
        [0, 500],
        opts,
        None
    )
    .is_err());

    let failed_gif = export_dir.join("failed.gif");
    assert!(make_gif(&dummy_gif_src, &failed_gif, 10, 80, "floyd").is_err());

    let remaining = dir_entries(&export_dir);
    assert!(
        remaining.is_empty(),
        "failed export left partial or temp files in export dir: {remaining:?}"
    );
    let leftover = dir_entries(&cut_data_staging_dir());
    assert!(
        leftover.is_empty(),
        "failed export left files in staging dir: {leftover:?}"
    );
}

#[cfg(unix)]
#[test]
fn target_symlink_is_never_followed_on_publish() {
    let _lock = FfmpegWrapperGuard::acquire_lock();
    let root = tempfile::tempdir().unwrap();
    create_dir_with_mode(root.path(), 0o700);
    let src = root.path().join("src.mp4");
    make_test_clip(&src);

    let export_dir = root.path().join("exports");
    create_dir_with_mode(&export_dir, 0o755);

    let sentinel = b"outside-sentinel-do-not-overwrite\n";
    let victim_atomic = root.path().join("victim-atomic.mp4");
    std::fs::write(&victim_atomic, sentinel).unwrap();
    let target_atomic = export_dir.join("linked_atomic.mp4");
    std::os::unix::fs::symlink(&victim_atomic, &target_atomic).unwrap();

    let victim_gif = root.path().join("victim-gif.gif");
    std::fs::write(&victim_gif, sentinel).unwrap();
    let target_gif = export_dir.join("linked.gif");
    std::os::unix::fs::symlink(&victim_gif, &target_gif).unwrap();

    let raw_args = vec![
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        "color=size=64x64:duration=0.2".into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "ultrafast".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        target_atomic.display().to_string(),
    ];
    let _ = run_ffmpeg_atomic_output(&raw_args, &target_atomic);
    assert_eq!(
        std::fs::read(&victim_atomic).unwrap(),
        sentinel,
        "atomic publish must never follow a symlink at target name"
    );

    let _ = make_gif(&src, &target_gif, 10, 80, "floyd");
    assert_eq!(
        std::fs::read(&victim_gif).unwrap(),
        sentinel,
        "make_gif publish must never follow a symlink at target name"
    );
}

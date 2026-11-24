#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

cut_core::isolate_test_process!();

fn write_concurrent_limits_ffmpeg(dir: &Path) -> std::path::PathBuf {
    let ffmpeg = dir.join("ffmpeg");
    let seen_h264 = dir.join("seen_h264");
    let seen_hevc = dir.join("seen_hevc");
    let script = format!(
        r#"#!/bin/sh
set -eu
args="$*"
case " $args " in
  *" -version "*)
    echo "ffmpeg version 8.0.1-fixture"
    exit 0
    ;;
  *" -filters "*)
    echo " T.. subtitles ..."
    exit 0
    ;;
  *" -encoders "*)
    printf "Encoders:\n V..... h264_nvenc\n V..... hevc_nvenc\n"
    exit 0
    ;;
esac
enc=""
size=""
prev=""
for a in "$@"; do
  if [ "$prev" = "-c:v" ]; then enc="$a"; fi
  case "$a" in
    *size=*)
      size="${{a#*size=}}"
      size="${{size%%:*}}"
      ;;
  esac
  prev="$a"
done
case "$enc:$size" in
  h264_nvenc:256x256|hevc_nvenc:256x256)
    exit 0
    ;;
  h264_nvenc:3840x2160)
    touch "{seen_h264}"
    i=0
    while [ ! -f "{seen_hevc}" ] && [ "$i" -lt 40 ]; do
      sleep 0.05
      i=$((i + 1))
    done
    [ -f "{seen_hevc}" ]
    exit $?
    ;;
  hevc_nvenc:3840x2160)
    touch "{seen_hevc}"
    i=0
    while [ ! -f "{seen_h264}" ] && [ "$i" -lt 40 ]; do
      sleep 0.05
      i=$((i + 1))
    done
    [ -f "{seen_h264}" ]
    exit $?
    ;;
  h264_nvenc:4096x2160|hevc_nvenc:4096x2160)
    exit 0
    ;;
  h264_nvenc:7680x4320|hevc_nvenc:7680x4320)
    echo "Cannot allocate" >&2
    exit 1
    ;;
  *)
    exit 1
    ;;
esac
"#,
        seen_h264 = seen_h264.display(),
        seen_hevc = seen_hevc.display(),
    );
    fs::write(&ffmpeg, script).unwrap();
    fs::set_permissions(&ffmpeg, fs::Permissions::from_mode(0o755)).unwrap();
    ffmpeg
}

#[test]
fn size_limits_probes_working_hardware_encoders_concurrently() {
    const CHILD: &str = "SHELLX_CUT_HWENCODE_LIMITS_PARALLEL_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "size_limits_probes_working_hardware_encoders_concurrently",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env_remove("SHELLX_CUT_NO_HWENC")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child test failed:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let ffmpeg = write_concurrent_limits_ffmpeg(tmp.path());
    std::env::set_var("SHELLX_CUT_FFMPEG", &ffmpeg);
    std::env::set_var("SHELLX_CUT_FFMPEG_AUTO", "0");
    std::env::remove_var("SHELLX_CUT_NO_HWENC");
    cut_media::toolpath::refresh_media_tools();

    let limits = cut_media::hwencode_limits::size_limits();
    let h264 = limits
        .iter()
        .find(|l| l.encoder == "h264_nvenc")
        .expect("h264_nvenc present");
    let hevc = limits
        .iter()
        .find(|l| l.encoder == "hevc_nvenc")
        .expect("hevc_nvenc present");

    assert_eq!(
        h264.largest_accepted.as_deref(),
        Some("4096x2160"),
        "h264_nvenc and hevc_nvenc 3840x2160 probes must run concurrently: {limits:?}"
    );
    assert_eq!(h264.smallest_refused.as_deref(), Some("7680x4320"));
    assert_eq!(
        hevc.largest_accepted.as_deref(),
        Some("4096x2160"),
        "hevc_nvenc and h264_nvenc 3840x2160 probes must run concurrently: {limits:?}"
    );
    assert_eq!(hevc.smallest_refused.as_deref(), Some("7680x4320"));
}

fn write_counting_limits_ffmpeg(dir: &Path, log: &Path) -> std::path::PathBuf {
    let ffmpeg = dir.join("ffmpeg_counting");
    let script = format!(
        r#"#!/bin/sh
for a in "$@"; do
  case "$a" in
    *size=*) echo "$*" >> "{log}" ;;
  esac
done
for a in "$@"; do
  case "$a" in
    *size=7680x4320*|*size=8192x4320*) exit 1 ;;
  esac
done
exit 0
"#,
        log = log.display(),
    );
    fs::write(&ffmpeg, script).unwrap();
    fs::set_permissions(&ffmpeg, fs::Permissions::from_mode(0o755)).unwrap();
    ffmpeg
}

fn probe_count(log: &Path) -> usize {
    fs::read_to_string(log)
        .map(|text| text.lines().count())
        .unwrap_or(0)
}

#[test]
fn doctor_size_limits_repeat_scan_reuses_answers_and_cancelled_probes_are_not_kept() {
    const CHILD: &str = "SHELLX_CUT_HWENCODE_LIMITS_CACHED_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "doctor_size_limits_repeat_scan_reuses_answers_and_cancelled_probes_are_not_kept",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child test failed:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("probes.log");
    let ffmpeg = write_counting_limits_ffmpeg(tmp.path(), &log);
    let caps = cut_media::hwencode::HwCaps {
        h264: Some("h264_nvenc".into()),
        hevc: None,
        av1: None,
    };

    // A flag raised before the probes start: nothing may be learned from it.
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let _ = cut_media::hwencode_limits::size_limits_for_cancelable(&ffmpeg, &caps, Some(cancelled));

    let first = cut_media::hwencode_limits::size_limits_for_cancelable(&ffmpeg, &caps, None);
    assert_eq!(
        first[0].largest_accepted.as_deref(),
        Some("4096x2160"),
        "a cancelled earlier probe must not be remembered as a refusal: {first:?}"
    );
    let after_first = probe_count(&log);
    assert!(
        after_first > 0,
        "the first complete scan runs the test encodes"
    );

    let second = cut_media::hwencode_limits::size_limits_for_cancelable(&ffmpeg, &caps, None);
    assert_eq!(second[0].largest_accepted, first[0].largest_accepted);
    assert_eq!(second[0].smallest_refused, first[0].smallest_refused);
    assert_eq!(
        probe_count(&log),
        after_first,
        "an explicit re-scan of the same ffmpeg in the same tools generation reuses the answers"
    );
}

//! Vidstab filter generation and pre-pass ownership for the renderer.
//!
//! The pre-pass runs through the active render process control, so cancellation
//! and deadlines are terminal rather than cached as a negative capability result.

use super::input_paths::strip_verbatim_prefix;
use crate::ffmpeg::escape_filter_path;
use cut_core::{CutError, Edl, Project, TrackKind};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub(super) fn filter(
    stabilize: Option<&cut_core::ClipStabilize>,
    frozen: bool,
    asset_hash: &str,
    src_in_ms: u64,
    src_out_ms: u64,
    project_dir: &Path,
) -> Result<String, CutError> {
    let Some(st) = stabilize else {
        return Ok(String::new());
    };
    if frozen {
        return Ok(String::new());
    }
    let trf = trf_path(project_dir, asset_hash, src_in_ms, src_out_ms);
    if !trf.exists() {
        return Ok(String::new());
    }
    let smoothing = st.smoothing.clamp(1.0, 100.0).round() as u64;
    let (_, transform_fileformat) = crate::ffmpeg::vidstab_fileformat_support()?;
    Ok(transform_filter(&trf, smoothing, transform_fileformat))
}

pub(super) fn detect_filter(
    src_in_ms: u64,
    src_out_ms: u64,
    trf: &Path,
    detect_fileformat: bool,
) -> String {
    let fileformat = if detect_fileformat {
        ":fileformat=ascii"
    } else {
        ""
    };
    format!(
        "trim=start={}:end={},setpts=PTS-STARTPTS,\
         vidstabdetect=shakiness=8:accuracy=15{}:result={}",
        secs(src_in_ms),
        secs(src_out_ms),
        fileformat,
        escape_filter_path(trf),
    )
}

pub(super) fn transform_filter(trf: &Path, smoothing: u64, transform_fileformat: bool) -> String {
    let fileformat = if transform_fileformat {
        ":fileformat=ascii"
    } else {
        ""
    };
    format!(
        ",vidstabtransform=input={}:smoothing={}:crop=black{}",
        escape_filter_path(trf),
        smoothing,
        fileformat,
    )
}

pub(super) fn prepare(project: &Project, edl: &Edl, project_dir: &Path) -> Result<(), CutError> {
    let mut done: HashSet<PathBuf> = HashSet::new();
    for seg in &edl.segments {
        if seg.stabilize.is_none() || seg.freeze.is_some() || seg.track_kind != TrackKind::Video {
            continue;
        }
        let (Some(asset_id), Some(src_in), Some(src_out)) =
            (&seg.asset, seg.src_in_ms, seg.src_out_ms)
        else {
            continue;
        };
        let Some(asset) = project.assets.get(asset_id) else {
            continue;
        };
        let trf = trf_path(project_dir, &asset.hash, src_in, src_out);
        if trf.exists() || !done.insert(trf.clone()) {
            continue;
        }
        if let Some(parent) = trf.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let (detect_fileformat, _) = crate::ffmpeg::vidstab_fileformat_support()?;
        let vf = detect_filter(src_in, src_out, &trf, detect_fileformat);
        let mut input_path = PathBuf::from(&asset.path);
        if input_path.is_relative() {
            input_path = project_dir.join(input_path);
        }
        let input_path = strip_verbatim_prefix(&input_path);
        crate::ffmpeg::run_ffmpeg(&[
            "-i".to_string(),
            input_path.display().to_string(),
            "-vf".to_string(),
            vf,
            "-f".to_string(),
            "null".to_string(),
            "-".to_string(),
        ])?;
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn stab_detect_filter_for_test(
    src_in_ms: u64,
    src_out_ms: u64,
    trf: &Path,
    detect_fileformat: bool,
) -> String {
    detect_filter(src_in_ms, src_out_ms, trf, detect_fileformat)
}

#[cfg(test)]
pub(super) fn stab_transform_filter_for_test(
    trf: &Path,
    smoothing: u64,
    transform_fileformat: bool,
) -> String {
    transform_filter(trf, smoothing, transform_fileformat)
}

fn trf_path(project_dir: &Path, asset_hash: &str, src_in_ms: u64, src_out_ms: u64) -> PathBuf {
    let safe: String = asset_hash
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    strip_verbatim_prefix(project_dir)
        .join("stab")
        .join(format!("{safe}_{src_in_ms}_{src_out_ms}.trf"))
}

fn secs(ms: u64) -> String {
    format!("{:.3}", ms as f64 / 1000.0)
}

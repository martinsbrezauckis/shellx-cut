//! Prepared-context MatAnyone2 execution.
//!
//! This module owns the premium launch and seed cache rules. It receives only a
//! fully admitted runtime/model group from `matte_premium_runtime`; it never
//! consults the ordinary installation, model settings, environment overrides,
//! or a hub cache.

use super::{io_err, matte_runner_command, parse_stats_line, MatteStats};
use crate::matte::premium_runtime::PreparedMatanyoneRuntime;
use cut_core::{error_codes, ClipMatte, CutError, MatteQuality, MatteSeed};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
#[cfg(test)]
#[path = "matte_prepared_output_tests.rs"]
mod output_tests;

pub(super) fn bake_matanyone(
    cache_dir: &Path,
    in_path: &Path,
    asset_hash: &str,
    alpha_out: &Path,
    matte: &ClipMatte,
    runtime: &PreparedMatanyoneRuntime,
) -> Result<MatteStats, CutError> {
    let seed = resolve_seed_mask(cache_dir, in_path, asset_hash, matte, runtime)?;
    let max_size = if matches!(matte.quality, MatteQuality::Fast) {
        "720"
    } else {
        "1080"
    };
    let mut command = matte_runner_command(&runtime.python, &runtime.matanyone_script, true);
    command
        .arg(in_path)
        .arg(alpha_out)
        .arg("--mask")
        .arg(&seed)
        .arg("--model")
        .arg(&runtime.matanyone_model)
        .arg("--max-size")
        .arg(max_size);
    let output =
        crate::dispatch::run_bounded_foreground_command(&mut command, "prepared MatAnyone runner")
            .map_err(|error| {
                CutError::new(
                    error_codes::IO,
                    format!("prepared MatAnyone runner spawn failed: {error}"),
                    "the declared native premium matte runtime could not be started",
                )
                .with_suggested_action(
                    "repair or reprepare the declared native runtime and model inputs",
                )
            })?;
    if !output.status.success() {
        return Err(CutError::new(
            error_codes::IO,
            format!(
                "prepared MatAnyone runner failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            "the declared native premium matte runtime could not bake the alpha",
        ));
    }
    parse_stats_line(
        &String::from_utf8_lossy(&output.stdout),
        "prepared MatAnyone runner",
    )
}

fn resolve_seed_mask(
    cache_dir: &Path,
    asset_path: &Path,
    asset_hash: &str,
    matte: &ClipMatte,
    runtime: &PreparedMatanyoneRuntime,
) -> Result<PathBuf, CutError> {
    match &matte.seed {
        Some(seed) => sam2_seed_mask(cache_dir, asset_path, asset_hash, seed, runtime),
        None => rvm_seed_mask(cache_dir, asset_path, asset_hash, runtime),
    }
}

pub(crate) fn prepared_seed_path(
    cache_dir: &Path,
    asset_hash: &str,
    seed_kind: &str,
    runtime: &PreparedMatanyoneRuntime,
) -> PathBuf {
    // Every cache-relevant value is length-delimited before hashing. This keeps
    // names well below common filesystem limits while retaining the full asset,
    // prompt, receipt, and sealed model-group identity. A seed from an ordinary
    // install or a prior prepared runtime cannot satisfy current native proof.
    let mut hasher = Sha256::new();
    for component in [
        "cut.matte.prepared-seed/v1",
        asset_hash,
        seed_kind,
        runtime.binding.contract.as_str(),
        runtime.binding.manifest_sha256.as_str(),
        runtime.binding.receipt_sha256.as_str(),
        runtime.binding.rvm_model_id.as_str(),
        runtime.binding.rvm_model_sha256.as_str(),
        runtime.binding.matanyone_model_id.as_str(),
        runtime.binding.matanyone_model_sha256.as_str(),
        runtime.binding.sam2_model_id.as_str(),
        runtime.binding.sam2_model_sha256.as_str(),
    ] {
        hasher.update((component.len() as u64).to_be_bytes());
        hasher.update(component.as_bytes());
    }
    let digest = format!("{:x}", hasher.finalize());
    let asset_prefix = asset_hash
        .get(..12)
        .filter(|prefix| prefix.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .unwrap_or("asset");
    cache_dir.join(format!(
        "{asset_prefix}-prepared-{}.seed.png",
        &digest[..24]
    ))
}

fn sam2_seed_mask(
    cache_dir: &Path,
    asset_path: &Path,
    asset_hash: &str,
    seed: &MatteSeed,
    runtime: &PreparedMatanyoneRuntime,
) -> Result<PathBuf, CutError> {
    cut_core::matte_cache::validate_asset_hash(asset_hash)?;
    let out = prepared_seed_path(
        cache_dir,
        asset_hash,
        &format!("sam2-{}", seed.short_hash()),
        runtime,
    );
    super::output::bake_seed(&out, |staged| {
        let mut command = sam2_seed_command(runtime, asset_path, staged, seed)?;
        let output = crate::dispatch::run_bounded_foreground_command(
            &mut command,
            "prepared SAM2 seed runner",
        )
        .map_err(|error| io_err("prepared SAM2 seed: spawn runner", error))?;
        if !output.status.success() {
            return Err(CutError::new(
                error_codes::IO,
                format!(
                    "prepared SAM2 seed generation failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
                "the declared native SAM2 runtime could not produce the subject mask",
            ));
        }
        Ok(())
    })
}

pub(crate) fn sam2_seed_command(
    runtime: &PreparedMatanyoneRuntime,
    asset_path: &Path,
    out: &Path,
    seed: &MatteSeed,
) -> Result<std::process::Command, CutError> {
    let mut command = matte_runner_command(&runtime.python, &runtime.sam2_script, true);
    command
        .arg(asset_path)
        .arg(out)
        .arg("--at-ms")
        .arg(seed.at_ms.to_string())
        .arg("--checkpoint")
        .arg(&runtime.sam2_model);
    match (seed.point, seed.bbox) {
        (Some(point), _) => {
            command
                .arg("--point")
                .arg(format!("{},{}", point[0], point[1]));
        }
        (None, Some(bounds)) => {
            command.arg("--box").arg(format!(
                "{},{},{},{}",
                bounds[0], bounds[1], bounds[2], bounds[3]
            ));
        }
        (None, None) => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "matte seed needs a point [x,y] or a box [x,y,w,h]",
                "pass seed.point or seed.bbox to pick the subject",
            ));
        }
    }
    Ok(command)
}

fn rvm_seed_mask(
    cache_dir: &Path,
    asset_path: &Path,
    asset_hash: &str,
    runtime: &PreparedMatanyoneRuntime,
) -> Result<PathBuf, CutError> {
    cut_core::matte_cache::validate_asset_hash(asset_hash)?;
    let seed = prepared_seed_path(cache_dir, asset_hash, "rvm", runtime);
    super::output::bake_seed(&seed, |staged| {
        let mut command = matte_runner_command(&runtime.python, &runtime.rvm_script, true);
        command
            .arg(asset_path)
            .arg("--first-frame-mask")
            .arg(staged)
            .arg("--model")
            .arg(&runtime.rvm_model);
        let output = crate::dispatch::run_bounded_foreground_command(
            &mut command,
            "prepared RVM seed runner",
        )
        .map_err(|error| io_err("prepared RVM seed: spawn runner", error))?;
        if !output.status.success() {
            return Err(CutError::new(
                error_codes::IO,
                format!(
                    "prepared RVM seed generation failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
                "the declared native RVM runtime could not produce the first-frame mask",
            ));
        }
        Ok(())
    })
}

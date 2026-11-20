//! Matte alpha/receipt transaction shared by all admitted bake transports.

use super::native_runtime::PreparedMatteBinding;
use super::{cache_matches_prepared_runtime, io_err, output, MatteStats};
use cut_core::{ClipMatte, CutError};
use std::path::Path;

fn cached(
    alpha: &Path,
    expected: Option<&PreparedMatteBinding>,
) -> Result<Option<MatteStats>, CutError> {
    let receipt = alpha.with_extension("json");
    let alpha_exists = output::cache_hit(alpha)?;
    let receipt_exists = output::cache_hit(&receipt)?;
    // Item 155: only an alpha this engine baked counts as a cache hit; one
    // that came with the project (planted, or baked on another computer, or a
    // legacy name from an older version) is baked again.
    if !alpha_exists || !receipt_exists || !cut_media::cache_trust::trusted(alpha) {
        return Ok(None);
    }
    // A cached receipt holds a few numbers; a larger or linked one is a miss
    // and is replaced by the bake.
    let cut_core::store::BoundedCacheRead::Bytes(body) =
        cut_core::store::read_bounded_cache(&receipt, 1024 * 1024)
    else {
        return Ok(None);
    };
    Ok(serde_json::from_slice::<MatteStats>(&body)
        .ok()
        .filter(|stats| cache_matches_prepared_runtime(stats, expected)))
}

pub(super) fn ensure(
    project: &Path,
    hash: &str,
    matte: &ClipMatte,
    expected: Option<PreparedMatteBinding>,
    bake: impl FnOnce(&Path, &Path) -> Result<MatteStats, CutError>,
) -> Result<MatteStats, CutError> {
    cut_core::matte_cache::validate_asset_hash(hash)?;
    let dir = cut_core::matte_cache::cache_dir(project, true)?;
    let alpha = dir.join(matte.cache_filename(hash));
    if let Some(mut stats) = cached(&alpha, expected.as_ref())? {
        stats.cached = true;
        return Ok(stats);
    }
    let alpha_stage = output::StagedOutput::new(&alpha)?;
    let receipt_stage = output::StagedOutput::new(&alpha.with_extension("json"))?;
    // A strictly reconstructed Unix legacy leaf can be migrated through the
    // same owned transaction without models/network. Never resolve ADS on Windows.
    if !output::cache_hit(&alpha)? {
        if let Some(legacy) = cut_core::matte_cache::legacy_alpha_path(&dir, hash, matte) {
            if let Some(mut stats) = cached(&legacy, expected.as_ref())? {
                std::fs::copy(&legacy, alpha_stage.path())?;
                receipt_stage.write(
                    &serde_json::to_vec(&stats).map_err(|e| io_err("serialize receipt", e))?,
                )?;
                output::publish_pair(alpha_stage, receipt_stage)?;
                cut_media::cache_trust::remember(&alpha);
                stats.cached = true;
                return Ok(stats);
            }
        }
    }
    let mut stats = bake(&dir, alpha_stage.path())?;
    stats.native_runtime = expected;
    stats.cached = false;
    receipt_stage
        .write(&serde_json::to_vec(&stats).map_err(|e| io_err("serialize receipt", e))?)?;
    output::publish_pair(alpha_stage, receipt_stage)?;
    cut_media::cache_trust::remember(&alpha);
    Ok(stats)
}

#[cfg(test)]
#[path = "matte_cache_bake_tests.rs"]
mod tests;

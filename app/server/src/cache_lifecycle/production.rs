//! A producer's source identity, captured before encoding and rechecked before publication.

use super::{cache_error, ownership, CacheKind, SourceCheck};
use crate::state::AppState;
use cut_core::CutError;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(crate) struct CacheSource {
    asset: String,
    path: String,
    pub(crate) source: PathBuf,
    hash: String,
}

impl CacheSource {
    /// The caller holds the cache lifecycle lease through generation and recording.
    pub(crate) async fn capture(
        state: &AppState,
        project_dir: &Path,
        asset: &str,
        expected: Option<(&Path, &str)>,
    ) -> Result<Self, CutError> {
        // Reject an invalid asset name before it can become an encoder output path.
        let _ = ownership::base_output_name(CacheKind::Thumbnails, asset)?;
        let identity = {
            let guard = state.project.read().await;
            let store = guard
                .as_ref()
                .filter(|store| store.dir == project_dir)
                .ok_or_else(source_changed)?;
            let current = store.project.assets.get(asset).ok_or_else(source_changed)?;
            let source = project_dir.join(&current.path);
            if expected.is_some_and(|(path, hash)| path != source || hash != current.hash) {
                return Err(source_changed());
            }
            Self {
                asset: asset.into(),
                path: current.path.clone(),
                source,
                hash: current.hash.clone(),
            }
        };
        identity.verify_source().await?;
        Ok(identity)
    }

    pub(crate) fn binding(&self, kind: CacheKind) -> String {
        output_binding(kind, &self.asset, &self.hash)
    }

    pub(crate) async fn proxy_input(&self, project_dir: &Path) -> Result<PathBuf, CutError> {
        let path = project_dir.join(format!("proxies/{}.mp4", self.asset));
        let binding = self.binding(CacheKind::Proxies);
        crate::dispatch::run_blocking("preview proxy identity", move || {
            if cut_core::matte_cache::plain_file_exists(&path).unwrap_or(false)
                && cut_media::cache_trust::trusted_bound(&path, &binding) {
                Ok(path)
            } else {
                Err(cache_error("the editing proxy needs rebuilding", "the existing proxy is not verified for this source; rebuild its editing cache first"))
            }
        }).await
    }

    async fn verify_source(&self) -> Result<(), CutError> {
        let source = self.source.clone();
        let hash = self.hash.clone();
        crate::dispatch::run_blocking("preview source identity", move || {
            if SourceCheck::verify(&source, &hash).matches {
                Ok(())
            } else {
                Err(source_changed())
            }
        })
        .await
    }

    /// An editable project ledger never establishes engine origin. Both the
    /// encoded bytes' external origin record and unchanged source must match.
    pub(crate) async fn record(
        &self,
        state: &AppState,
        project_dir: &Path,
        kind: CacheKind,
        output: &Path,
    ) -> Result<(), CutError> {
        if let Err(error) = self.verify_source().await {
            // A failed identity check must not leave a source-bound origin claim.
            cut_media::cache_trust::forget(output);
            return Err(error);
        }
        let guard = state.project.read().await;
        let current = guard
            .as_ref()
            .filter(|store| store.dir == project_dir)
            .and_then(|store| store.project.assets.get(&self.asset));
        if !current.is_some_and(|asset| asset.path == self.path && asset.hash == self.hash) {
            cut_media::cache_trust::forget(output);
            return Err(source_changed());
        }
        let (dir, asset, hash, output, binding) = (
            project_dir.to_path_buf(),
            self.asset.clone(),
            self.hash.clone(),
            output.to_path_buf(),
            self.binding(kind),
        );
        // Keep the read guard until the ledger write completes; removal/relink
        // also need the caller's lifecycle lease exclusively.
        crate::dispatch::run_blocking("preview ownership", move || {
            if output
                != ownership::cache_root(&dir, kind)?.join(output.file_name().unwrap_or_default())
            {
                return Err(cache_error(
                    "preview ownership could not be recorded",
                    "the output is outside its exact cache root",
                ));
            }
            if !cut_media::cache_trust::trusted_bound(&output, &binding) {
                return Err(cache_error(
                    "preview ownership could not be recorded",
                    "the output has no matching engine-origin record",
                ));
            }
            ownership::record_generated_with_hash(
                &dir,
                kind,
                &asset,
                output.file_name().unwrap_or_default(),
                Some(&hash),
            )
        })
        .await
    }
}

pub(super) fn output_binding(kind: CacheKind, asset: &str, hash: &str) -> String {
    match kind {
        CacheKind::Proxies => crate::dispatch::proxy_binding(asset, hash),
        CacheKind::Thumbnails => format!("filmstrip\n{asset}\n{hash}"),
    }
}

fn source_changed() -> CutError {
    cache_error(
        "preview source changed or is unavailable",
        "restore or relink the asset before rebuilding its previews",
    )
}

//! Give the WebView document a URL tied to the UI it is about to load.
//! The API origin remains the plain engine URL; Vite's hashed assets retain
//! their own cache behavior.

use sha2::{Digest, Sha256};
use std::path::Path;

fn versioned_url(
    engine_url: &str,
    index: Option<&[u8]>,
    external_nonce: Option<&str>,
) -> Result<tauri::Url, String> {
    let mut url: tauri::Url = engine_url
        .parse()
        .map_err(|e| format!("invalid engine UI URL: {e}"))?;
    if url.scheme() != "http" || url.path() != "/" || url.query().is_some() {
        return Err("engine UI URL must be an HTTP origin root".into());
    }
    if let Some(nonce) = external_nonce {
        // The adopted server owns its UI; our local index cannot identify it.
        url.query_pairs_mut().append_pair("cut-session", nonce);
    } else {
        let bytes = index
            .filter(|bytes| !bytes.is_empty())
            .ok_or("bundled UI index identity is missing")?;
        let identity = format!("{:x}", Sha256::digest(bytes));
        url.query_pairs_mut().append_pair("cut-ui", &identity);
    }
    Ok(url)
}

pub(super) fn navigation_url(
    engine_url: &str,
    bundled_index: &Path,
    external: bool,
) -> Result<tauri::Url, String> {
    // An adopted cutd may serve different UI bytes from our bundle. A fresh
    // per-launch URL forces a document request to that engine even when its
    // version or our bundle did not change. Missing bundle identity still gets
    // a fresh URL; the server's own UI presence check owns that decision.
    let external_nonce = if external {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes)
            .map_err(|e| format!("could not create external UI navigation nonce: {e}"))?;
        Some(format!("{:032x}", u128::from_be_bytes(bytes)))
    } else {
        None
    };
    let index = std::fs::read(bundled_index).ok();
    versioned_url(engine_url, index.as_deref(), external_nonce.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_index_changes_navigation_but_not_origin_or_asset_path() {
        let old = versioned_url("http://127.0.0.1:6161/", Some(b"old index"), None).unwrap();
        let new = versioned_url("http://127.0.0.1:6161/", Some(b"new index"), None).unwrap();
        assert_ne!(old, new);
        assert_eq!(old.origin(), new.origin());
        assert_eq!(new.path(), "/");
        assert_eq!(new.query_pairs().count(), 1);
        assert_eq!(new.join("/assets/app-123.js").unwrap().query(), None);
    }

    #[test]
    fn missing_identity_fails_closed_for_spawned_and_stays_fresh_for_external() {
        assert!(versioned_url("http://127.0.0.1:6161/", None, None).is_err());
        let first = versioned_url("http://127.0.0.1:6161/", None, Some("one")).unwrap();
        let second = versioned_url("http://127.0.0.1:6161/", None, Some("two")).unwrap();
        assert_ne!(first, second);
        assert_eq!(first.path(), "/");
        assert_eq!(
            first,
            versioned_url(
                "http://127.0.0.1:6161/",
                Some(b"unrelated local UI"),
                Some("one")
            )
            .unwrap()
        );
    }
}

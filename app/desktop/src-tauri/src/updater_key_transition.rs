//! Updater trust transition for the v0.6.107 bridge release.
//!
//! The Tauri bundle configuration intentionally retains the previous public
//! key while v0.6.107 artifacts are produced: already-installed v0.6.106 apps
//! can only authenticate a bridge artifact signed by that key. Once v0.6.107
//! is running, this runtime override becomes the only key accepted for later
//! updates. The private halves of both keys remain outside the repository.

pub(crate) const RUNTIME_UPDATER_PUBLIC_KEY: &str =
    "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDYyQjIxNTI2RTZERDMzNzAKUldSd005M21KaFd5WXNUazZTSTFVSEhublZxem5vWTZMeDY3WVlSUFJKbThvSHZoZEtNcEV1NkMK";

pub(crate) fn plugin_builder() -> tauri_plugin_updater::Builder {
    tauri_plugin_updater::Builder::new().pubkey(RUNTIME_UPDATER_PUBLIC_KEY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    fn build_time_public_key() -> String {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("Tauri config JSON");
        config["plugins"]["updater"]["pubkey"]
            .as_str()
            .expect("build-time updater public key")
            .to_owned()
    }

    #[test]
    fn bridge_build_and_runtime_keys_are_distinct_public_keys() {
        assert_ne!(build_time_public_key(), RUNTIME_UPDATER_PUBLIC_KEY);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(RUNTIME_UPDATER_PUBLIC_KEY)
            .expect("runtime key is base64");
        let decoded = std::str::from_utf8(&decoded).expect("runtime key is UTF-8 minisign text");
        assert!(decoded.contains("minisign public key"));
        assert!(!decoded.contains("secret key"));
    }
}

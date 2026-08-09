//! Updater public-key binding for desktop release builds.
//!
//! Bundle-time artifact verification and runtime update verification use the
//! same public key. The private key remains outside the repository.

pub(crate) const UPDATER_PUBLIC_KEY: &str =
    "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDYyQjIxNTI2RTZERDMzNzAKUldSd005M21KaFd5WXNUazZTSTFVSEhublZxem5vWTZMeDY3WVlSUFJKbThvSHZoZEtNcEV1NkMK";

pub(crate) fn plugin_builder() -> tauri_plugin_updater::Builder {
    tauri_plugin_updater::Builder::new().pubkey(UPDATER_PUBLIC_KEY)
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
    fn build_and_runtime_keys_are_the_same_public_key() {
        assert_eq!(build_time_public_key(), UPDATER_PUBLIC_KEY);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(UPDATER_PUBLIC_KEY)
            .expect("runtime key is base64");
        let decoded = std::str::from_utf8(&decoded).expect("runtime key is UTF-8 minisign text");
        assert!(decoded.contains("minisign public key"));
        assert!(!decoded.contains("secret key"));
    }
}

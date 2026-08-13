//! Parsing for the text that `cargo tauri signer sign` writes for an updater
//! signature.
//!
//! Tauri package builds write the base64 payload alone to `.sig`, but signer
//! subcommands also print human status lines around a `Public signature:`
//! field. Both forms carry the same encoded Minisign record. Treat only the
//! explicit field as a signature; do not try to decode arbitrary log output.

use base64::Engine as _;
use minisign_verify::Signature;

const PUBLIC_SIGNATURE_HEADER: &str = "Public signature:";

/// Return the canonical one-line base64 updater signature from either a
/// regular Tauri `.sig` file or the labelled stdout emitted by
/// `cargo tauri signer sign`.
pub fn normalized_tauri_updater_signature(value: &str, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} is empty"));
    }

    if !trimmed.contains('\n') && !trimmed.contains('\r') {
        return validate_base64_signature(trimmed, label);
    }

    let mut signature = None;
    let lines = value.lines().collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate() {
        if line.trim() != PUBLIC_SIGNATURE_HEADER {
            continue;
        }
        if signature.is_some() {
            return Err(format!(
                "{label} has multiple {PUBLIC_SIGNATURE_HEADER} fields"
            ));
        }
        let encoded = lines
            .get(index + 1)
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .ok_or_else(|| format!("{label} has no value after {PUBLIC_SIGNATURE_HEADER}"))?;
        signature = Some(validate_base64_signature(encoded, label)?);
    }

    signature.ok_or_else(|| {
        format!(
            "{label} must be one base64 line or contain exactly one {PUBLIC_SIGNATURE_HEADER} field"
        )
    })
}

fn validate_base64_signature(value: &str, label: &str) -> Result<String, String> {
    if value.chars().any(char::is_whitespace) {
        return Err(format!("{label} base64 value contains whitespace"));
    }
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|error| format!("decode {label} base64 failed: {error}"))?;
    Ok(value.to_string())
}

/// Decode the outer Tauri base64 wrapper and parse the inner Minisign record.
/// Cryptographic verification remains the caller's responsibility.
pub fn parse_tauri_updater_signature(value: &str, label: &str) -> Result<Signature, String> {
    let encoded = normalized_tauri_updater_signature(value, label)?;
    let signature_text = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| format!("decode {label} base64 failed: {error}"))?;
    let signature_text = String::from_utf8(signature_text)
        .map_err(|error| format!("decode {label} UTF-8 failed: {error}"))?;
    Signature::decode(&signature_text).map_err(|error| format!("parse {label} failed: {error}"))
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use minisign_verify::PublicKey;

    use super::{normalized_tauri_updater_signature, parse_tauri_updater_signature};

    // This is the exact shape printed by cargo-tauri 2.11.2 for a signed
    // v0.6.109 Windows artifact identity. The public payload is deliberately
    // split from surrounding progress text before Minisign sees it.
    const ACTUAL_TAURI_SIGNER_OUTPUT: &str = r#"
Your file was signed successfully, You can find the signature here:
C:\build\ShellX Cut_0.6.109_x64-setup.exe.identity.sig

Public signature:
dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVSd005M21KaFd5WW1vaFVHSVFGb0cwajFLUlJ0NWNZc1dxc0ludThsbXU5NjR2QzBFcGhsellOVTVJVjZRYnZqTnl6RXNRMUtSSUNmSHc4b2lXZnd4WjJuRWRWN1Ayc3dFPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzg2NTM4MTE3CWZpbGU6U2hlbGxYIEN1dF8wLjYuMTA5X3g2NC1zZXR1cC5leGUuaWRlbnRpdHkKczJick8wSGZnUERnc05UekVuRWlKVmFKM1BSUWhnRnBFa1cvUnNRUnBHY2xwUEV0Wk9FL0ZzNnFlSWduTUlEeWRXRDVwYy9VM3BGTk1QYS9lYlNhQ2c9PQo=

Make sure to include this into the signature field of your update server.
"#;
    const SIGNED_IDENTITY: &str = "shellx-cut/updater-artifact-identity@1\nversion=0.6.109\nplatform=windows-x86_64\nsha256=7a88e19237256c8a801e33c00eddc4a790cee7b24396dfe4bef3b004b7b43ca1\n";

    #[test]
    fn parses_and_verifies_actual_multiline_tauri_signer_output() {
        let encoded = normalized_tauri_updater_signature(ACTUAL_TAURI_SIGNER_OUTPUT, "fixture")
            .expect("extract public signature from cargo-tauri output");
        assert_eq!(encoded.len(), 436);
        let signature = parse_tauri_updater_signature(ACTUAL_TAURI_SIGNER_OUTPUT, "fixture")
            .expect("parse Minisign signature after extracting public field");
        let public_key = base64::engine::general_purpose::STANDARD
            .decode(crate::updater_key_transition::UPDATER_PUBLIC_KEY)
            .expect("configured updater key is base64");
        let public_key = std::str::from_utf8(&public_key).expect("public key is UTF-8 Minisign");
        let public_key = PublicKey::decode(public_key).expect("configured updater key parses");
        public_key
            .verify(SIGNED_IDENTITY.as_bytes(), &signature, true)
            .expect("the extracted signature still binds the exact identity bytes");
    }

    #[test]
    fn rejects_ambiguous_or_unlabelled_multiline_output() {
        assert!(normalized_tauri_updater_signature(
            "Public signature:\naGVsbG8=\nPublic signature:\nd29ybGQ=",
            "fixture",
        )
        .is_err());
        assert!(normalized_tauri_updater_signature("not\na\nsignature", "fixture").is_err());
    }
}

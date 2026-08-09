//! Shell/engine version identity for coherent desktop installs.
//!
//! The Windows registry and shell executable can both report a new version
//! while a locked `cutd.exe` remains old. The shell therefore reads the live
//! engine's own `/api/agent` identity before navigating to its UI and refuses a
//! mismatch. Artifact hashes remain a packaging responsibility; this module
//! owns the runtime version invariant and its actionable repair message.

fn response_body(raw: &str) -> Result<&str, String> {
    let status = raw
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| "engine identity response had no HTTP status".to_string())?;
    if status != 200 {
        return Err(format!("engine identity endpoint returned HTTP {status}"));
    }
    raw.split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .ok_or_else(|| "engine identity response had no body".to_string())
}

pub(crate) fn engine_version_from_http(raw: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(response_body(raw)?)
        .map_err(|e| format!("engine identity response was not valid JSON: {e}"))?;
    value
        .get("version")
        .and_then(serde_json::Value::as_str)
        .filter(|version| !version.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "engine identity response did not contain a version".to_string())
}

pub(crate) fn require_coherent_versions(
    shell_version: &str,
    engine_version: &str,
) -> Result<(), String> {
    if shell_version == engine_version {
        return Ok(());
    }
    Err(format!(
        "engine version mismatch: the desktop shell is v{shell_version}, but cutd is v{engine_version}. Close every ShellX Cut window, then reinstall ShellX Cut v{shell_version} from the official release before editing"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_identity_rejects_the_mixed_install_seen_in_the_field() {
        let raw = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n{\"product\":\"ShellX Cut\",\"version\":\"0.6.105\"}";
        let engine = engine_version_from_http(raw).expect("version");
        assert_eq!(engine, "0.6.105");
        let error = require_coherent_versions("0.6.106", &engine).unwrap_err();
        assert!(error.contains("desktop shell is v0.6.106"));
        assert!(error.contains("cutd is v0.6.105"));
        assert!(require_coherent_versions("0.6.106", "0.6.106").is_ok());
    }

    #[test]
    fn runtime_identity_fails_closed_on_missing_or_failed_identity() {
        assert!(engine_version_from_http("HTTP/1.1 404 Not Found\r\n\r\n{}").is_err());
        assert!(engine_version_from_http("HTTP/1.1 200 OK\r\n\r\n{}").is_err());
    }
}
